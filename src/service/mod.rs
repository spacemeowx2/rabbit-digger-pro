mod daemon;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use anyhow::{Context, Result};
use clap::Parser;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
use std::process::Command;

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
    /// Install rdp as a user-level service and start it
    #[cfg(target_os = "linux")]
    InstallUser {
        /// Bind address for the API/WebUI server
        #[clap(long, default_value = "127.0.0.1:9091")]
        bind: String,
        /// Access token for API authentication
        #[clap(long)]
        access_token: Option<String>,
        /// Binary to install. Defaults to the current executable.
        #[clap(long)]
        binary: Option<PathBuf>,
    },
    /// Uninstall the system service
    Uninstall,
    /// Uninstall the user-level service
    #[cfg(target_os = "linux")]
    UninstallUser,
    /// Start the installed service
    Start,
    /// Start the installed user-level service
    #[cfg(target_os = "linux")]
    StartUser,
    /// Stop the installed service
    Stop,
    /// Stop the installed user-level service
    #[cfg(target_os = "linux")]
    StopUser,
    /// Show service status
    Status,
    /// Show user-level service status
    #[cfg(target_os = "linux")]
    StatusUser,
    /// Run in daemon mode (called by the service manager)
    Run {
        /// Bind address for the API/WebUI server
        #[clap(long, default_value = "127.0.0.1:9091")]
        bind: String,
        /// Access token for API authentication
        #[clap(long, env = "RD_ACCESS_TOKEN")]
        access_token: Option<String>,
    },
}

const SERVICE_LABEL: &str = "com.rabbit-digger-pro";

/// Re-exec the current process under `sudo` if not already root.
#[cfg(unix)]
fn ensure_root() -> Result<()> {
    let output = Command::new("id").arg("-u").output()?;
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if uid == "0" {
        return Ok(());
    }

    let exe = std::env::current_exe().context("Cannot determine binary path")?;
    let args: Vec<String> = std::env::args().skip(1).collect();

    let status = Command::new("sudo")
        .arg(exe)
        .args(&args)
        .status()
        .context("Failed to run sudo")?;

    std::process::exit(status.code().unwrap_or(1));
}

pub async fn run(action: ServiceAction) -> Result<()> {
    match action {
        ServiceAction::Run { bind, access_token } => daemon::run_daemon(bind, access_token).await,
        #[cfg(target_os = "macos")]
        action => macos::handle_service_action(action).await,
        #[cfg(target_os = "linux")]
        action => linux::handle_service_action(action).await,
        #[cfg(target_os = "windows")]
        action => windows::handle_service_action(action).await,
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        _ => {
            anyhow::bail!("System service management is not supported on this platform. Use 'service run' to run in daemon mode manually.")
        }
    }
}
