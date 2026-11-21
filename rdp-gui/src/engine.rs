//! Runs the proxy engine inside the GUI process, so the native app is one
//! binary instead of a window plus a daemon.
//!
//! The engine keeps the exact shape `rabbit-digger-pro`'s own `main` uses: the
//! API server answers on an ephemeral port, and the config file becomes the
//! stream that starts and reloads the runtime. The UI then talks to that port
//! over the same JSON-RPC it would use against a remote daemon.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use rabbit_digger_pro::{config::ImportSource, ApiServerConfig, App};

/// Boots the engine on a background thread and returns the API address.
pub fn start(config: PathBuf) -> Result<SocketAddr> {
    let (ready, wait) = std::sync::mpsc::channel::<Result<SocketAddr, String>>();

    std::thread::Builder::new()
        .name("rdp-engine".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready.send(Err(format!("engine runtime failed: {error}")));
                    return;
                }
            };

            runtime.block_on(async move {
                match boot(config, &ready).await {
                    // The stream ends when `config.apply` or `engine.stop`
                    // stops the runtime. The API server stays essential: a
                    // replacement stream or a later start runs in this same
                    // runtime, so keep it (and the API server task) alive.
                    Ok(()) => futures::future::pending::<()>().await,
                    Err(error) => {
                        // The receiver is gone once the UI has the address.
                        let _ = ready.send(Err(error.to_string()));
                    }
                }
            });
        })
        .context("failed to spawn the engine thread")?;

    match wait.recv() {
        Ok(Ok(addr)) => Ok(addr),
        Ok(Err(message)) => Err(anyhow!(message)),
        Err(error) => Err(anyhow!("engine thread stopped before reporting: {error}")),
    }
}

async fn boot(
    config: PathBuf,
    ready: &std::sync::mpsc::Sender<Result<SocketAddr, String>>,
) -> Result<()> {
    let app = App::new().await.context("failed to create the runtime")?;

    // The same shape the daemon uses: the initial file is the first source, and
    // `config.apply` feeds new sources through this channel instead of stopping
    // the engine from the API task. One engine loop owns the lifecycle.
    let (source_tx, source_rx) = tokio::sync::mpsc::channel(4);

    let addr = app
        .run_api_server(ApiServerConfig {
            bind: Some("127.0.0.1:0".to_string()),
            source_sender: Some(source_tx.clone()),
            ..Default::default()
        })
        .await
        .context("failed to start the API server")?
        .ok_or_else(|| anyhow!("the API server did not bind"))?;

    if ready.send(Ok(addr)).is_err() {
        return Ok(());
    }

    let sources = futures::stream::once(async move { ImportSource::Path(config) })
        .chain(tokio_stream::wrappers::ReceiverStream::new(source_rx));
    let config_stream = app
        .cfg_mgr
        .config_stream_from_sources(sources)
        .await
        .context("failed to read the config file")?;

    app.rd
        .start_stream(config_stream)
        .await
        .context("the engine stopped")?;

    Ok(())
}

/// Sends tracing events to the broadcast channel the `logs` RPC topic reads, so
/// the log page shows what the embedded engine says.
pub fn init_logging() {
    use tracing_subscriber::prelude::*;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(
            "rabbit_digger=info,rabbit_digger_pro=info,rd_std=info,raw=info,ss=info",
        )
    });

    let subscriber = tracing_subscriber::registry().with(filter).with(
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(rabbit_digger_pro::log::LogWriter::new),
    );

    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio_tungstenite::tungstenite::Message;

    async fn call(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        id: u64,
        method: &str,
        params: Value,
    ) -> Value {
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        socket
            .send(Message::Text(request.to_string().into()))
            .await
            .expect("send the RPC request");
        loop {
            let message = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
                .await
                .expect("the engine answers in time")
                .expect("the socket stays open")
                .expect("the frame is valid");
            let text = message.to_text().expect("the reply is text");
            let value: Value = serde_json::from_str(text).expect("the reply is JSON");
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return value;
            }
        }
    }

    #[test]
    fn embedded_engine_survives_a_config_reload() {
        let dir = std::env::temp_dir().join(format!("rdp-gui-engine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the temp config dir");
        // Keep app state (userdata, select cache, side effects) in the temp
        // directory instead of the developer's real profile.
        std::env::set_var("RDP_DATA_DIR", dir.join("data"));
        std::env::set_var("RDP_CONFIG_DIR", dir.join("config"));
        std::env::set_var("RDP_CACHE_DIR", dir.join("cache"));
        let config = dir.join("config.yaml");
        std::fs::write(
            &config,
            "net:\n  local:\n    type: local\nserver:\n  mixed:\n    type: http+socks5\n    bind: 127.0.0.1:0\n    net: local\n",
        )
        .expect("write the temp config");

        let addr = start(config).expect("the embedded engine starts");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");

        runtime.block_on(async {
            let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/rpc"))
                .await
                .expect("the API socket accepts connections");

            let reply = call(&mut socket, 1, "config.get", json!({})).await;
            assert!(reply.get("result").is_some(), "{reply}");

            // The settings page applies a text config; this stops the file
            // stream and starts a replacement in the same runtime.
            let reply = call(
                &mut socket,
                2,
                "config.apply",
                json!({
                    "text": "id: applied\nnet:\n  local:\n    type: local\nserver:\n  mixed:\n    type: http+socks5\n    bind: 127.0.0.1:0\n    net: local\n",
                }),
            )
            .await;
            assert_eq!(reply.get("result"), Some(&Value::Null), "{reply}");

            // The API stays up and the replacement stream becomes the config.
            // Import sources set the engine config id to their cache key, so a
            // text config lands as `text` while the file stream starts as
            // `path:...`.
            let mut applied = false;
            for id in 3..40 {
                let reply = call(&mut socket, id, "config.get", json!({})).await;
                let loaded = reply
                    .get("result")
                    .and_then(|result| result.get("id"))
                    .and_then(Value::as_str);
                if loaded == Some("text") {
                    applied = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            assert!(applied, "the applied config never reached the engine");

            let reply = call(&mut socket, 100, "engine.stop", json!({})).await;
            assert!(reply.get("error").is_none(), "{reply}");

            // The engine loop keeps waiting after a stop, so the settings page
            // can start it again without a new process.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let reply = call(
                &mut socket,
                101,
                "config.apply",
                json!({
                    "text": "id: restarted\nnet:\n  local:\n    type: local\nserver:\n  mixed:\n    type: http+socks5\n    bind: 127.0.0.1:0\n    net: local\n",
                }),
            )
            .await;
            assert_eq!(reply.get("result"), Some(&Value::Null), "{reply}");

            let mut restarted = false;
            for id in 102..150 {
                let reply = call(&mut socket, id, "config.get", json!({})).await;
                let loaded = reply
                    .get("result")
                    .and_then(|result| result.get("id"))
                    .and_then(Value::as_str);
                if loaded == Some("text") {
                    restarted = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            assert!(restarted, "the engine did not start again after a stop");
        });
    }
}
