use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};

use super::{ServiceAction, SERVICE_LABEL};

fn plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Cannot determine home directory")?;
    let path = home
        .join("Library/LaunchAgents")
        .join(format!("{SERVICE_LABEL}.plist"));
    Ok(path)
}

fn log_dir() -> Result<PathBuf> {
    let data_dir = dirs::data_local_dir().context("Cannot determine data directory")?;
    let dir = data_dir.join("rabbit_digger_pro");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn generate_plist(bind: &str, access_token: Option<&str>) -> Result<String> {
    let binary_path = std::env::current_exe().context("Cannot determine binary path")?;
    let binary_path = binary_path
        .to_str()
        .context("Binary path is not valid UTF-8")?;
    let log_dir = log_dir()?;
    let stdout_log = log_dir.join("daemon.stdout.log");
    let stderr_log = log_dir.join("daemon.stderr.log");

    let mut program_args = format!(
        r#"    <array>
        <string>{binary_path}</string>
        <string>service</string>
        <string>run</string>
        <string>--bind</string>
        <string>{bind}</string>"#
    );

    if let Some(token) = access_token {
        program_args.push_str(&format!(
            r#"
        <string>--access-token</string>
        <string>{token}</string>"#
        ));
    }

    program_args.push_str("\n    </array>");

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{SERVICE_LABEL}</string>
    <key>ProgramArguments</key>
{program_args}
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{stdout}</string>
    <key>StandardErrorPath</key>
    <string>{stderr}</string>
</dict>
</plist>
"#,
        stdout = stdout_log.display(),
        stderr = stderr_log.display(),
    ))
}

pub async fn handle_service_action(action: ServiceAction) -> Result<()> {
    match action {
        ServiceAction::Install { bind, access_token } => {
            install(&bind, access_token.as_deref()).await
        }
        ServiceAction::Uninstall => uninstall().await,
        ServiceAction::Start => start().await,
        ServiceAction::Stop => stop().await,
        ServiceAction::Status => status().await,
        ServiceAction::Run { .. } => unreachable!("Run is handled by daemon module"),
    }
}

async fn install(bind: &str, access_token: Option<&str>) -> Result<()> {
    let plist = plist_path()?;

    // Ensure LaunchAgents directory exists
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Unload existing service if present
    if plist.exists() {
        let _ = Command::new("launchctl")
            .args(["unload", plist.to_str().unwrap()])
            .output();
    }

    let content = generate_plist(bind, access_token)?;
    std::fs::write(&plist, &content).context("Failed to write plist file")?;

    let output = Command::new("launchctl")
        .args(["load", plist.to_str().unwrap()])
        .output()
        .context("Failed to run launchctl load")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("launchctl load failed: {}", stderr);
    }

    println!("Service installed and started.");
    println!("  Plist: {}", plist.display());
    println!("  WebUI: http://{}", bind);
    Ok(())
}

async fn uninstall() -> Result<()> {
    let plist = plist_path()?;

    if !plist.exists() {
        bail!(
            "Service is not installed (plist not found: {})",
            plist.display()
        );
    }

    let output = Command::new("launchctl")
        .args(["unload", plist.to_str().unwrap()])
        .output()
        .context("Failed to run launchctl unload")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("launchctl unload warning: {}", stderr);
    }

    std::fs::remove_file(&plist).context("Failed to remove plist file")?;
    println!("Service uninstalled.");
    Ok(())
}

async fn start() -> Result<()> {
    let output = Command::new("launchctl")
        .args(["start", SERVICE_LABEL])
        .output()
        .context("Failed to run launchctl start")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("launchctl start failed: {}", stderr);
    }

    println!("Service started.");
    Ok(())
}

async fn stop() -> Result<()> {
    let output = Command::new("launchctl")
        .args(["stop", SERVICE_LABEL])
        .output()
        .context("Failed to run launchctl stop")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("launchctl stop failed: {}", stderr);
    }

    println!("Service stopped.");
    Ok(())
}

async fn status() -> Result<()> {
    let output = Command::new("launchctl")
        .args(["list", SERVICE_LABEL])
        .output()
        .context("Failed to run launchctl list")?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("Service is installed and loaded:");
        println!("{}", stdout);
    } else {
        println!("Service is not running or not installed.");
    }

    Ok(())
}
