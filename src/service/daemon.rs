use std::future::Future;

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
    crate::util::app_dirs::data_dir().join("daemon.log")
}

fn side_effects_path() -> std::path::PathBuf {
    crate::util::app_dirs::data_dir().join("side_effects.json")
}

pub async fn run_daemon(bind: String, access_token: Option<String>) -> Result<()> {
    run_daemon_with_shutdown(bind, access_token, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}

/// Runs the daemon until the engine/config stream ends or `shutdown` fires.
/// The Windows service handler uses this to stop the engine gracefully so
/// `SideEffectManager` can remove TUN routes and restore DNS.
pub async fn run_daemon_with_shutdown<F>(
    bind: String,
    access_token: Option<String>,
    shutdown: F,
) -> Result<()>
where
    F: Future<Output = ()>,
{
    // Crash recovery: clean up any side effects from a previous crash
    let se_path = side_effects_path();
    rd_interface::SideEffectManager::recover(&se_path.to_string_lossy());

    let app = App::new().await?;
    tracing::info!("Daemon app created");

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

    tracing::info!("Daemon started, API available at http://{}", bind);

    // Restore the last config in the background. Opening userdata must not
    // block the config stream: a service can receive a config.apply before the
    // old source is read, especially after a crash.
    let restore_tx = source_tx.clone();
    tokio::spawn(async move {
        tracing::info!("Opening daemon userdata");
        let userdata = match FileStorage::new(FolderType::Data, "userdata").await {
            Ok(userdata) => userdata,
            Err(error) => {
                tracing::warn!("Failed to open daemon userdata: {error:#}");
                return;
            }
        };
        tracing::info!("Daemon userdata opened");
        if let Ok(Some(item)) = userdata.get(LAST_SOURCE_KEY).await {
            tracing::info!("Restoring last configuration");
            match serde_json::from_str::<ImportSource>(&item.content) {
                Ok(mut source) => {
                    if let ImportSource::Text(text) = &mut source {
                        *text = text.trim_start_matches('\u{feff}').to_string();
                    }
                    let _ = restore_tx.send(source).await;
                }
                Err(e) => {
                    tracing::warn!("Failed to parse saved config source: {e}");
                }
            }
        } else {
            tracing::info!("No saved configuration, waiting for a config import...");
        }
    });

    // Convert channel receiver into a stream for config_stream_from_sources
    let source_stream = ReceiverStream::new(source_rx);

    let rd = app.rd.clone();
    let cfg_mgr = app.cfg_mgr.clone();
    tracing::info!("Daemon waiting for config stream");

    tokio::select! {
        result = async {
            let config_stream = cfg_mgr.config_stream_from_sources(source_stream).await?;
            tracing::info!("Daemon config stream ready");
            futures::pin_mut!(config_stream);
            rd.start_stream(config_stream).await
        } => {
            if let Err(e) = result {
                tracing::error!("Engine loop exited with error: {:?}", e);
            }
        }
        _ = shutdown => {
            tracing::info!("Shutting down daemon...");
            app.rd.stop().await?;
        }
    }

    Ok(())
}
