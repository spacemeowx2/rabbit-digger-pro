use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Extension, WebSocketUpgrade,
    },
    response::Response,
};
use jsonrpc_core::{Error as JsonRpcError, IoHandler, Params, Value};
use rd_interface::IntoAddress;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    api_server::handlers::Ctx,
    config::{ImportSource, SelectMap},
    storage::Storage,
};

#[derive(Debug, Serialize)]
struct DelayResponse {
    connect: u64,
    response: u64,
}

async fn handle_ws(mut socket: WebSocket, ctx: Arc<Ctx>) {
    let mut io = IoHandler::new();
    let ctx_clone = ctx.clone();

    // 获取配置
    io.add_method("get_config", move |_| {
        let ctx = ctx_clone.clone();
        async move {
            match ctx.rd.get_config(|c| c.to_owned()).await {
                Ok(config) => Ok(Value::String(config)),
                Err(e) => Err(JsonRpcError::internal_error()),
            }
        }
    });

    let ctx_clone = ctx.clone();
    // 设置配置
    io.add_method("post_config", move |params: Params| {
        let ctx = ctx_clone.clone();
        async move {
            let source: ImportSource = params.parse()?;
            let stream = ctx
                .cfg_mgr
                .config_stream(source)
                .await
                .map_err(|_| JsonRpcError::internal_error())?;

            ctx.rd
                .stop()
                .await
                .map_err(|_| JsonRpcError::internal_error())?;
            let rd = ctx.rd.clone();
            tokio::spawn(rd.start_stream(stream));

            Ok(Value::Null)
        }
    });

    let ctx_clone = ctx.clone();
    // 获取注册表
    io.add_method("get_registry", move |_| {
        let ctx = ctx_clone.clone();
        async move {
            ctx.rd
                .registry(|r| serde_json::to_value(r))
                .await
                .map_err(|_| JsonRpcError::internal_error())
        }
    });

    let ctx_clone = ctx.clone();
    // 获取连接
    io.add_method("get_connections", move |_| {
        let ctx = ctx_clone.clone();
        async move {
            ctx.rd
                .connection(|c| serde_json::to_value(c))
                .await
                .map_err(|_| JsonRpcError::internal_error())
        }
    });

    let ctx_clone = ctx.clone();
    // 删除所有连接
    io.add_method("delete_connections", move |_| {
        let ctx = ctx_clone.clone();
        async move {
            ctx.rd
                .stop_connections()
                .await
                .map(|n| Value::Number(n.into()))
                .map_err(|_| JsonRpcError::internal_error())
        }
    });

    let ctx_clone = ctx.clone();
    // 获取状态
    io.add_method("get_state", move |_| {
        let ctx = ctx_clone.clone();
        async move {
            ctx.rd
                .state_str()
                .await
                .map_err(|_| JsonRpcError::internal_error())
                .map(|s| Value::String(s.to_string()))
        }
    });

    let ctx_clone = ctx.clone();
    // 选择节点
    io.add_method("post_select", move |params: Params| {
        let ctx = ctx_clone.clone();
        async move {
            #[derive(Deserialize)]
            struct SelectParams {
                net_name: String,
                selected: String,
            }

            let params: SelectParams = params.parse()?;

            ctx.rd
                .update_net(&params.net_name, |o| {
                    if o.net_type == "select" {
                        if let Some(o) = o.opt.as_object_mut() {
                            o.insert("selected".to_string(), params.selected.clone().into());
                        }
                    }
                })
                .await
                .map_err(|_| JsonRpcError::internal_error())?;

            if let Some(id) = ctx.rd.get_id().await {
                let mut select_map = SelectMap::from_cache(&id, ctx.cfg_mgr.select_storage())
                    .await
                    .map_err(|_| JsonRpcError::internal_error())?;

                select_map.insert(params.net_name.to_string(), params.selected);

                select_map
                    .write_cache(&id, ctx.cfg_mgr.select_storage())
                    .await
                    .map_err(|_| JsonRpcError::internal_error())?;
            }

            Ok(Value::Null)
        }
    });

    let ctx_clone = ctx.clone();
    // 删除连接
    io.add_method("delete_conn", move |params: Params| {
        let ctx = ctx_clone.clone();
        async move {
            let uuid: String = params.parse()?;
            ctx.rd
                .stop_connection(
                    uuid.parse()
                        .map_err(|_| JsonRpcError::invalid_params("Invalid UUID"))?,
                )
                .await
                .map_err(|_| JsonRpcError::internal_error())
                .map(Value::Bool)
        }
    });

    let ctx_clone = ctx.clone();
    // 测试延迟
    io.add_method("get_delay", move |params: Params| {
        let ctx = ctx_clone.clone();
        async move {
            #[derive(Deserialize)]
            struct DelayParams {
                net_name: String,
                url: url::Url,
                timeout: Option<u64>,
            }

            let params: DelayParams = params.parse()?;

            let net = ctx
                .rd
                .get_net(&params.net_name)
                .await
                .map_err(|_| JsonRpcError::internal_error())?
                .map(|n| n.as_net());

            let host = params.url.host_str();
            let port = params.url.port_or_known_default();
            let timeout = params.timeout.unwrap_or(5000);

            match (net, host, port) {
                (Some(net), Some(host), Some(port)) => {
                    let start = Instant::now();
                    let fut = async {
                        let socket = net
                            .tcp_connect(
                                &mut rd_interface::Context::new(),
                                &(host, port)
                                    .into_address()
                                    .map_err(|_| JsonRpcError::internal_error())?,
                            )
                            .await
                            .map_err(|_| JsonRpcError::internal_error())?;

                        let connect = start.elapsed().as_millis() as u64;
                        let (mut request_sender, connection) =
                            hyper::client::conn::handshake(socket)
                                .await
                                .map_err(|_| JsonRpcError::internal_error())?;

                        let connect_req = hyper::Request::builder()
                            .method("GET")
                            .uri(params.url.path())
                            .body(hyper::Body::empty())
                            .map_err(|_| JsonRpcError::internal_error())?;

                        tokio::spawn(connection);
                        request_sender
                            .send_request(connect_req)
                            .await
                            .map_err(|_| JsonRpcError::internal_error())?;

                        let response = start.elapsed().as_millis() as u64;

                        Ok::<_, JsonRpcError>(DelayResponse { connect, response })
                    };

                    match tokio::time::timeout(Duration::from_millis(timeout), fut).await {
                        Ok(Ok(resp)) => {
                            serde_json::to_value(resp).map_err(|_| JsonRpcError::internal_error())
                        }
                        _ => Ok(Value::Null),
                    }
                }
                _ => Ok(Value::Null),
            }
        }
    });

    let ctx_clone = ctx.clone();
    // 获取用户数据
    io.add_method("get_userdata", move |params: Params| {
        let ctx = ctx_clone.clone();
        async move {
            let key: String = params.parse()?;
            let item = ctx
                .userdata
                .get(&key)
                .await
                .map_err(|_| JsonRpcError::internal_error())?
                .ok_or_else(|| JsonRpcError::internal_error())?;

            Ok(serde_json::to_value(item).map_err(|_| JsonRpcError::internal_error())?)
        }
    });

    let ctx_clone = ctx.clone();
    // 设置用户数据
    io.add_method("put_userdata", move |params: Params| {
        let ctx = ctx_clone.clone();
        async move {
            #[derive(Deserialize)]
            struct UserDataParams {
                key: String,
                value: String,
            }

            let params: UserDataParams = params.parse()?;

            ctx.userdata
                .set(&params.key, &params.value)
                .await
                .map_err(|_| JsonRpcError::internal_error())?;

            Ok(json!({"ok": true}))
        }
    });

    let ctx_clone = ctx.clone();
    // 删除用户数据
    io.add_method("delete_userdata", move |params: Params| {
        let ctx = ctx_clone.clone();
        async move {
            let key: String = params.parse()?;
            ctx.userdata
                .remove(&key)
                .await
                .map_err(|_| JsonRpcError::internal_error())?;

            Ok(json!({"ok": true}))
        }
    });

    let ctx_clone = ctx.clone();
    // 列出用户数据
    io.add_method("list_userdata", move |_| {
        let ctx = ctx_clone.clone();
        async move {
            let keys = ctx
                .userdata
                .keys()
                .await
                .map_err(|_| JsonRpcError::internal_error())?;

            Ok(json!({"keys": keys}))
        }
    });

    while let Some(Ok(msg)) = socket.recv().await {
        if let Message::Text(text) = msg {
            if let Some(response) = io.handle_request(&text).await {
                if let Err(e) = socket.send(Message::Text(response)).await {
                    tracing::error!("Failed to send response: {}", e);
                    break;
                }
            }
        }
    }
}

pub(crate) async fn websocket_handler(
    ws: WebSocketUpgrade,
    Extension(ctx): Extension<Ctx>,
) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, Arc::new(ctx)))
}
