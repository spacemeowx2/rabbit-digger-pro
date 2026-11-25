//! Developer probe for the installed Windows service.
//!
//! ```powershell
//! $env:RDP_SERVICE_TOKEN_FILE = "$env:TEMP\opencode\rdp-service-data\service.token"
//! cargo run --example service_probe -- "$env:TEMP\opencode\rdp-tun-test.yaml" --apply-only
//! cargo run --example service_probe -- --stop
//! ```

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, http::header::AUTHORIZATION, Message,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stop_only = args.iter().any(|arg| arg == "--stop");
    let apply_only = args.iter().any(|arg| arg == "--apply-only");
    let config_path = args.iter().find(|arg| !arg.starts_with("--"));
    if !stop_only && config_path.is_none() {
        anyhow::bail!("usage: service_probe <config.yaml> [--apply-only] | --stop");
    }

    let token_file = std::env::var("RDP_SERVICE_TOKEN_FILE").unwrap_or_else(|_| {
        format!(
            "{}\\rdp-service-data\\service.token",
            std::env::temp_dir().display()
        )
    });
    let token = std::fs::read_to_string(token_file)?.trim().to_string();

    let mut request = "ws://127.0.0.1:19091/api/rpc".into_client_request()?;
    request.headers_mut().insert(AUTHORIZATION, token.parse()?);
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;

    if !stop_only {
        let config_path = config_path
            .ok_or_else(|| anyhow::anyhow!("usage: service_probe <config.yaml> [--apply-only]"))?;
        let yaml = std::fs::read_to_string(config_path)?;
        send(&mut socket, 1, "config.apply", json!({ "text": yaml })).await?;
    }
    if !apply_only {
        send(&mut socket, 2, "engine.stop", json!({})).await?;
    }

    socket.close(None).await?;
    Ok(())
}

async fn send<S>(socket: &mut S, id: u64, method: &str, params: Value) -> anyhow::Result<()>
where
    S: futures::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    let request = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    socket
        .send(Message::Text(request.to_string().into()))
        .await?;
    loop {
        let message = tokio::time::timeout(std::time::Duration::from_secs(15), socket.next())
            .await
            .map_err(|_| anyhow::anyhow!("timeout waiting for {method}"))?
            .ok_or_else(|| anyhow::anyhow!("socket closed before {method} replied"))??;
        let Message::Text(text) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(&text)?;
        if value.get("id").and_then(Value::as_u64) == Some(id) {
            println!("{method}: {value}");
            return Ok(());
        }
    }
}
