use std::{
    error::Error,
    future::ready,
    str::from_utf8,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket},
        Extension, Path, Query, WebSocketUpgrade,
    },
    http::HeaderValue,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    BoxError, Json,
};
use futures::{Stream, StreamExt, TryStreamExt};
use hyper::{header::HeaderName, HeaderMap, StatusCode};
use rabbit_digger::{RabbitDigger, Uuid};
use rd_interface::{IntoAddress, Value};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::{pin, time::interval};
use tokio_stream::wrappers::IntervalStream;

use crate::{
    config::{ConfigManager, ImportSource, SelectMap},
    storage::{FileStorage, Storage},
};

#[derive(Clone)]
pub(super) struct Ctx {
    pub(super) rd: RabbitDigger,
    pub(super) cfg_mgr: ConfigManager,
    pub(super) userdata: Arc<FileStorage>,
    pub(super) source_sender: Option<Arc<tokio::sync::mpsc::Sender<ImportSource>>>,
    pub(super) log_file_path: Option<std::path::PathBuf>,
}

pub(super) enum ApiError {
    NotFound,
    /// The engine is not running; the requested operation requires a running engine.
    EngineNotRunning,
    Anyhow(anyhow::Error),
    Other(BoxError),
}

impl ApiError {
    pub(super) fn other<E: std::error::Error + Send + Sync + 'static>(err: E) -> Self {
        ApiError::Other(Box::new(err))
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(inner: anyhow::Error) -> Self {
        // Map well-known engine errors to proper status codes
        if inner.to_string().contains("Not running") {
            return ApiError::EngineNotRunning;
        }
        ApiError::Anyhow(inner)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "Not found".to_string()),
            ApiError::EngineNotRunning => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Engine not running".to_string(),
            ),
            ApiError::Anyhow(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            ApiError::Other(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub(super) async fn get_config(
    Extension(Ctx { rd, .. }): Extension<Ctx>,
) -> Result<impl IntoResponse, ApiError> {
    match rd.get_config(|c| c.to_owned()).await {
        Ok(config_str) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                HeaderName::from_static("content-type"),
                HeaderValue::from_static("application/json"),
            );
            Ok((headers, config_str).into_response())
        }
        Err(_) => Ok(Json(Value::Null).into_response()),
    }
}

const LAST_SOURCE_KEY: &str = "daemon/last_source";

pub(super) async fn post_config(
    Extension(Ctx {
        rd,
        cfg_mgr,
        source_sender,
        userdata,
        ..
    }): Extension<Ctx>,
    Json(source): Json<ImportSource>,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(sender) = &source_sender {
        // Daemon mode: persist source for restore on restart
        if let Ok(json) = serde_json::to_string(&source) {
            let _ = userdata.set(LAST_SOURCE_KEY, &json).await;
        }
        // Send source through the channel to the main loop
        sender
            .send(source)
            .await
            .map_err(|_| ApiError::Anyhow(anyhow::anyhow!("Engine loop is not running")))?;
    } else {
        // Legacy mode: directly spawn start_stream
        let stream = cfg_mgr.config_stream(source).await?;
        rd.stop().await?;
        tokio::spawn(rd.start_stream(stream));
    }

    Ok(Json(Value::Null))
}

pub(super) async fn post_engine_stop(
    Extension(Ctx { rd, .. }): Extension<Ctx>,
) -> Result<impl IntoResponse, ApiError> {
    rd.stop().await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn get_registry(
    Extension(Ctx { rd, .. }): Extension<Ctx>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(rd.registry(|r| Json(&r).into_response()).await)
}

#[derive(Deserialize)]
pub struct ConnectionQuery {
    #[serde(default)]
    pub patch: bool,
    #[serde(default)]
    pub without_connections: bool,
}

pub(super) async fn get_connections(
    Extension(Ctx { rd, .. }): Extension<Ctx>,
) -> Result<Response, ApiError> {
    Ok(rd.connection(|c| Json(&c).into_response()).await)
}

pub(super) async fn delete_connections(
    Extension(Ctx { rd, .. }): Extension<Ctx>,
) -> Result<Response, ApiError> {
    Ok(Json(&rd.stop_connections().await?).into_response())
}

#[derive(Debug, Deserialize)]
pub struct PostSelect {
    selected: String,
}
pub(super) async fn post_select(
    Extension(Ctx { rd, cfg_mgr, .. }): Extension<Ctx>,
    Path(net_name): Path<String>,
    Json(PostSelect { selected }): Json<PostSelect>,
) -> Result<impl IntoResponse, ApiError> {
    rd.update_net(&net_name, |o| {
        if o.net_type == "select" {
            if let Some(o) = o.opt.as_object_mut() {
                o.insert("selected".to_string(), selected.clone().into());
            }
        } else {
            tracing::warn!("net_type is not select");
        }
    })
    .await?;

    if let Some(id) = rd.get_id().await {
        let mut select_map = SelectMap::from_cache(&id, cfg_mgr.select_storage()).await?;

        select_map.insert(net_name.to_string(), selected);

        select_map
            .write_cache(&id, cfg_mgr.select_storage())
            .await?;
    }

    Ok(Json(Value::Null))
}

pub(super) async fn delete_conn(
    Extension(Ctx { rd, .. }): Extension<Ctx>,
    Path(uuid): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let ok = rd.stop_connection(uuid).await?;
    Ok(Json(ok))
}

#[derive(Debug, Deserialize)]
pub struct DelayRequest {
    url: url::Url,
    timeout: Option<u64>,
}
#[derive(Debug, Serialize)]
pub struct DelayResponse {
    pub(super) connect: u64,
    pub(super) response: u64,
}
pub(super) async fn get_delay(
    Extension(Ctx { rd, .. }): Extension<Ctx>,
    Path(net_name): Path<String>,
    Query(DelayRequest { url, timeout }): Query<DelayRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let net = rd.get_net(&net_name).await?.map(|n| n.as_net());
    let host = url.host_str();
    let port = url.port_or_known_default();
    let timeout = timeout.unwrap_or(5000);
    Ok(match (net, host, port) {
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

                let host_header = match url.port_or_known_default() {
                    Some(p) => format!("{}:{}", host, p),
                    None => host.to_string(),
                };
                let mut path_and_query = url.path().to_string();
                if path_and_query.is_empty() {
                    path_and_query = "/".to_string();
                }
                if let Some(q) = url.query() {
                    path_and_query.push('?');
                    path_and_query.push_str(q);
                }
                let req = format!(
                    "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n",
                    path = path_and_query,
                    host = host_header
                );
                socket.write_all(req.as_bytes()).await?;
                socket.flush().await?;

                // Read 1 byte to ensure server responded.
                let mut one = [0u8; 1];
                socket.read_exact(&mut one).await?;

                let response = start.elapsed().as_millis() as u64;
                anyhow::Result::<DelayResponse>::Ok(DelayResponse { connect, response })
            };
            let resp = tokio::time::timeout(Duration::from_millis(timeout), fut).await;
            let resp = match resp {
                Ok(v) => Some(v?),
                _ => None,
            };
            Json(&resp).into_response()
        }
        _ => Json(&Value::Null).into_response(),
    })
}

pub(super) async fn get_userdata(
    Extension(Ctx { userdata, .. }): Extension<Ctx>,
    Path(tail): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let item = userdata
        .get(tail.as_str())
        .await?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(item))
}

pub(super) async fn put_userdata(
    Extension(Ctx { userdata, .. }): Extension<Ctx>,
    Path(tail): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    userdata
        .set(tail.as_str(), from_utf8(&body).map_err(ApiError::other)?)
        .await?;

    Ok(Json(json!({ "copied": body.len() })))
}

pub(super) async fn delete_userdata(
    Extension(Ctx { userdata, .. }): Extension<Ctx>,
    Path(tail): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    userdata.remove(tail.as_str()).await?;

    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn list_userdata(
    Extension(Ctx { userdata, .. }): Extension<Ctx>,
) -> Result<impl IntoResponse, ApiError> {
    let keys = userdata.keys().await?;
    Ok(Json(json!({ "keys": keys })))
}

async fn forward<E>(
    sub: impl Stream<Item = Result<Message, E>>,
    mut ws: WebSocket,
) -> anyhow::Result<()>
where
    E: Error + Send + Sync + 'static,
{
    pin!(sub);
    while let Some(item) = sub.try_next().await? {
        ws.send(item).await?;
    }
    Ok(())
}

pub(super) async fn get_connection(
    Query(query): Query<ConnectionQuery>,
    ws: WebSocketUpgrade,
    Extension(Ctx { rd, .. }): Extension<Ctx>,
) -> Result<Response, ApiError> {
    ws_conn(ws, rd, query).await
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MaybePatch {
    Full(Value),
    Patch(json_patch::Patch),
}

fn is_benign_websocket_disconnect(message: &str) -> bool {
    message.contains("Broken pipe")
        || message.contains("Connection reset by peer")
        || message.contains("Connection closed")
}

pub(super) async fn ws_conn(
    ws: WebSocketUpgrade,
    rd: RabbitDigger,
    query: ConnectionQuery,
) -> Result<Response, ApiError> {
    let ConnectionQuery {
        patch: patch_mode,
        without_connections,
    } = query;
    let stream = IntervalStream::new(interval(Duration::from_secs(1)));
    let stream = stream
        .then(move |_| {
            let rd = rd.clone();
            async move { rd.connection(|c| serde_json::to_value(c)).await }
        })
        .map_ok(move |mut v| {
            if let (Some(o), true) = (v.as_object_mut(), without_connections) {
                o.remove("connections");
            }
            v
        })
        .scan(Option::<Value>::None, move |last, r| {
            ready(Some(match (patch_mode, r) {
                (true, Ok(x)) => {
                    let r = if let Some(lv) = last {
                        MaybePatch::Patch(json_patch::diff(lv, &x))
                    } else {
                        MaybePatch::Full(x.clone())
                    };
                    *last = Some(x);
                    Ok(r)
                }
                (_, Ok(x)) => Ok(MaybePatch::Full(x)),
                (_, Err(e)) => Err(e),
            }))
        })
        .map_ok(|p| Message::Text(serde_json::to_string(&p).unwrap().into()));
    Ok(ws.on_upgrade(move |ws| async move {
        if let Err(e) = forward(stream, ws).await {
            let message = e.to_string();
            if !is_benign_websocket_disconnect(&message) {
                tracing::error!("WebSocket event error: {:?}", e)
            }
        }
    }))
}

pub(super) async fn ws_log(ws: WebSocketUpgrade) -> Result<impl IntoResponse, ApiError> {
    Ok(ws.on_upgrade(move |mut ws| async move {
        let mut recv = crate::log::get_sender().subscribe();
        while let Ok(content) = recv.recv().await {
            if let Err(e) = ws
                .send(Message::Text(
                    String::from_utf8_lossy(&content).to_string().into(),
                ))
                .await
            {
                let message = e.to_string();
                if !is_benign_websocket_disconnect(&message) {
                    tracing::error!("WebSocket event error: {:?}", e);
                }
                break;
            }
        }
    }))
}

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    #[serde(default = "default_tail")]
    pub tail: usize,
}
fn default_tail() -> usize {
    500
}

pub(super) async fn get_logs(
    Extension(Ctx { log_file_path, .. }): Extension<Ctx>,
    Query(LogsQuery { tail }): Query<LogsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let path =
        log_file_path.ok_or_else(|| ApiError::Anyhow(anyhow::anyhow!("No log file configured")))?;

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| ApiError::Anyhow(e.into()))?;

    // Take last N lines
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(tail);
    let tail_lines: Vec<&str> = lines[start..].to_vec();

    // Return as JSON array of raw JSON strings (each line is a JSON object)
    let entries: Vec<Value> = tail_lines
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    Ok(Json(entries))
}

pub(super) async fn get_suggest_tun_ip() -> Json<Value> {
    Json(json!({ "ip": crate::util::suggest_tun_ip() }))
}

pub(super) async fn sse_events(
    Extension(Ctx { rd, .. }): Extension<Ctx>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    // Subscribe before reading current status to avoid missing events
    let mut rx = rd.subscribe_events();
    let initial_status = rd.status();

    let stream = async_stream::stream! {
        // Send current status immediately on connect
        let init = rabbit_digger::ServerEvent::StatusChanged { status: initial_status };
        if let Ok(data) = serde_json::to_string(&init) {
            yield Ok(Event::default().data(data));
        }

        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(data) = serde_json::to_string(&event) {
                        yield Ok(Event::default().data(data));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("SSE client lagged, missed {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}
