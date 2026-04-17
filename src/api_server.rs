use std::net::SocketAddr;

use anyhow::Result;
use rabbit_digger::RabbitDigger;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::config::{ConfigManager, ImportSource};

#[cfg(feature = "webui_embed")]
mod embedded_webui;
mod handlers;
mod routes;
mod rpc;

pub struct ApiServer {
    pub rabbit_digger: RabbitDigger,
    pub config_manager: ConfigManager,
    pub access_token: Option<String>,
    pub web_ui: Option<String>,
    /// When set, config.apply sends ImportSource through this channel
    /// instead of spawning start_stream directly.
    pub source_sender: Option<mpsc::Sender<ImportSource>>,
    /// Path to the daemon log file (JSON lines), for logs.tail.
    pub log_file_path: Option<std::path::PathBuf>,
}

impl ApiServer {
    pub async fn run(self, bind: &str) -> Result<SocketAddr> {
        let app = self.routes().await?;

        let listener = TcpListener::bind(bind).await?;
        let local_addr = listener.local_addr()?;
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("api server exited: {e}");
            }
        });

        Ok(local_addr)
    }
}

#[cfg(all(test, feature = "api_server"))]
mod tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    fn test_http_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    async fn rpc_call(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        id: u64,
        method: &str,
        params: Value,
    ) -> Value {
        ws.send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

        loop {
            let message = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let Message::Text(text) = message {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value.get("id").and_then(|value| value.as_u64()) == Some(id) {
                    return value;
                }
            }
        }
    }

    async fn rpc_notification(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Value {
        loop {
            let message = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let Message::Text(text) = message {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value.get("method").and_then(|value| value.as_str()) == Some("rpc.subscription")
                {
                    return value;
                }
            }
        }
    }

    #[tokio::test]
    async fn test_api_server_smoke_rpc() {
        let app = crate::App::new().await.unwrap();

        let mut cfg = rabbit_digger::config::Config::default();
        cfg.id = "test".to_string();
        rabbit_digger::config::init_default_net(&mut cfg.net).unwrap();
        cfg.server.insert(
            "echo".to_string(),
            rabbit_digger::config::Server::new_opt(
                "echo",
                serde_json::json!({"bind":"127.0.0.1:0","listen":"local"}),
            )
            .unwrap(),
        );
        app.rd.start(cfg).await.unwrap();

        let server = ApiServer {
            rabbit_digger: app.rd.clone(),
            config_manager: app.cfg_mgr.clone(),
            access_token: None,
            web_ui: None,
            source_sender: None,
            log_file_path: None,
        };
        let addr = server.run("127.0.0.1:0").await.unwrap();
        let ws_base = format!("ws://{addr}");

        let (mut ws, _) = connect_async(format!("{ws_base}/api/rpc")).await.unwrap();

        let response = rpc_call(&mut ws, 1, "registry.get", json!({})).await;
        assert!(response.get("result").is_some());

        let response = rpc_call(&mut ws, 2, "config.get", json!({})).await;
        assert!(response.get("result").is_some());

        let key = format!("test-{}", uuid::Uuid::new_v4());
        let response = rpc_call(
            &mut ws,
            3,
            "userdata.put",
            json!({ "path": key, "value": "hello" }),
        )
        .await;
        assert_eq!(
            response
                .get("result")
                .and_then(|value| value.get("copied"))
                .and_then(|value| value.as_u64()),
            Some(5)
        );

        let response = rpc_call(&mut ws, 4, "userdata.get", json!({ "path": key })).await;
        assert_eq!(
            response
                .get("result")
                .and_then(|value| value.get("content"))
                .and_then(|value| value.as_str()),
            Some("hello")
        );

        let response = rpc_call(&mut ws, 5, "userdata.list", json!({})).await;
        let keys = response
            .get("result")
            .and_then(|value| value.get("keys"))
            .and_then(|value| value.as_array())
            .unwrap();
        assert!(keys
            .iter()
            .filter_map(|o| o.get("key").and_then(|k| k.as_str()))
            .any(|k| k == key));

        let response = rpc_call(&mut ws, 6, "userdata.delete", json!({ "path": key })).await;
        assert_eq!(
            response
                .get("result")
                .and_then(|value| value.get("ok"))
                .and_then(|value| value.as_bool()),
            Some(true)
        );

        let response = rpc_call(
            &mut ws,
            7,
            "net.select",
            json!({ "net_name": "local", "selected": "local" }),
        )
        .await;
        assert_eq!(response.get("result"), Some(&Value::Null));

        let response = rpc_call(
            &mut ws,
            8,
            "rpc.subscribe",
            json!({ "topic": "engine.events", "params": {} }),
        )
        .await;
        let engine_subscription = response
            .get("result")
            .and_then(|value| value.get("subscription"))
            .and_then(|value| value.as_str())
            .unwrap()
            .to_string();
        let notification = rpc_notification(&mut ws).await;
        assert_eq!(
            notification
                .get("params")
                .and_then(|value| value.get("subscription"))
                .and_then(|value| value.as_str()),
            Some(engine_subscription.as_str())
        );

        let response = rpc_call(
            &mut ws,
            9,
            "rpc.subscribe",
            json!({
                "topic": "connections",
                "params": { "patch": true, "without_connections": true }
            }),
        )
        .await;
        let connection_subscription = response
            .get("result")
            .and_then(|value| value.get("subscription"))
            .and_then(|value| value.as_str())
            .unwrap()
            .to_string();
        let notification = rpc_notification(&mut ws).await;
        assert_eq!(
            notification
                .get("params")
                .and_then(|value| value.get("subscription"))
                .and_then(|value| value.as_str()),
            Some(connection_subscription.as_str())
        );
        assert!(notification
            .get("params")
            .and_then(|value| value.get("payload"))
            .and_then(|value| value.get("full"))
            .is_some());

        let response = rpc_call(
            &mut ws,
            10,
            "rpc.subscribe",
            json!({ "topic": "logs", "params": {} }),
        )
        .await;
        let log_subscription = response
            .get("result")
            .and_then(|value| value.get("subscription"))
            .and_then(|value| value.as_str())
            .unwrap()
            .to_string();
        crate::log::get_sender()
            .send(Box::<[u8]>::from(&b"hello"[..]))
            .ok();
        let notification = rpc_notification(&mut ws).await;
        assert_eq!(
            notification
                .get("params")
                .and_then(|value| value.get("subscription"))
                .and_then(|value| value.as_str()),
            Some(log_subscription.as_str())
        );
        assert!(notification
            .get("params")
            .and_then(|value| value.get("payload"))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .contains("hello"));
    }

    #[tokio::test]
    async fn test_api_server_web_ui_fallback_serves_static_files() {
        let app = crate::App::new().await.unwrap();

        let mut cfg = rabbit_digger::config::Config::default();
        cfg.id = "test".to_string();
        rabbit_digger::config::init_default_net(&mut cfg.net).unwrap();
        app.rd.start(cfg).await.unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let index_path = tmp.path().join("index.html");
        tokio::fs::write(&index_path, b"<h1>ok</h1>").await.unwrap();

        let server = ApiServer {
            rabbit_digger: app.rd.clone(),
            config_manager: app.cfg_mgr.clone(),
            access_token: None,
            web_ui: Some(tmp.path().to_string_lossy().to_string()),
            source_sender: None,
            log_file_path: None,
        };
        let addr = server.run("127.0.0.1:0").await.unwrap();
        let base = format!("http://{addr}");

        let client = test_http_client();
        let r = client
            .get(format!("{base}/index.html"))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
        assert!(r.text().await.unwrap().contains("ok"));

        let r = client
            .get(format!("{base}/connections"))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
        assert!(r.text().await.unwrap().contains("ok"));
    }

    #[tokio::test]
    async fn test_api_server_auth_token_blocks_requests() {
        let app = crate::App::new().await.unwrap();

        let mut cfg = rabbit_digger::config::Config::default();
        cfg.id = "test".to_string();
        rabbit_digger::config::init_default_net(&mut cfg.net).unwrap();
        app.rd.start(cfg).await.unwrap();

        let server = ApiServer {
            rabbit_digger: app.rd.clone(),
            config_manager: app.cfg_mgr.clone(),
            access_token: Some("secret".to_string()),
            web_ui: None,
            source_sender: None,
            log_file_path: None,
        };
        let addr = server.run("127.0.0.1:0").await.unwrap();

        let client = test_http_client();
        let unauthorized = client
            .get(format!("http://{addr}/api/rpc"))
            .send()
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);

        let (_ws, response) = connect_async(format!("ws://{addr}/api/rpc?token=secret"))
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::SWITCHING_PROTOCOLS);
    }
}
