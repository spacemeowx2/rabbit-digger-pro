use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    config::ImportSource,
    storage::{FileStorage, FolderType, Storage},
    ApiServerConfig, App,
};

const LAST_SOURCE_KEY: &str = "daemon/last_source";

fn log_file_path() -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("rabbit_digger_pro")
        .join("daemon.log")
}

fn side_effects_path() -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("rabbit_digger_pro")
        .join("side_effects.json")
}

pub async fn run_daemon(bind: String, access_token: Option<String>) -> Result<()> {
    // Crash recovery: clean up any side effects from a previous crash
    let se_path = side_effects_path();
    rd_interface::SideEffectManager::recover(&se_path.to_string_lossy());

    let app = App::new().await?;

    // Channel: API handlers send ImportSource here → main loop feeds to engine
    let (source_tx, source_rx) = mpsc::channel(4);

    app.run_api_server(ApiServerConfig {
        bind: Some(bind.clone()),
        access_token,
        web_ui: None,
        source_sender: Some(source_tx.clone()),
        log_file_path: Some(log_file_path()),
    })
    .await
    .context("Failed to start API server")?;

    tracing::info!("Daemon started, WebUI available at http://{}", bind);

    // Restore last config if available
    let userdata = FileStorage::new(FolderType::Data, "userdata").await?;
    if let Ok(Some(item)) = userdata.get(LAST_SOURCE_KEY).await {
        match serde_json::from_str::<ImportSource>(&item.content) {
            Ok(source) => {
                tracing::info!("Restoring last configuration...");
                let _ = source_tx.send(source).await;
            }
            Err(e) => {
                tracing::warn!("Failed to parse saved config source: {e}");
            }
        }
    } else {
        tracing::info!("No saved configuration, waiting for WebUI...");
    }

    // Convert channel receiver into a stream for config_stream_from_sources
    let source_stream = ReceiverStream::new(source_rx);

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
