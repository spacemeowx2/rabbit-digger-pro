use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::{ApiServerConfig, App};

pub async fn run_daemon(bind: String, access_token: Option<String>) -> Result<()> {
    let app = App::new().await?;

    // Channel: API handlers send ImportSource here → main loop feeds to engine
    let (source_tx, source_rx) = mpsc::channel(4);

    app.run_api_server(ApiServerConfig {
        bind: Some(bind.clone()),
        access_token,
        web_ui: None,
        source_sender: Some(source_tx),
    })
    .await
    .context("Failed to start API server")?;

    tracing::info!("Daemon started, WebUI available at http://{}", bind);
    tracing::info!("Waiting for configuration via WebUI...");

    // Convert channel receiver into a stream for config_stream_from_sources
    let source_stream = ReceiverStream::new(source_rx);

    // This blocks until the source stream ends (all senders dropped)
    // or ctrl-c is received.
    let rd = app.rd.clone();
    let cfg_mgr = app.cfg_mgr.clone();

    tokio::select! {
        result = async {
            let config_stream = cfg_mgr.config_stream_from_sources(source_stream).await?;
            futures::pin_mut!(config_stream);
            rd.start_stream(config_stream).await
        } => {
            if let Err(e) = result {
                tracing::error!("Engine loop exited with error: {:?}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Shutting down daemon...");
            app.rd.stop().await?;
        }
    }

    Ok(())
}
