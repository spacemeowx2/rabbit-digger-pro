mod daemon;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
mod windows;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
pub enum ServiceAction {
    /// Install rdp as a system service and start it
    Install {
        /// Bind address for the API/WebUI server
        #[clap(long, default_value = "127.0.0.1:9091")]
        bind: String,
        /// Access token for API authentication
        #[clap(long)]
        access_token: Option<String>,
    },
    /// Uninstall the system service
    Uninstall,
    /// Start the installed service
    Start,
    /// Stop the installed service
    Stop,
    /// Show service status
    Status,
    /// Run in daemon mode (called by the service manager)
    Run {
        /// Bind address for the API/WebUI server
        #[clap(long, default_value = "127.0.0.1:9091")]
        bind: String,
        /// Access token for API authentication
        #[clap(long)]
        access_token: Option<String>,
    },
}

const SERVICE_LABEL: &str = "com.rabbit-digger-pro";

pub async fn run(action: ServiceAction) -> Result<()> {
    match action {
        ServiceAction::Run { bind, access_token } => daemon::run_daemon(bind, access_token).await,
        #[cfg(target_os = "macos")]
        action => macos::handle_service_action(action).await,
        #[cfg(target_os = "windows")]
        action => windows::handle_service_action(action).await,
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        _ => {
            anyhow::bail!("System service management is not supported on this platform. Use 'service run' to run in daemon mode manually.")
        }
    }
}
