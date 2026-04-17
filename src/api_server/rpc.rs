use std::{
    collections::HashMap,
    str::from_utf8,
    time::{Duration, Instant},
};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Extension, WebSocketUpgrade,
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use rabbit_digger::{RabbitDigger, ServerEvent, Uuid};
use rd_interface::IntoAddress;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot},
    time::interval,
};
use tokio_stream::wrappers::IntervalStream;

use super::shared::{ApiError, ConnectionQuery, Ctx, DelayResponse, MaybePatch};
use crate::{
    config::{ImportSource, SelectMap},
    storage::Storage,
};

#[derive(Debug, Deserialize)]
struct RpcRequest {
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: JsonValue,
    id: Option<JsonValue>,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<JsonValue>,
}

#[derive(Debug, Serialize)]
struct RpcSubscriptionEnvelope<'a> {
    jsonrpc: &'static str,
    method: &'static str,
    params: RpcSubscriptionParams<'a>,
}

#[derive(Debug, Serialize)]
struct RpcSubscriptionParams<'a> {
    subscription: &'a str,
    topic: &'a str,
    payload: JsonValue,
}

#[derive(Debug, Deserialize)]
struct SubscribeParams {
    topic: String,
    #[serde(default)]
    params: JsonValue,
}

#[derive(Debug, Deserialize)]
struct UnsubscribeParams {
    subscription: String,
}

#[derive(Debug, Deserialize)]
struct CloseConnectionParams {
    uuid: Uuid,
}

#[derive(Debug, Deserialize)]
struct SelectNetParams {
    net_name: String,
    selected: String,
}

#[derive(Debug, Deserialize)]
struct DelayParams {
    net_name: String,
    url: url::Url,
    timeout: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct UserdataPathParams {
    path: String,
}

#[derive(Debug, Deserialize)]
struct PutUserdataParams {
    path: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct TailLogsParams {
    #[serde(default = "default_tail")]
    tail: usize,
}

fn default_tail() -> usize {
    500
}

pub(super) async fn ws_rpc(
    ws: WebSocketUpgrade,
    Extension(ctx): Extension<Ctx>,
) -> Result<Response, ApiError> {
    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(error) = handle_socket(socket, ctx).await {
            tracing::error!("rpc websocket exited: {error:?}");
        }
    }))
}

async fn handle_socket(socket: WebSocket, ctx: Ctx) -> anyhow::Result<()> {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            ws_tx.send(message).await?;
        }
        anyhow::Ok(())
    });

    let mut subscriptions: HashMap<String, oneshot::Sender<()>> = HashMap::new();

    while let Some(message) = ws_rx.next().await {
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                if !is_benign_websocket_disconnect(&error.to_string()) {
                    tracing::warn!("rpc websocket read error: {error}");
                }
                break;
            }
        };

        match message {
            Message::Text(text) => {
                let parsed: Result<RpcRequest, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(request) => {
                        if request.jsonrpc != "2.0" {
                            send_response(
                                &tx,
                                RpcResponse {
                                    jsonrpc: "2.0",
                                    id: request.id,
                                    result: None,
                                    error: Some(RpcError {
                                        code: -32600,
                                        message: "Invalid Request".into(),
                                        data: Some(json!("jsonrpc must be 2.0")),
                                    }),
                                },
                            )?;
                            continue;
                        }

                        if request.method == "rpc.subscribe" {
                            match handle_subscribe(request, &ctx, &tx, &mut subscriptions).await {
                                Ok(response) => send_response(&tx, response)?,
                                Err(response) => send_response(&tx, response)?,
                            }
                            continue;
                        }

                        if request.method == "rpc.unsubscribe" {
                            match handle_unsubscribe(request, &mut subscriptions) {
                                Ok(response) => send_response(&tx, response)?,
                                Err(response) => send_response(&tx, response)?,
                            }
                            continue;
                        }

                        if request.id.is_none() {
                            continue;
                        }

                        let response = handle_request(request, &ctx).await;
                        send_response(&tx, response)?;
                    }
                    Err(error) => {
                        send_response(
                            &tx,
                            RpcResponse {
                                jsonrpc: "2.0",
                                id: None,
                                result: None,
                                error: Some(RpcError {
                                    code: -32700,
                                    message: "Parse error".into(),
                                    data: Some(json!(error.to_string())),
                                }),
                            },
                        )?;
                    }
                }
            }
            Message::Close(_) => break,
            Message::Ping(payload) => {
                tx.send(Message::Pong(payload))?;
            }
            Message::Binary(_) | Message::Pong(_) => {}
        }
    }

    for (_, cancel) in subscriptions.drain() {
        let _ = cancel.send(());
    }
    drop(tx);
    writer.await??;
    Ok(())
}

async fn handle_subscribe(
    request: RpcRequest,
    ctx: &Ctx,
    tx: &mpsc::UnboundedSender<Message>,
    subscriptions: &mut HashMap<String, oneshot::Sender<()>>,
) -> Result<RpcResponse, RpcResponse> {
    let id = request.id.clone();
    let parsed: SubscribeParams = serde_json::from_value(request.params)
        .map_err(|error| invalid_params(id.clone(), error))?;
    let subscription = Uuid::new_v4().to_string();
    let (cancel_tx, cancel_rx) = oneshot::channel();

    let spawn_result = match parsed.topic.as_str() {
        "engine.events" => {
            tokio::spawn(engine_events_subscription(
                subscription.clone(),
                parsed.topic.clone(),
                ctx.rd.clone(),
                tx.clone(),
                cancel_rx,
            ));
            Ok(())
        }
        "connections" => {
            let query: ConnectionQuery = serde_json::from_value(parsed.params)
                .map_err(|error| invalid_params(id.clone(), error))?;
            tokio::spawn(connections_subscription(
                subscription.clone(),
                parsed.topic.clone(),
                ctx.rd.clone(),
                tx.clone(),
                query,
                cancel_rx,
            ));
            Ok(())
        }
        "logs" => {
            tokio::spawn(logs_subscription(
                subscription.clone(),
                parsed.topic.clone(),
                tx.clone(),
                cancel_rx,
            ));
            Ok(())
        }
        _ => Err(method_not_found(
            id.clone(),
            format!("Unknown subscription topic {}", parsed.topic),
        )),
    };

    match spawn_result {
        Ok(()) => {
            subscriptions.insert(subscription.clone(), cancel_tx);
            Ok(RpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({ "subscription": subscription })),
                error: None,
            })
        }
        Err(error) => Err(error),
    }
}

fn handle_unsubscribe(
    request: RpcRequest,
    subscriptions: &mut HashMap<String, oneshot::Sender<()>>,
) -> Result<RpcResponse, RpcResponse> {
    let id = request.id.clone();
    let parsed: UnsubscribeParams = serde_json::from_value(request.params)
        .map_err(|error| invalid_params(id.clone(), error))?;
    let removed = subscriptions.remove(&parsed.subscription);
    let unsubscribed = removed.is_some();
    if let Some(cancel) = removed {
        let _ = cancel.send(());
    }
    Ok(RpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(json!({ "unsubscribed": unsubscribed })),
        error: None,
    })
}

async fn handle_request(request: RpcRequest, ctx: &Ctx) -> RpcResponse {
    let id = request.id.clone();
    let result = match request.method.as_str() {
        "config.get" => get_config_value(ctx).await,
        "config.apply" => {
            let source: ImportSource = match serde_json::from_value(request.params) {
                Ok(params) => params,
                Err(error) => return invalid_params(id, error),
            };
            apply_config(ctx, source).await.map(|_| JsonValue::Null)
        }
        "registry.get" => get_registry_value(ctx).await,
        "connection.list" => get_connections_value(ctx).await,
        "connection.close" => {
            let params: CloseConnectionParams = match serde_json::from_value(request.params) {
                Ok(params) => params,
                Err(error) => return invalid_params(id, error),
            };
            close_connection(ctx, params.uuid).await.map(|ok| json!(ok))
        }
        "connection.closeAll" => close_all_connections(ctx).await.map(|count| json!(count)),
        "net.select" => {
            let params: SelectNetParams = match serde_json::from_value(request.params) {
                Ok(params) => params,
                Err(error) => return invalid_params(id, error),
            };
            select_net(ctx, params).await.map(|_| JsonValue::Null)
        }
        "net.delay" => {
            let params: DelayParams = match serde_json::from_value(request.params) {
                Ok(params) => params,
                Err(error) => return invalid_params(id, error),
            };
            net_delay(ctx, params).await
        }
        "userdata.get" => {
            let params: UserdataPathParams = match serde_json::from_value(request.params) {
                Ok(params) => params,
                Err(error) => return invalid_params(id, error),
            };
            userdata_get(ctx, &params.path).await
        }
        "userdata.put" => {
            let params: PutUserdataParams = match serde_json::from_value(request.params) {
                Ok(params) => params,
                Err(error) => return invalid_params(id, error),
            };
            userdata_put(ctx, &params.path, &params.value)
                .await
                .map(|copied| json!({ "copied": copied }))
        }
        "userdata.delete" => {
            let params: UserdataPathParams = match serde_json::from_value(request.params) {
                Ok(params) => params,
                Err(error) => return invalid_params(id, error),
            };
            userdata_delete(ctx, &params.path)
                .await
                .map(|_| json!({ "ok": true }))
        }
        "userdata.list" => userdata_list(ctx).await,
        "engine.stop" => engine_stop(ctx).await.map(|_| json!({ "ok": true })),
        "logs.tail" => {
            let params: TailLogsParams =
                serde_json::from_value(request.params).unwrap_or(TailLogsParams {
                    tail: default_tail(),
                });
            logs_tail(ctx, params.tail).await
        }
        "tun.suggestIp" => Ok(json!({ "ip": crate::util::suggest_tun_ip() })),
        _ => return method_not_found(id, format!("Unknown method {}", request.method)),
    };

    match result {
        Ok(result) => RpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        },
        Err(error) => RpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(map_api_error(error)),
        },
    }
}

async fn get_config_value(ctx: &Ctx) -> Result<JsonValue, ApiError> {
    match ctx.rd.get_config(|c| c.to_owned()).await {
        Ok(config_str) => serde_json::from_str(&config_str).map_err(ApiError::other),
        Err(_) => Ok(JsonValue::Null),
    }
}

async fn apply_config(ctx: &Ctx, source: ImportSource) -> Result<(), ApiError> {
    const LAST_SOURCE_KEY: &str = "daemon/last_source";
    if let Some(sender) = &ctx.source_sender {
        if let Ok(json) = serde_json::to_string(&source) {
            let _ = ctx.userdata.set(LAST_SOURCE_KEY, &json).await;
        }
        sender
            .send(source)
            .await
            .map_err(|_| ApiError::Anyhow(anyhow::anyhow!("Engine loop is not running")))?;
    } else {
        let stream = ctx.cfg_mgr.config_stream(source).await?;
        ctx.rd.stop().await?;
        tokio::spawn(ctx.rd.clone().start_stream(stream));
    }
    Ok(())
}

async fn get_registry_value(ctx: &Ctx) -> Result<JsonValue, ApiError> {
    ctx.rd
        .registry(|r| serde_json::to_value(r).map_err(ApiError::other))
        .await
}

async fn get_connections_value(ctx: &Ctx) -> Result<JsonValue, ApiError> {
    ctx.rd
        .connection(|c| serde_json::to_value(c).map_err(ApiError::other))
        .await
}

async fn close_connection(ctx: &Ctx, uuid: Uuid) -> Result<bool, ApiError> {
    Ok(ctx.rd.stop_connection(uuid).await?)
}

async fn close_all_connections(ctx: &Ctx) -> Result<usize, ApiError> {
    Ok(ctx.rd.stop_connections().await?)
}

async fn select_net(ctx: &Ctx, params: SelectNetParams) -> Result<(), ApiError> {
    ctx.rd
        .update_net(&params.net_name, |option| {
            if option.net_type == "select" {
                if let Some(object) = option.opt.as_object_mut() {
                    object.insert("selected".to_string(), params.selected.clone().into());
                }
            } else {
                tracing::warn!("net_type is not select");
            }
        })
        .await?;

    if let Some(id) = ctx.rd.get_id().await {
        let mut select_map = SelectMap::from_cache(&id, ctx.cfg_mgr.select_storage()).await?;
        select_map.insert(params.net_name, params.selected);
        select_map
            .write_cache(&id, ctx.cfg_mgr.select_storage())
            .await?;
    }

    Ok(())
}

async fn net_delay(ctx: &Ctx, params: DelayParams) -> Result<JsonValue, ApiError> {
    let net = ctx.rd.get_net(&params.net_name).await?.map(|n| n.as_net());
    let host = params.url.host_str();
    let port = params.url.port_or_known_default();
    let timeout = params.timeout.unwrap_or(5000);

    match (net, host, port) {
        (Some(net), Some(host), Some(port)) => {
            let start = Instant::now();
            let fut = async {
                let mut socket = net
                    .tcp_connect(
                        &mut rd_interface::Context::new(),
                        &(host, port).into_address()?,
                    )
                    .await?;
                let connect = start.elapsed().as_millis() as u64;

                let host_header = match params.url.port_or_known_default() {
                    Some(p) => format!("{}:{}", host, p),
                    None => host.to_string(),
                };
                let mut path_and_query = params.url.path().to_string();
                if path_and_query.is_empty() {
                    path_and_query = "/".to_string();
                }
                if let Some(query) = params.url.query() {
                    path_and_query.push('?');
                    path_and_query.push_str(query);
                }
                let request = format!(
                    "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n",
                    path = path_and_query,
                    host = host_header
                );
                socket.write_all(request.as_bytes()).await?;
                socket.flush().await?;

                let mut one = [0u8; 1];
                socket.read_exact(&mut one).await?;

                let response = start.elapsed().as_millis() as u64;
                anyhow::Result::<DelayResponse>::Ok(DelayResponse { connect, response })
            };
            let response = tokio::time::timeout(Duration::from_millis(timeout), fut).await;
            let response = match response {
                Ok(value) => Some(value?),
                Err(_) => None,
            };
            serde_json::to_value(response).map_err(ApiError::other)
        }
        _ => Ok(JsonValue::Null),
    }
}

async fn userdata_get(ctx: &Ctx, path: &str) -> Result<JsonValue, ApiError> {
    let item = ctx.userdata.get(path).await?.ok_or(ApiError::NotFound)?;
    serde_json::to_value(item).map_err(ApiError::other)
}

async fn userdata_put(ctx: &Ctx, path: &str, value: &str) -> Result<usize, ApiError> {
    ctx.userdata
        .set(path, from_utf8(value.as_bytes()).map_err(ApiError::other)?)
        .await?;
    Ok(value.len())
}

async fn userdata_delete(ctx: &Ctx, path: &str) -> Result<(), ApiError> {
    ctx.userdata.remove(path).await?;
    Ok(())
}

async fn userdata_list(ctx: &Ctx) -> Result<JsonValue, ApiError> {
    let keys = ctx.userdata.keys().await?;
    Ok(json!({ "keys": keys }))
}

async fn engine_stop(ctx: &Ctx) -> Result<(), ApiError> {
    ctx.rd.stop().await?;
    Ok(())
}

async fn logs_tail(ctx: &Ctx, tail: usize) -> Result<JsonValue, ApiError> {
    let path = ctx
        .log_file_path
        .clone()
        .ok_or_else(|| ApiError::Anyhow(anyhow::anyhow!("No log file configured")))?;
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|error| ApiError::Anyhow(error.into()))?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(tail);
    let entries: Vec<JsonValue> = lines[start..]
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    Ok(JsonValue::Array(entries))
}

async fn engine_events_subscription(
    subscription: String,
    topic: String,
    rd: RabbitDigger,
    tx: mpsc::UnboundedSender<Message>,
    mut cancel_rx: oneshot::Receiver<()>,
) {
    if send_subscription(
        &tx,
        &subscription,
        &topic,
        json!(ServerEvent::StatusChanged {
            status: rd.status()
        }),
    )
    .is_err()
    {
        return;
    }

    let mut rx = rd.subscribe_events();
    loop {
        tokio::select! {
            _ = &mut cancel_rx => break,
            received = rx.recv() => {
                match received {
                    Ok(event) => {
                        if send_subscription(&tx, &subscription, &topic, json!(event)).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("engine event subscriber lagged by {n}");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn connections_subscription(
    subscription: String,
    topic: String,
    rd: RabbitDigger,
    tx: mpsc::UnboundedSender<Message>,
    query: ConnectionQuery,
    mut cancel_rx: oneshot::Receiver<()>,
) {
    let stream = IntervalStream::new(interval(Duration::from_secs(1)));
    let mut stream = Box::pin(
        stream
            .then(move |_| {
                let rd = rd.clone();
                async move { rd.connection(|c| serde_json::to_value(c)).await }
            })
            .map(move |result| {
                let mut value = match result {
                    Ok(value) => value,
                    Err(error) => return Err(error),
                };
                if let (Some(object), true) = (value.as_object_mut(), query.without_connections) {
                    object.remove("connections");
                }
                Ok(value)
            }),
    );
    let mut last: Option<JsonValue> = None;

    loop {
        tokio::select! {
            _ = &mut cancel_rx => break,
            next = stream.next() => {
                let Some(result) = next else { break; };
                let value = match result {
                    Ok(value) => value,
                    Err(error) => {
                        tracing::warn!("connections subscription error: {error}");
                        break;
                    }
                };
                let payload = if query.patch {
                    if let Some(previous) = &last {
                        serde_json::to_value(MaybePatch::Patch(json_patch::diff(previous, &value))).unwrap_or(JsonValue::Null)
                    } else {
                        serde_json::to_value(MaybePatch::Full(value.clone())).unwrap_or(JsonValue::Null)
                    }
                } else {
                    serde_json::to_value(MaybePatch::Full(value.clone())).unwrap_or(JsonValue::Null)
                };
                last = Some(value);
                if send_subscription(&tx, &subscription, &topic, payload).is_err() {
                    break;
                }
            }
        }
    }
}

async fn logs_subscription(
    subscription: String,
    topic: String,
    tx: mpsc::UnboundedSender<Message>,
    mut cancel_rx: oneshot::Receiver<()>,
) {
    let mut rx = crate::log::get_sender().subscribe();
    loop {
        tokio::select! {
            _ = &mut cancel_rx => break,
            received = rx.recv() => {
                match received {
                    Ok(content) => {
                        let payload = JsonValue::String(String::from_utf8_lossy(&content).to_string());
                        if send_subscription(&tx, &subscription, &topic, payload).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("log subscriber lagged by {n}");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

fn send_response(tx: &mpsc::UnboundedSender<Message>, response: RpcResponse) -> anyhow::Result<()> {
    tx.send(Message::Text(serde_json::to_string(&response)?.into()))?;
    Ok(())
}

fn send_subscription(
    tx: &mpsc::UnboundedSender<Message>,
    subscription: &str,
    topic: &str,
    payload: JsonValue,
) -> anyhow::Result<()> {
    let envelope = RpcSubscriptionEnvelope {
        jsonrpc: "2.0",
        method: "rpc.subscription",
        params: RpcSubscriptionParams {
            subscription,
            topic,
            payload,
        },
    };
    tx.send(Message::Text(serde_json::to_string(&envelope)?.into()))?;
    Ok(())
}

fn invalid_params(id: Option<JsonValue>, error: impl std::fmt::Display) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcError {
            code: -32602,
            message: "Invalid params".into(),
            data: Some(json!(error.to_string())),
        }),
    }
}

fn method_not_found(id: Option<JsonValue>, message: String) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcError {
            code: -32601,
            message,
            data: None,
        }),
    }
}

fn map_api_error(error: ApiError) -> RpcError {
    match error {
        ApiError::NotFound => RpcError {
            code: 404,
            message: "Not found".into(),
            data: None,
        },
        ApiError::EngineNotRunning => RpcError {
            code: 503,
            message: "Engine not running".into(),
            data: None,
        },
        ApiError::Anyhow(error) => RpcError {
            code: -32000,
            message: error.to_string(),
            data: None,
        },
        ApiError::Other(error) => RpcError {
            code: -32000,
            message: error.to_string(),
            data: None,
        },
    }
}

fn is_benign_websocket_disconnect(message: &str) -> bool {
    message.contains("Broken pipe")
        || message.contains("Connection reset by peer")
        || message.contains("Connection closed")
}
