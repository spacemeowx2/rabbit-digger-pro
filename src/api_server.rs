use std::net::SocketAddr;

use anyhow::Result;
use rabbit_digger::RabbitDigger;
use tokio::net::TcpListener;

use crate::config::ConfigManager;

mod handlers;
mod routes;

pub struct ApiServer {
    pub rabbit_digger: RabbitDigger,
    pub config_manager: ConfigManager,
    pub access_token: Option<String>,
    pub web_ui: Option<String>,
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
    use futures::StreamExt;
    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::connect_async;

    #[tokio::test]
    async fn test_api_server_smoke_http() {
        let app = crate::App::new().await.unwrap();

        // Start rabbit-digger with a minimal config so endpoints like /api/config work.
        let mut cfg = rabbit_digger::config::Config::default();
        cfg.id = "test".to_string();
        rabbit_digger::config::init_default_net(&mut cfg.net).unwrap();
        app.rd.start(cfg).await.unwrap();

        let server = ApiServer {
            rabbit_digger: app.rd.clone(),
            config_manager: app.cfg_mgr.clone(),
            access_token: None,
            web_ui: None,
        };
        let addr = server.run("127.0.0.1:0").await.unwrap();
        let base = format!("http://{addr}");

        let client = reqwest::Client::new();

        async fn get_until_ok(client: &reqwest::Client, url: String) -> reqwest::Response {
            for _ in 0..100 {
                if let Ok(resp) = client.get(&url).send().await {
                    if resp.status().is_success() {
                        return resp;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            client.get(&url).send().await.unwrap()
        }

        let r = get_until_ok(&client, format!("{base}/api/get")).await;
        if !r.status().is_success() {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            panic!("/api/get status={status} body={body}");
        }

        let r = get_until_ok(&client, format!("{base}/api/state")).await;
        if !r.status().is_success() {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            panic!("/api/state status={status} body={body}");
        }

        let r = get_until_ok(&client, format!("{base}/api/config")).await;
        if !r.status().is_success() {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            panic!("/api/config status={status} body={body}");
        }
        assert_eq!(
            r.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/json")
        );

        let key = format!("test-{}", uuid::Uuid::new_v4());
        let r = client
            .put(format!("{base}/api/userdata/{key}"))
            .body("hello")
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
        let v: Value = r.json().await.unwrap();
        assert_eq!(v.get("copied").and_then(|n| n.as_u64()), Some(5));

        let r = client
            .get(format!("{base}/api/userdata/{key}"))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
        let v: Value = r.json().await.unwrap();
        assert_eq!(v.get("content").and_then(|s| s.as_str()), Some("hello"));

        let r = client
            .get(format!("{base}/api/userdata"))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
        let v: Value = r.json().await.unwrap();
        let keys = v.get("keys").and_then(|v| v.as_array()).unwrap();
        assert!(keys
            .iter()
            .filter_map(|o| o.get("key").and_then(|k| k.as_str()))
            .any(|k| k == key));

        let r = client
            .delete(format!("{base}/api/userdata/{key}"))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
        let v: Value = r.json().await.unwrap();
        assert_eq!(v.get("ok").and_then(|b| b.as_bool()), Some(true));
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
        };
        let addr = server.run("127.0.0.1:0").await.unwrap();
        let base = format!("http://{addr}");

        let client = reqwest::Client::new();
        let r = client
            .get(format!("{base}/index.html"))
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
        };
        let addr = server.run("127.0.0.1:0").await.unwrap();
        let base = format!("http://{addr}");

        let client = reqwest::Client::new();

        let r = client
            .get(format!("{base}/api/state"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);

        let r = client
            .get(format!("{base}/api/state"))
            .header(reqwest::header::AUTHORIZATION, "secret")
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
    }

    #[tokio::test]
    async fn test_api_server_handlers_and_websockets() {
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
        };
        let addr = server.run("127.0.0.1:0").await.unwrap();
        let base = format!("http://{addr}");
        let ws_base = format!("ws://{addr}");

        let client = reqwest::Client::new();

        // Wait for rd to be fully running.
        for _ in 0..100 {
            let r = client
                .get(format!("{base}/api/state"))
                .send()
                .await
                .unwrap();
            if r.status().is_success() {
                let v: Value = r.json().await.unwrap();
                if v.as_str() == Some("Running") {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        // Ensure the local net exists.
        let local = app.rd.get_net("local").await.unwrap();
        assert!(local.is_some());

        // post_select (should be ok even if net isn't select)
        let r = client
            .post(format!("{base}/api/net/local"))
            .json(&serde_json::json!({"selected": "local"}))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());

        // get_delay early-return branch (no host/port)
        let r = client
            .get(format!(
                "{base}/api/net/local/delay?url=file%3A%2F%2F%2Ftmp&timeout=1"
            ))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());

        // get_userdata not found -> ApiError::NotFound
        let r = client
            .get(format!("{base}/api/userdata/does-not-exist"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);
        let v: Value = r.json().await.unwrap();
        assert_eq!(v.get("error").and_then(|s| s.as_str()), Some("Not found"));

        // put_userdata invalid UTF-8 -> ApiError::Other
        let r = client
            .put(format!("{base}/api/userdata/bad-utf8"))
            .body(vec![0xff, 0xfe, 0xfd])
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);
        let v: Value = r.json().await.unwrap();
        assert!(v.get("error").is_some());

        // delete_conn with random uuid should return a bool
        let r = client
            .delete(format!("{base}/api/connection/{}", uuid::Uuid::new_v4()))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
        let _v: Value = r.json().await.unwrap();

        // delete_connections should return json
        let r = client
            .delete(format!("{base}/api/connection"))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
        let _v: Value = r.json().await.unwrap();

        // WebSocket: connection stream
        let (mut ws, _) = connect_async(format!(
            "{ws_base}/api/stream/connection?patch=true&without_connections=true"
        ))
        .await
        .unwrap();
        let msg = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let text = match msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t,
            other => panic!("unexpected ws message: {other:?}"),
        };
        let v: Value = serde_json::from_str(&text).unwrap();
        assert!(v.get("full").is_some());

        ws.close(None).await.unwrap();

        // WebSocket: logs stream
        let (mut logs_ws, _) = connect_async(format!("{ws_base}/api/stream/logs"))
            .await
            .unwrap();
        crate::log::get_sender()
            .send(Box::<[u8]>::from(&b"hello"[..]))
            .ok();

        let msg = tokio::time::timeout(std::time::Duration::from_secs(3), logs_ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let text = match msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t,
            other => panic!("unexpected ws message: {other:?}"),
        };
        assert!(text.contains("hello"));
        logs_ws.close(None).await.unwrap();

        // get_delay success path: connect, write request, and receive 1 byte
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let delay_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ =
                tokio::time::timeout(std::time::Duration::from_millis(200), s.read(&mut buf)).await;
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\nX").await;
        });

        let mut url = reqwest::Url::parse(&format!("{base}/api/net/local/delay")).unwrap();
        url.query_pairs_mut()
            .append_pair("url", &format!("http://127.0.0.1:{}/", delay_addr.port()))
            .append_pair("timeout", "5000");
        let r = client.get(url).send().await.unwrap();
        assert!(r.status().is_success());
        let v: Value = r.json().await.unwrap();
        assert!(v.is_object());
        assert!(v.get("connect").is_some());
        assert!(v.get("response").is_some());

        // get_delay timeout path -> null
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let delay_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let _ = s.write_all(b"X").await;
        });
        let mut url = reqwest::Url::parse(&format!("{base}/api/net/local/delay")).unwrap();
        url.query_pairs_mut()
            .append_pair("url", &format!("http://127.0.0.1:{}/", delay_addr.port()))
            .append_pair("timeout", "5");
        let r = client.get(url).send().await.unwrap();
        assert!(r.status().is_success());
        let v: Value = r.json().await.unwrap();
        assert!(v.is_null());

        // post_config with ImportSource::Text (do this last; it stops/restarts rd)
        let new_cfg = "id: test2\nnet: {}\nserver: {}\n";
        let r = client
            .post(format!("{base}/api/config"))
            .json(&serde_json::json!({"text": new_cfg}))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
        assert_eq!(r.text().await.unwrap(), "null");
    }
}
