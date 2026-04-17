use std::process::Command;

use anyhow::{bail, Context, Result};

use super::{ServiceAction, SERVICE_LABEL};

const SERVICE_NAME: &str = "RabbitDiggerPro";
const DISPLAY_NAME: &str = "Rabbit Digger Pro";

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
    let binary_path = std::env::current_exe().context("Cannot determine binary path")?;
    let binary_path = binary_path
        .to_str()
        .context("Binary path is not valid UTF-8")?;

    let mut bin_path_arg = format!("\"{binary_path}\" service run --bind {bind}");
    if let Some(token) = access_token {
        bin_path_arg.push_str(&format!(" --access-token {token}"));
    }

    // Stop and delete existing service if present
    let _ = Command::new("sc.exe").args(["stop", SERVICE_NAME]).output();
    let _ = Command::new("sc.exe")
        .args(["delete", SERVICE_NAME])
        .output();

    let output = Command::new("sc.exe")
        .args([
            "create",
            SERVICE_NAME,
            &format!("binPath= {bin_path_arg}"),
            &format!("DisplayName= {DISPLAY_NAME}"),
            "start= auto",
        ])
        .output()
        .context("Failed to run sc.exe create")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("sc.exe create failed: {}", stderr);
    }

    // Set description
    let _ = Command::new("sc.exe")
        .args([
            "description",
            SERVICE_NAME,
            "Rabbit Digger Pro proxy service",
        ])
        .output();

    // Start the service
    let output = Command::new("sc.exe")
        .args(["start", SERVICE_NAME])
        .output()
        .context("Failed to start service")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("Service start warning: {}", stderr);
    }

    println!("Service installed and started.");
    println!("  Name: {SERVICE_NAME}");
    println!("  WebUI: http://{bind}");
    Ok(())
}

async fn uninstall() -> Result<()> {
    let _ = Command::new("sc.exe").args(["stop", SERVICE_NAME]).output();

    let output = Command::new("sc.exe")
        .args(["delete", SERVICE_NAME])
        .output()
        .context("Failed to run sc.exe delete")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("sc.exe delete failed: {}", stderr);
    }

    println!("Service uninstalled.");
    Ok(())
}

async fn start() -> Result<()> {
    let output = Command::new("sc.exe")
        .args(["start", SERVICE_NAME])
        .output()
        .context("Failed to run sc.exe start")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("sc.exe start failed: {}", stderr);
    }

    println!("Service started.");
    Ok(())
}

async fn stop() -> Result<()> {
    let output = Command::new("sc.exe")
        .args(["stop", SERVICE_NAME])
        .output()
        .context("Failed to run sc.exe stop")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("sc.exe stop failed: {}", stderr);
    }

    println!("Service stopped.");
    Ok(())
}

async fn status() -> Result<()> {
    let output = Command::new("sc.exe")
        .args(["query", SERVICE_NAME])
        .output()
        .context("Failed to run sc.exe query")?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("{}", stdout);
    } else {
        println!("Service is not installed.");
    }

    Ok(())
}
