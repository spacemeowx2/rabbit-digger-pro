use std::path::PathBuf;

const PROGRAM_DIR: &str = "rabbit_digger_pro";

/// Application data directory.
///
/// - `RDP_DATA_DIR`, when set, overrides the platform directory (portable
///   installs and tests).
/// - LaunchDaemon / root: `/Library/Application Support/rabbit_digger_pro`
/// - Linux root:          `/var/lib/rabbit_digger_pro`
/// - Normal user:         `dirs::data_local_dir()/rabbit_digger_pro`
pub fn data_dir() -> PathBuf {
    if let Some(dir) = override_dir("RDP_DATA_DIR") {
        return dir;
    }
    if is_root() {
        return system_data_dir();
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(PROGRAM_DIR)
}

/// Application configuration directory. `RDP_CONFIG_DIR` overrides it.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = override_dir("RDP_CONFIG_DIR") {
        return dir;
    }
    if is_root() {
        return system_data_dir().join("config");
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(PROGRAM_DIR)
}

/// Application cache directory. `RDP_CACHE_DIR` overrides it.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = override_dir("RDP_CACHE_DIR") {
        return dir;
    }
    if is_root() {
        // Keep cache alongside data for system-level installs
        return system_data_dir().join("cache");
    }
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(PROGRAM_DIR)
}

fn override_dir(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn system_data_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support").join(PROGRAM_DIR)
    } else {
        PathBuf::from("/var/lib").join(PROGRAM_DIR)
    }
}

fn is_root() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: getuid() is always safe
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}
