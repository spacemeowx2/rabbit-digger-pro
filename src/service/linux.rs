use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use super::ServiceAction;

const SERVICE_NAME: &str = "rabbit-digger-pro";
const SYSTEMD_INSTALL_SUBDIR: &str = "bin";
const PROCD_INSTALL_DIR: &str = "/usr/local/bin";

#[derive(Debug, Clone, Copy)]
enum InitSystem {
    Systemd,
    Procd,
}

impl InitSystem {
    fn installed_binary(self) -> PathBuf {
        match self {
            InitSystem::Systemd => crate::util::app_dirs::data_dir()
                .join(SYSTEMD_INSTALL_SUBDIR)
                .join(SERVICE_NAME),
            InitSystem::Procd => PathBuf::from(PROCD_INSTALL_DIR).join(SERVICE_NAME),
        }
    }
}

// ---------------------------------------------------------------------------
// Init system detection
// ---------------------------------------------------------------------------

fn unit_name() -> String {
    format!("{SERVICE_NAME}.service")
}

fn unit_path() -> String {
    format!("/etc/systemd/system/{}", unit_name())
}

fn procd_script_path() -> String {
    format!("/etc/init.d/{SERVICE_NAME}")
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
        ServiceAction::Uninstall => uninstall().await,
        ServiceAction::Start => start().await,
        ServiceAction::Stop => stop().await,
        ServiceAction::Status => status().await,
        ServiceAction::Run { .. } => unreachable!("Run is handled by daemon module"),
    }
}

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

async fn install(bind: &str, access_token: Option<&str>) -> Result<()> {
    super::ensure_root()?;
    let init = detect_init()?;

    let current_exe = std::env::current_exe().context("Cannot determine binary path")?;
    let dest = init.installed_binary();
    install_binary(&current_exe, &dest)?;

    match init {
        InitSystem::Systemd => install_systemd(&dest, bind, access_token)?,
        InitSystem::Procd => install_procd(&dest, bind, access_token)?,
    }

    println!("Service installed.");
    println!("  Name:   {SERVICE_NAME}");
    println!("  Binary: {}", dest.display());
    println!("  WebUI:  http://{bind}");
    Ok(())
}

fn install_binary(current_exe: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if current_exe.canonicalize().ok() != dest.canonicalize().ok() {
        std::fs::copy(current_exe, dest)
            .with_context(|| format!("Failed to copy binary to {}", dest.display()))?;
    }
    let dest_str = dest.to_str().context("Invalid binary path")?;
    run("chmod", &["755", dest_str])?;
    Ok(())
}

fn install_systemd(binary: &Path, bind: &str, access_token: Option<&str>) -> Result<()> {
    let binary = binary.to_str().context("Invalid binary path")?;

    let exec_start = systemd_command_line(binary, &daemon_exec_args(bind, access_token));

    let unit = format!(
        r#"[Unit]
Description=Rabbit Digger Pro
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=RUST_LOG=info
ExecStart={exec_start}
Restart=on-failure
RestartSec=5
KillSignal=SIGTERM
TimeoutStopSec=20

[Install]
WantedBy=multi-user.target
"#
    );

    let unit_path = unit_path();
    std::fs::write(&unit_path, &unit).context("Failed to write systemd unit")?;

    run("systemctl", &["daemon-reload"])?;
    run("systemctl", &["enable", "--now", &unit_name()])?;

    println!("  Unit:   {unit_path}");
    Ok(())
}

fn install_procd(binary: &Path, bind: &str, access_token: Option<&str>) -> Result<()> {
    let binary = binary.to_str().context("Invalid binary path")?;
    let binary = shell_quote_arg(binary);

    let args = shell_command_args(&daemon_exec_args(bind, access_token));

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

    let script_path = procd_script_path();
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
            let unit_name = unit_name();
            let _ = run("systemctl", &["stop", &unit_name]);
            let _ = run("systemctl", &["disable", &unit_name]);
            let unit_path = unit_path();
            if Path::new(&unit_path).exists() {
                std::fs::remove_file(&unit_path)?;
            }
            let _ = run("systemctl", &["daemon-reload"]);
            remove_binary_if_unused(InitSystem::Systemd)?;
        }
        InitSystem::Procd => {
            let script_path = procd_script_path();
            if Path::new(&script_path).exists() {
                let _ = run(&script_path, &["stop"]);
                let _ = run(&script_path, &["disable"]);
                std::fs::remove_file(&script_path)?;
            }
            remove_binary_if_unused(InitSystem::Procd)?;
        }
    }

    println!("Service uninstalled.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Start / Stop / Status
// ---------------------------------------------------------------------------

async fn start() -> Result<()> {
    match detect_init()? {
        InitSystem::Systemd => run("systemctl", &["start", &unit_name()])?,
        InitSystem::Procd => run(&procd_script_path(), &["start"])?,
    }
    println!("Service started.");
    Ok(())
}

async fn stop() -> Result<()> {
    match detect_init()? {
        InitSystem::Systemd => run("systemctl", &["stop", &unit_name()])?,
        InitSystem::Procd => run(&procd_script_path(), &["stop"])?,
    }
    println!("Service stopped.");
    Ok(())
}

async fn status() -> Result<()> {
    match detect_init()? {
        InitSystem::Systemd => {
            let output = Command::new("systemctl")
                .args(["status", &unit_name()])
                .output()
                .context("Failed to run systemctl status")?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            println!("{}", stdout);
        }
        InitSystem::Procd => {
            let script_path = procd_script_path();
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

fn daemon_exec_args(bind: &str, access_token: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "service".to_string(),
        "run".to_string(),
        "--bind".to_string(),
        bind.to_string(),
    ];
    if let Some(token) = access_token {
        args.push("--access-token".to_string());
        args.push(token.to_string());
    }
    args
}

fn systemd_command_line(binary: &str, args: &[String]) -> String {
    std::iter::once(binary.to_string())
        .chain(args.iter().cloned())
        .map(|arg| systemd_quote_arg(&arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn systemd_quote_arg(arg: &str) -> String {
    format!(
        "\"{}\"",
        arg.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    )
}

fn shell_command_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| shell_quote_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote_arg(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

fn remove_binary_if_unused(init: InitSystem) -> Result<()> {
    let service_exists = match init {
        InitSystem::Systemd => Path::new(&unit_path()).exists(),
        InitSystem::Procd => Path::new(&procd_script_path()).exists(),
    };
    if service_exists {
        return Ok(());
    }

    let dest = init.installed_binary();
    if dest.exists() {
        std::fs::remove_file(&dest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_exec_args_uses_service_run() {
        assert_eq!(
            daemon_exec_args("127.0.0.1:9091", Some("token")),
            vec![
                "service".to_string(),
                "run".to_string(),
                "--bind".to_string(),
                "127.0.0.1:9091".to_string(),
                "--access-token".to_string(),
                "token".to_string(),
            ]
        );
    }

    #[test]
    fn systemd_command_line_quotes_paths_and_args() {
        assert_eq!(
            systemd_command_line(
                "/var/lib/rabbit_digger_pro/bin/rabbit-digger-pro",
                &daemon_exec_args("127.0.0.1:9091", Some("token with $space"))
            ),
            "\"/var/lib/rabbit_digger_pro/bin/rabbit-digger-pro\" \"service\" \"run\" \"--bind\" \"127.0.0.1:9091\" \"--access-token\" \"token with $$space\""
        );
    }

    #[test]
    fn systemd_command_line_escapes_unit_specifier_percent() {
        assert_eq!(
            systemd_command_line(
                "/var/lib/rabbit_digger_pro/bin/rabbit-digger-pro",
                &daemon_exec_args("127.0.0.1:90%91", None)
            ),
            "\"/var/lib/rabbit_digger_pro/bin/rabbit-digger-pro\" \"service\" \"run\" \"--bind\" \"127.0.0.1:90%%91\""
        );
    }

    #[test]
    fn procd_binary_path_stays_persistent() {
        assert_eq!(
            InitSystem::Procd.installed_binary(),
            PathBuf::from("/usr/local/bin/rabbit-digger-pro")
        );
    }
}
