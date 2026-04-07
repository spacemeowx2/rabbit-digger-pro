//! System route setup for TUN global mode.
//!
//! Linux: policy routing with fwmark (clash-rs style)
//!   - TUN default route in a dedicated table
//!   - `ip rule add not fwmark <mark> table <tun_table>` → unmarked packets go through TUN
//!   - `ip rule add table main suppress_prefixlength 0` → marked packets use main table
//!
//! macOS: route commands
//!   - Replace default route with TUN gateway
//!   - Scoped route back to original gateway for marked traffic

use std::process::Command;

use tracing::{debug, info, warn};

const TUN_TABLE: &str = "2468";

pub struct RouteState {
    tun_name: String,
    fwmark: u32,
    #[allow(dead_code)]
    original_dns: Option<String>,
    platform: PlatformState,
}

enum PlatformState {
    #[cfg(target_os = "linux")]
    Linux {
        original_gateway: Option<String>,
        dns_modified: bool,
    },
    #[cfg(target_os = "macos")]
    Macos {
        original_gateway: String,
        original_interface: String,
        network_service: String,
        original_dns: Vec<String>,
    },
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Unsupported,
}

pub struct RouteSetupConfig {
    pub tun_name: String,
    pub tun_gateway: String,
    pub fwmark: u32,
    pub dns_ip: String,
}

impl RouteState {
    pub fn setup(config: RouteSetupConfig) -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        let platform = setup_linux(&config)?;
        #[cfg(target_os = "macos")]
        let platform = setup_macos(&config)?;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let platform = {
            let _ = &config;
            PlatformState::Unsupported
        };

        info!("Routes configured for TUN {}", config.tun_name);

        Ok(RouteState {
            tun_name: config.tun_name,
            fwmark: config.fwmark,
            original_dns: None,
            platform,
        })
    }
}

impl Drop for RouteState {
    fn drop(&mut self) {
        if let Err(e) = self.teardown() {
            warn!("Route cleanup failed: {e}");
        }
    }
}

impl RouteState {
    fn teardown(&mut self) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        teardown_linux(&self.tun_name, self.fwmark, &mut self.platform)?;
        #[cfg(target_os = "macos")]
        teardown_macos(&mut self.platform)?;

        info!("Routes cleaned up");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn setup_linux(config: &RouteSetupConfig) -> Result<PlatformState, String> {
    // Save original default gateway
    let original_gateway = get_default_gateway_linux();
    debug!("Original gateway: {:?}", original_gateway);

    // 1. Add default route to TUN in dedicated table
    run(
        "ip",
        &[
            "route",
            "add",
            "default",
            "dev",
            &config.tun_name,
            "table",
            TUN_TABLE,
        ],
    )?;

    // 2. Rule: packets WITHOUT fwmark → use TUN table (highest priority)
    let mark_str = config.fwmark.to_string();
    run(
        "ip",
        &[
            "rule", "add", "not", "fwmark", &mark_str, "table", TUN_TABLE, "priority", "100",
        ],
    )?;

    // 3. DNS hijack: port 53 always goes through TUN table
    let _ = run(
        "ip",
        &[
            "rule", "add", "dport", "53", "table", TUN_TABLE, "priority", "101",
        ],
    );

    // 4. Suppress: allow local/direct routes in main table (lower priority than fwmark rule)
    run(
        "ip",
        &[
            "rule",
            "add",
            "table",
            "main",
            "suppress_prefixlength",
            "0",
            "priority",
            "200",
        ],
    )?;

    // 5. Set DNS
    let dns_modified = set_dns_linux(&config.dns_ip);

    Ok(PlatformState::Linux {
        original_gateway,
        dns_modified,
    })
}

#[cfg(target_os = "linux")]
fn teardown_linux(tun_name: &str, fwmark: u32, platform: &mut PlatformState) -> Result<(), String> {
    let PlatformState::Linux {
        original_gateway: _,
        dns_modified,
    } = platform
    else {
        return Ok(());
    };

    // Remove rules by priority (ignore errors — might already be gone)
    let _ = run("ip", &["rule", "del", "priority", "200"]);
    let _ = run("ip", &["rule", "del", "priority", "101"]);
    let _ = run("ip", &["rule", "del", "priority", "100"]);

    // Remove TUN route from dedicated table
    let _ = run(
        "ip",
        &[
            "route", "del", "default", "dev", tun_name, "table", TUN_TABLE,
        ],
    );

    // Restore DNS
    if *dns_modified {
        restore_dns_linux();
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn get_default_gateway_linux() -> Option<String> {
    let output = Command::new("ip")
        .args(["route", "show", "default"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // "default via 192.168.1.1 dev eth0 ..."
    stdout.split_whitespace().nth(2).map(String::from)
}

#[cfg(target_os = "linux")]
fn set_dns_linux(dns_ip: &str) -> bool {
    let content = format!("nameserver {dns_ip}\n");
    match std::fs::write("/etc/resolv.conf", &content) {
        Ok(_) => {
            debug!("Set DNS to {dns_ip}");
            true
        }
        Err(e) => {
            warn!("Failed to set DNS: {e}");
            false
        }
    }
}

#[cfg(target_os = "linux")]
fn restore_dns_linux() {
    // Best effort: set to a reasonable default
    let _ = std::fs::write("/etc/resolv.conf", "nameserver 1.1.1.1\n");
    debug!("Restored DNS to default");
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn setup_macos(config: &RouteSetupConfig) -> Result<PlatformState, String> {
    // Get original default gateway and interface
    let (gw, iface) = get_default_route_macos()?;
    debug!("Original gateway: {gw} via {iface}");

    // Get network service name for DNS changes
    let service = get_network_service_macos(&iface).unwrap_or_else(|| "Wi-Fi".to_string());
    debug!("Network service: {service}");

    // Save original DNS
    let original_dns = get_dns_macos(&service);

    // 1. Delete existing default route
    let _ = run("route", &["delete", "default"]);

    // 2. Add default route through TUN
    run(
        "route",
        &[
            "add",
            "default",
            &config.tun_gateway,
            "-interface",
            &config.tun_name,
        ],
    )?;

    // 3. Add scoped route to original gateway (so marked/bound traffic can still exit)
    let _ = run("route", &["add", "default", &gw, "-ifscope", &iface]);

    // 4. Set DNS
    let mut dns_args = vec!["-setdnsservers", &service, &config.dns_ip];
    let _ = run("networksetup", &dns_args);

    Ok(PlatformState::Macos {
        original_gateway: gw,
        original_interface: iface,
        network_service: service,
        original_dns,
    })
}

#[cfg(target_os = "macos")]
fn teardown_macos(platform: &mut PlatformState) -> Result<(), String> {
    let PlatformState::Macos {
        original_gateway,
        original_interface,
        network_service,
        original_dns,
    } = platform
    else {
        return Ok(());
    };

    // Restore default route
    let _ = run("route", &["delete", "default"]);
    let _ = run("route", &["add", "default", original_gateway]);

    // Remove scoped route
    let _ = run(
        "route",
        &["delete", "default", "-ifscope", original_interface],
    );

    // Restore DNS
    if original_dns.is_empty() {
        let _ = run(
            "networksetup",
            &["-setdnsservers", network_service, "Empty"],
        );
    } else {
        let mut args = vec!["-setdnsservers".to_string(), network_service.clone()];
        args.extend(original_dns.iter().cloned());
        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let _ = run("networksetup", &args_ref);
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn get_default_route_macos() -> Result<(String, String), String> {
    let output = Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .map_err(|e| format!("route -n get default: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let gateway = stdout
        .lines()
        .find(|l| l.contains("gateway:"))
        .and_then(|l| l.split_whitespace().last())
        .map(String::from)
        .ok_or("No default gateway found")?;

    let interface = stdout
        .lines()
        .find(|l| l.contains("interface:"))
        .and_then(|l| l.split_whitespace().last())
        .map(String::from)
        .ok_or("No default interface found")?;

    Ok((gateway, interface))
}

#[cfg(target_os = "macos")]
fn get_network_service_macos(interface: &str) -> Option<String> {
    let output = Command::new("networksetup")
        .args(["-listnetworkserviceorder"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut last_service = None;
    for line in stdout.lines() {
        if let Some(name) = line
            .strip_prefix('(')
            .and_then(|l| l.split(')').nth(1).map(|s| s.trim().to_string()))
        {
            if !name.is_empty() {
                last_service = Some(name);
            }
        }
        if line.contains(&format!("Device: {interface}")) {
            return last_service;
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn get_dns_macos(service: &str) -> Vec<String> {
    let output = Command::new("networksetup")
        .args(["-getdnsservers", service])
        .output()
        .ok();
    match output {
        Some(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if stdout.contains("aren't any") {
                vec![]
            } else {
                stdout.lines().map(String::from).collect()
            }
        }
        None => vec![],
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    debug!("Running: {cmd} {}", args.join(" "));
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{cmd} {}: {}", args.join(" "), stderr.trim()));
    }
    Ok(())
}
