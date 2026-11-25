//! Windows privileged engine service.
//!
//! TUN needs administrator rights, so the GUI installs a copy of itself as a
//! LocalSystem service once. The copy runs the full engine and exposes the
//! normal localhost RPC endpoint; the GUI then connects to that endpoint with a
//! per-user token instead of starting an embedded engine. No elevation is
//! needed after installation.

use std::{
    ffi::{OsStr, OsString},
    fs,
    io::Read,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

use crate::store::ServiceVersion;

pub const SERVICE_NAME: &str = "RabbitDiggerPro";
pub const SERVICE_ENDPOINT: &str = "127.0.0.1:19091";

const SERVICE_DISPLAY_NAME: &str = "Rabbit Digger Pro";
const SERVICE_DESCRIPTION: &str = "Rabbit Digger Pro privileged engine for TUN mode";
const SERVICE_BINARY_NAME: &str = "rdp-service.exe";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// What the GUI reads to decide whether to use the service backend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub endpoint: String,
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct UserMarker {
    endpoint: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ServiceFile {
    bind: String,
    data_dir: PathBuf,
    token_file: PathBuf,
}

fn user_data_dir() -> PathBuf {
    rabbit_digger_pro::util::app_dirs::data_dir()
}

fn user_marker_path() -> PathBuf {
    user_data_dir().join("service.json")
}

fn user_token_path() -> PathBuf {
    user_data_dir().join("service.token")
}

fn program_data_dir() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("RabbitDiggerPro")
}

fn service_file_path() -> PathBuf {
    program_data_dir().join("service.json")
}

fn install_dir() -> Result<PathBuf> {
    let base = std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .ok_or_else(|| anyhow!("ProgramFiles is not set"))?;
    Ok(PathBuf::from(base).join("RabbitDiggerPro"))
}

/// Reads the user-side marker and token. Its presence means the GUI should use
/// the service backend instead of starting an embedded engine.
pub fn read_user_config() -> Option<ServiceConfig> {
    let marker: UserMarker =
        serde_json::from_str(&fs::read_to_string(user_marker_path()).ok()?).ok()?;
    let token = fs::read_to_string(user_token_path())
        .ok()?
        .trim()
        .to_string();
    if token.is_empty() {
        return None;
    }
    Some(ServiceConfig {
        endpoint: marker.endpoint,
        token,
    })
}

pub fn is_configured() -> bool {
    read_user_config().is_some()
}

/// Compares this executable with the copy installed as the service. A byte
/// difference means the GUI was updated without updating the service (or vice
/// versa); TUN is blocked until the user reinstalls it.
pub fn version_status() -> ServiceVersion {
    if !is_configured() {
        return ServiceVersion::NotInstalled;
    }
    let Ok(current) = std::env::current_exe() else {
        return ServiceVersion::Outdated;
    };
    let Ok(dir) = install_dir() else {
        return ServiceVersion::Outdated;
    };
    let installed = dir.join(SERVICE_BINARY_NAME);
    if !installed.exists() {
        return ServiceVersion::Outdated;
    }
    if same_file(&current, &installed) {
        return ServiceVersion::Current;
    }
    match (file_hash(&current), file_hash(&installed)) {
        (Ok(current), Ok(installed)) if current == installed => ServiceVersion::Current,
        _ => ServiceVersion::Outdated,
    }
}

fn file_hash(path: &Path) -> std::io::Result<[u8; 32]> {
    use sha2::{Digest, Sha256};

    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut result = [0u8; 32];
    result.copy_from_slice(&digest);
    Ok(result)
}

/// Runs the elevated installer. Called from `--install-service`; the parent GUI
/// passes its own data directory so the service uses the same settings and
/// subscription storage.
pub fn install_service(data_dir: PathBuf) -> Result<()> {
    let log_path = data_dir.join("service-install.log");
    let mut log = String::new();
    let result = install_inner(&data_dir, &mut log);
    if let Err(error) = &result {
        log.push_str(&format!("\nERROR: {error:#}\n"));
        // Never leave a marker that points the GUI at a service that failed to
        // start.
        let _ = fs::remove_file(data_dir.join("service.json"));
        let _ = fs::remove_file(data_dir.join("service.token"));
        let _ = fs::remove_file(service_file_path());
        let _ = remove_existing_service(&mut String::new());
    }
    let _ = fs::create_dir_all(&data_dir);
    let _ = fs::write(&log_path, log);
    result
}

fn install_inner(data_dir: &Path, log: &mut String) -> Result<()> {
    let current = std::env::current_exe().context("current executable path")?;
    log.push_str(&format!("source: {}\n", current.display()));

    // Stop the old copy before overwriting the service binary.
    remove_existing_service(log)?;

    let target_dir = install_dir()?;
    fs::create_dir_all(&target_dir).with_context(|| format!("create {}", target_dir.display()))?;
    let target = target_dir.join(SERVICE_BINARY_NAME);
    if !same_file(&current, &target) {
        fs::copy(&current, &target)
            .with_context(|| format!("copy service binary to {}", target.display()))?;
        log.push_str(&format!("copied to: {}\n", target.display()));
    }

    let wintun_source = current
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("wintun.dll");
    if !wintun_source.exists() {
        bail!(
            "wintun.dll is missing next to {}. Put it next to rdp-gui.exe and retry.",
            current.display()
        );
    }
    let wintun_target = target_dir.join("wintun.dll");
    fs::copy(&wintun_source, &wintun_target)
        .with_context(|| format!("copy wintun.dll to {}", wintun_target.display()))?;

    fs::create_dir_all(data_dir).with_context(|| format!("create {}", data_dir.display()))?;
    let token_path = data_dir.join("service.token");
    let marker_path = data_dir.join("service.json");
    let token = fs::read_to_string(&token_path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    fs::write(&token_path, &token).context("write service token")?;

    let program_data = program_data_dir();
    fs::create_dir_all(&program_data)
        .with_context(|| format!("create {}", program_data.display()))?;
    let service_file = ServiceFile {
        bind: SERVICE_ENDPOINT.to_string(),
        data_dir: data_dir.to_path_buf(),
        token_file: token_path,
    };
    fs::write(
        service_file_path(),
        serde_json::to_vec_pretty(&service_file)?,
    )
    .context("write service config")?;

    fs::write(
        marker_path,
        serde_json::to_vec_pretty(&UserMarker {
            endpoint: SERVICE_ENDPOINT.to_string(),
        })?,
    )
    .context("write user service marker")?;

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("open service manager (run as administrator)")?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: target,
        launch_arguments: vec![OsString::from("--service-run")],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let service = manager
        .create_service(
            &info,
            ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS,
        )
        .context("create RabbitDiggerPro service")?;
    let _ = service.set_description(SERVICE_DESCRIPTION);
    let _ = Command::new("sc.exe")
        .args([
            "failure",
            SERVICE_NAME,
            "reset=",
            "86400",
            "actions=",
            "restart/5000/restart/10000/restart/30000",
        ])
        .output();
    service.start(&[] as &[&OsStr]).context("start service")?;

    let addr: std::net::SocketAddr = SERVICE_ENDPOINT.parse().context("parse service endpoint")?;
    for _ in 0..50 {
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            log.push_str("service API is running\n");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    bail!("service did not start its API within 10 seconds")
}

fn remove_existing_service(log: &mut String) -> Result<()> {
    let manager = match ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
    {
        Ok(manager) => manager,
        Err(error) => {
            log.push_str(&format!("cannot open service manager: {error}\n"));
            return Ok(());
        }
    };
    let Ok(service) = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    ) else {
        return Ok(());
    };

    if let Ok(status) = service.query_status() {
        if status.current_state != ServiceState::Stopped {
            let _ = service.stop();
            for _ in 0..50 {
                match service.query_status() {
                    Ok(status) if status.current_state == ServiceState::Stopped => break,
                    _ => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        }
    }
    let _ = service.delete();
    drop(service);
    std::thread::sleep(Duration::from_millis(500));
    log.push_str("removed previous service\n");
    Ok(())
}

fn same_file(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

pub fn uninstall_service(data_dir: PathBuf) -> Result<()> {
    let mut log = String::new();
    let result = uninstall_inner(&data_dir, &mut log);
    let _ = fs::write(data_dir.join("service-install.log"), log);
    result
}

fn uninstall_inner(data_dir: &Path, log: &mut String) -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("open service manager (run as administrator)")?;
    if let Ok(service) = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    ) {
        let _ = service.stop();
        for _ in 0..50 {
            match service.query_status() {
                Ok(status) if status.current_state == ServiceState::Stopped => break,
                _ => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        let _ = service.delete();
        log.push_str("service stopped and deleted\n");
    }

    let _ = fs::remove_file(data_dir.join("service.json"));
    let _ = fs::remove_file(data_dir.join("service.token"));
    let _ = fs::remove_file(service_file_path());
    if let Ok(dir) = install_dir() {
        // The service process may still be closing its executable. Retry for a
        // few seconds so a normal uninstall also removes the Program Files copy.
        for _ in 0..10 {
            if fs::remove_dir_all(&dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    Ok(())
}

/// Runs the SCM dispatcher. Only valid when launched by the Service Control
/// Manager; the user-facing GUI never calls this directly.
pub fn run_service() -> Result<()> {
    let data = fs::read(service_file_path()).context("read service config")?;
    let config: ServiceFile = serde_json::from_slice(&data).context("parse service config")?;
    std::env::set_var("RDP_DATA_DIR", &config.data_dir);
    std::env::set_var("RDP_CONFIG_DIR", config.data_dir.join("config"));
    std::env::set_var("RDP_CACHE_DIR", config.data_dir.join("cache"));
    let _ = SERVICE_FILE.set(config);
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .context("start service dispatcher")?;
    Ok(())
}

static SERVICE_FILE: OnceLock<ServiceFile> = OnceLock::new();
static STOP: Notify = Notify::const_new();

define_windows_service!(ffi_service_main, service_main);

fn service_main(_args: Vec<OsString>) {
    let _ = service_main_inner();
}

fn service_main_inner() -> Result<()> {
    let handler = move |event| match event {
        ServiceControl::Stop => {
            STOP.notify_one();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status_handle = service_control_handler::register(SERVICE_NAME, handler)
        .context("register service control handler")?;
    status_handle
        .set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
        .context("report Running")?;

    let config = SERVICE_FILE.get().context("service config missing")?;
    let token = fs::read_to_string(&config.token_file)
        .context("read service token")?
        .trim()
        .to_string();
    if token.is_empty() {
        bail!("service token is empty");
    }

    crate::engine::init_logging_to_file(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(config.data_dir.join("daemon.log"))
            .ok(),
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("create service runtime")?;
    let result = runtime.block_on(async {
        rabbit_digger_pro::service::run_daemon_with_shutdown(
            config.bind.clone(),
            Some(token),
            async {
                STOP.notified().await;
            },
        )
        .await
    });

    let exit_code = if result.is_ok() {
        ServiceExitCode::Win32(0)
    } else {
        ServiceExitCode::ServiceSpecific(1)
    };
    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code,
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });
    result
}

/// Records a service startup/install failure somewhere a user can find it even
/// though the release build has no console.
pub fn write_error(error: &anyhow::Error) {
    let path = program_data_dir().join("service-error.log");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, format!("{error:#}\n"));
}

/// Asks the current executable to relaunch with the same arguments. Used after
/// install/uninstall so the GUI picks the new backend.
pub fn restart_app() -> Result<()> {
    let exe = std::env::current_exe().context("current executable path")?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    Command::new(exe)
        .args(args)
        .spawn()
        .context("relaunch rdp-gui")?;
    Ok(())
}

/// Elevates the current executable to install the service. One UAC prompt.
pub fn install_from_gui() -> Result<()> {
    run_elevated("--install-service")
}

/// Elevates the current executable to uninstall the service.
pub fn uninstall_from_gui() -> Result<()> {
    run_elevated("--uninstall-service")
}

fn run_elevated(action: &str) -> Result<()> {
    let exe = std::env::current_exe().context("current executable path")?;
    let data_dir = user_data_dir();
    let script = format!(
        "$p = Start-Process -FilePath '{}' -ArgumentList '{} --service-data-dir \"{}\"' -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
        ps_escape(&exe.to_string_lossy()),
        action,
        ps_escape(&data_dir.to_string_lossy()),
    );
    let encoded = encode_powershell(&script);
    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            &encoded,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .context("launch elevated installer")?;
    if !status.success() {
        let log =
            fs::read_to_string(user_data_dir().join("service-install.log")).unwrap_or_default();
        bail!("elevated installer failed with {status}\n{log}");
    }
    Ok(())
}

fn ps_escape(value: &str) -> String {
    value.replace('\'', "''")
}

fn encode_powershell(script: &str) -> String {
    use base64::Engine as _;
    let utf16: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(utf16)
}
