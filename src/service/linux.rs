use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use super::ServiceAction;

const SERVICE_NAME: &str = "rabbit-digger-pro";
const INSTALL_DIR: &str = "/usr/local/bin";
const USER_UNIT_NAME: &str = "rabbit-digger-pro.service";

fn installed_binary() -> PathBuf {
    PathBuf::from(INSTALL_DIR).join(SERVICE_NAME)
}

fn user_helper_binary() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".local/share")
        });
    base.join("rabbit_digger_pro")
        .join("helper")
        .join(SERVICE_NAME)
}

fn user_unit_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".config")
        });
    base.join("systemd").join("user").join(USER_UNIT_NAME)
}

fn user_env_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".config")
        });
    base.join("rabbit_digger_pro").join("helper.env")
}

// ---------------------------------------------------------------------------
// Init system detection
// ---------------------------------------------------------------------------

enum InitSystem {
    Systemd,
    Procd,
}

fn detect_init() -> Result<InitSystem> {
    // procd (OpenWrt): /etc/rc.common exists
    if Path::new("/etc/rc.common").exists() {
        return Ok(InitSystem::Procd);
    }
    // systemd: systemctl exists
    if Command::new("systemctl").arg("--version").output().is_ok() {
        return Ok(InitSystem::Systemd);
    }
    bail!("No supported init system found (need systemd or procd/OpenWrt)");
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub async fn handle_service_action(action: ServiceAction) -> Result<()> {
    match action {
        ServiceAction::Install { bind, access_token } => {
            install(&bind, access_token.as_deref()).await
        }
        ServiceAction::InstallUser {
            bind,
            access_token,
            binary,
        } => install_user(&bind, access_token.as_deref(), binary.as_deref()).await,
        ServiceAction::Uninstall => uninstall().await,
        ServiceAction::UninstallUser => uninstall_user().await,
        ServiceAction::Start => start().await,
        ServiceAction::StartUser => start_user().await,
        ServiceAction::Stop => stop().await,
        ServiceAction::StopUser => stop_user().await,
        ServiceAction::Status => status().await,
        ServiceAction::StatusUser => status_user().await,
        ServiceAction::Run { .. } => unreachable!("Run is handled by daemon module"),
    }
}

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

async fn install(bind: &str, access_token: Option<&str>) -> Result<()> {
    super::ensure_root()?;

    // Copy binary
    let current_exe = std::env::current_exe().context("Cannot determine binary path")?;
    let dest = installed_binary();
    std::fs::create_dir_all(INSTALL_DIR)?;
    std::fs::copy(&current_exe, &dest)
        .with_context(|| format!("Failed to copy binary to {}", dest.display()))?;
    let _ = Command::new("chmod")
        .args(["755", dest.to_str().unwrap()])
        .output();

    match detect_init()? {
        InitSystem::Systemd => install_systemd(bind, access_token)?,
        InitSystem::Procd => install_procd(bind, access_token)?,
    }

    println!("Service installed.");
    println!("  Binary: {}", dest.display());
    println!("  WebUI:  http://{}", bind);
    Ok(())
}

fn install_systemd(bind: &str, access_token: Option<&str>) -> Result<()> {
    let binary = installed_binary();
    let binary = binary.to_str().context("Invalid binary path")?;

    let mut exec_start = format!("{binary} service run --bind {bind}");
    if let Some(token) = access_token {
        exec_start.push_str(&format!(" --access-token {token}"));
    }

    let unit = format!(
        r#"[Unit]
Description=Rabbit Digger Pro
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={exec_start}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
"#
    );

    let unit_path = format!("/etc/systemd/system/{SERVICE_NAME}.service");
    std::fs::write(&unit_path, &unit).context("Failed to write systemd unit")?;

    run("systemctl", &["daemon-reload"])?;
    run("systemctl", &["enable", SERVICE_NAME])?;
    run("systemctl", &["start", SERVICE_NAME])?;

    println!("  Unit:   {unit_path}");
    Ok(())
}

fn render_user_systemd_unit(binary: &Path, bind: &str, env_path: Option<&Path>) -> Result<String> {
    let binary = binary.to_str().context("Invalid binary path")?;
    let exec_start = format!("{binary} service run --bind {bind}");
    let environment_file = match env_path {
        Some(path) => {
            let path = path.to_str().context("Invalid environment file path")?;
            format!("EnvironmentFile={path}\n")
        }
        None => String::new(),
    };

    Ok(format!(
        r#"[Unit]
Description=Rabbit Digger Pro User Helper
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
{environment_file}ExecStart={exec_start}
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
"#
    ))
}

async fn install_user(bind: &str, access_token: Option<&str>, binary: Option<&Path>) -> Result<()> {
    let source = match binary {
        Some(path) => path.to_path_buf(),
        None => std::env::current_exe().context("Cannot determine binary path")?,
    };
    let dest = user_helper_binary();
    let dest_dir = dest
        .parent()
        .context("Cannot determine user helper directory")?;
    std::fs::create_dir_all(dest_dir)?;

    let tmp_dest = dest.with_extension("tmp");
    std::fs::copy(&source, &tmp_dest).with_context(|| {
        format!(
            "Failed to copy binary from {} to {}",
            source.display(),
            tmp_dest.display()
        )
    })?;
    let _ = Command::new("chmod")
        .args(["755", tmp_dest.to_str().unwrap()])
        .output();
    std::fs::rename(&tmp_dest, &dest)
        .with_context(|| format!("Failed to install binary to {}", dest.display()))?;

    let unit_path = user_unit_path();
    let unit_dir = unit_path
        .parent()
        .context("Cannot determine systemd user unit directory")?;
    std::fs::create_dir_all(unit_dir)?;
    let env_path = if let Some(token) = access_token {
        let env_path = user_env_path();
        let env_dir = env_path
            .parent()
            .context("Cannot determine service environment directory")?;
        std::fs::create_dir_all(env_dir)?;
        std::fs::write(&env_path, format!("RD_ACCESS_TOKEN={token}\n"))
            .context("Failed to write service environment file")?;
        let _ = Command::new("chmod")
            .args(["600", env_path.to_str().unwrap()])
            .output();
        Some(env_path)
    } else {
        None
    };
    let unit = render_user_systemd_unit(&dest, bind, env_path.as_deref())?;
    std::fs::write(&unit_path, unit).context("Failed to write systemd user unit")?;

    run("systemctl", &["--user", "daemon-reload"])?;
    run("systemctl", &["--user", "enable", "--now", USER_UNIT_NAME])?;

    println!("User service installed.");
    println!("  Binary: {}", dest.display());
    println!("  Unit:   {}", unit_path.display());
    println!("  WebUI:  http://{}", bind);
    Ok(())
}

fn install_procd(bind: &str, access_token: Option<&str>) -> Result<()> {
    let binary = installed_binary();
    let binary = binary.to_str().context("Invalid binary path")?;

    let mut args = format!("service run --bind {bind}");
    if let Some(token) = access_token {
        args.push_str(&format!(" --access-token {token}"));
    }

    let script = format!(
        r#"#!/bin/sh /etc/rc.common

USE_PROCD=1
START=95
STOP=01

start_service() {{
    procd_open_instance
    procd_set_param command {binary} {args}
    procd_set_param stdout 1
    procd_set_param stderr 1
    procd_set_param respawn
    procd_close_instance
}}
"#
    );

    let script_path = format!("/etc/init.d/{SERVICE_NAME}");
    std::fs::write(&script_path, &script).context("Failed to write init script")?;
    run("chmod", &["+x", &script_path])?;
    run(&script_path, &["enable"])?;
    run(&script_path, &["start"])?;

    println!("  Init:   {script_path}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Uninstall
// ---------------------------------------------------------------------------

async fn uninstall() -> Result<()> {
    super::ensure_root()?;

    match detect_init()? {
        InitSystem::Systemd => {
            let _ = run("systemctl", &["stop", SERVICE_NAME]);
            let _ = run("systemctl", &["disable", SERVICE_NAME]);
            let unit_path = format!("/etc/systemd/system/{SERVICE_NAME}.service");
            if Path::new(&unit_path).exists() {
                std::fs::remove_file(&unit_path)?;
            }
            let _ = run("systemctl", &["daemon-reload"]);
        }
        InitSystem::Procd => {
            let script_path = format!("/etc/init.d/{SERVICE_NAME}");
            if Path::new(&script_path).exists() {
                let _ = run(&script_path, &["stop"]);
                let _ = run(&script_path, &["disable"]);
                std::fs::remove_file(&script_path)?;
            }
        }
    }

    let dest = installed_binary();
    if dest.exists() {
        std::fs::remove_file(&dest)?;
    }

    println!("Service uninstalled.");
    Ok(())
}

async fn uninstall_user() -> Result<()> {
    let _ = run("systemctl", &["--user", "stop", USER_UNIT_NAME]);
    let _ = run("systemctl", &["--user", "disable", USER_UNIT_NAME]);

    let unit_path = user_unit_path();
    if unit_path.exists() {
        std::fs::remove_file(&unit_path)?;
    }
    let env_path = user_env_path();
    if env_path.exists() {
        std::fs::remove_file(&env_path)?;
    }
    let _ = run("systemctl", &["--user", "daemon-reload"]);

    let dest = user_helper_binary();
    if dest.exists() {
        std::fs::remove_file(&dest)?;
    }

    println!("User service uninstalled.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Start / Stop / Status
// ---------------------------------------------------------------------------

async fn start() -> Result<()> {
    match detect_init()? {
        InitSystem::Systemd => run("systemctl", &["start", SERVICE_NAME])?,
        InitSystem::Procd => run(&format!("/etc/init.d/{SERVICE_NAME}"), &["start"])?,
    }
    println!("Service started.");
    Ok(())
}

async fn start_user() -> Result<()> {
    run("systemctl", &["--user", "start", USER_UNIT_NAME])?;
    println!("User service started.");
    Ok(())
}

async fn stop() -> Result<()> {
    match detect_init()? {
        InitSystem::Systemd => run("systemctl", &["stop", SERVICE_NAME])?,
        InitSystem::Procd => run(&format!("/etc/init.d/{SERVICE_NAME}"), &["stop"])?,
    }
    println!("Service stopped.");
    Ok(())
}

async fn stop_user() -> Result<()> {
    run("systemctl", &["--user", "stop", USER_UNIT_NAME])?;
    println!("User service stopped.");
    Ok(())
}

async fn status() -> Result<()> {
    match detect_init()? {
        InitSystem::Systemd => {
            let output = Command::new("systemctl")
                .args(["status", SERVICE_NAME])
                .output()
                .context("Failed to run systemctl status")?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            println!("{}", stdout);
        }
        InitSystem::Procd => {
            let script_path = format!("/etc/init.d/{SERVICE_NAME}");
            if Path::new(&script_path).exists() {
                // procd doesn't have a nice status command; check if process is running
                let output = Command::new("pgrep")
                    .args(["-f", &format!("{SERVICE_NAME} service run")])
                    .output();
                match output {
                    Ok(o) if o.status.success() => {
                        let pids = String::from_utf8_lossy(&o.stdout);
                        println!("Service is running (PIDs: {})", pids.trim());
                    }
                    _ => println!("Service is not running."),
                }
            } else {
                println!("Service is not installed.");
            }
        }
    }
    Ok(())
}

async fn status_user() -> Result<()> {
    let output = Command::new("systemctl")
        .args(["--user", "status", USER_UNIT_NAME])
        .output()
        .context("Failed to run systemctl --user status")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        println!("{}", stdout);
    }
    if !stderr.is_empty() {
        eprintln!("{}", stderr);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("Failed to run {cmd}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{cmd} {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_user_systemd_unit_without_token() {
        let unit = render_user_systemd_unit(
            Path::new("/home/deck/.local/share/rabbit_digger_pro/helper/rabbit-digger-pro"),
            "127.0.0.1:9091",
            None,
        )
        .unwrap();

        assert!(unit.contains("WantedBy=default.target"));
        assert!(unit.contains("RestartSec=3"));
        assert!(unit.contains(
            "ExecStart=/home/deck/.local/share/rabbit_digger_pro/helper/rabbit-digger-pro service run --bind 127.0.0.1:9091"
        ));
        assert!(!unit.contains("EnvironmentFile="));
        assert!(!unit.contains("--access-token"));
    }

    #[test]
    fn render_user_systemd_unit_with_env_file() {
        let unit = render_user_systemd_unit(
            Path::new("/home/deck/.local/share/rabbit_digger_pro/helper/rabbit-digger-pro"),
            "127.0.0.1:9091",
            Some(Path::new("/home/deck/.config/rabbit_digger_pro/helper.env")),
        )
        .unwrap();

        assert!(unit.contains("EnvironmentFile=/home/deck/.config/rabbit_digger_pro/helper.env"));
        assert!(!unit.contains("test-token"));
        assert!(!unit.contains("--access-token"));
    }
}
