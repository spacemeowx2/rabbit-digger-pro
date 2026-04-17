//! System route setup for TUN global mode.
//!
//! All side effects are registered with the SideEffectManager so they
//! get rolled back on normal shutdown (Drop) or crash recovery.

use std::process::Command;

use rd_interface::side_effect::{SideEffectManager, SideEffectUndo};
use tracing::debug;

const TUN_TABLE: &str = "2468";

pub struct RouteSetupConfig {
    pub tun_name: String,
    pub tun_gateway: String,
    pub fwmark: u32,
    pub dns_ip: String,
}

/// Set up system routes for TUN global mode.
/// All side effects are registered in the manager for automatic rollback.
pub fn setup_routes(mgr: &mut SideEffectManager, config: &RouteSetupConfig) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    setup_linux(mgr, config)?;
    #[cfg(target_os = "macos")]
    setup_macos(mgr, config)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cmd_undo(cmd: &str, args: &[&str]) -> SideEffectUndo {
    SideEffectUndo::Command {
        cmd: cmd.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
    }
}

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

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn setup_linux(mgr: &mut SideEffectManager, config: &RouteSetupConfig) -> Result<(), String> {
    let mark_str = config.fwmark.to_string();

    // 1. TUN default route in dedicated table
    mgr.apply(
        format!(
            "ip route add default dev {} table {TUN_TABLE}",
            config.tun_name
        ),
        || {
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
            )
        },
        cmd_undo(
            "ip",
            &[
                "route",
                "del",
                "default",
                "dev",
                &config.tun_name,
                "table",
                TUN_TABLE,
            ],
        ),
    )?;

    // 2. suppress_prefixlength 0 (priority 100) — lets response packets
    //    match specific routes in main table (e.g. eth0 LAN subnet)
    mgr.apply(
        "ip rule add table main suppress_prefixlength 0 priority 100",
        || {
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
                    "100",
                ],
            )
        },
        cmd_undo("ip", &["rule", "del", "priority", "100"]),
    )?;

    // 3. not fwmark → TUN table (priority 200) — unmarked outbound goes through TUN
    mgr.apply(
        format!("ip rule add not fwmark {mark_str} table {TUN_TABLE} priority 200"),
        || {
            run(
                "ip",
                &[
                    "rule", "add", "not", "fwmark", &mark_str, "table", TUN_TABLE, "priority",
                    "200",
                ],
            )
        },
        cmd_undo("ip", &["rule", "del", "priority", "200"]),
    )?;

    // 4. DNS hijack (priority 201) — port 53 always goes through TUN
    let _ = mgr.apply(
        format!("ip rule add dport 53 table {TUN_TABLE} priority 201"),
        || {
            run(
                "ip",
                &[
                    "rule", "add", "dport", "53", "table", TUN_TABLE, "priority", "201",
                ],
            )
        },
        cmd_undo("ip", &["rule", "del", "priority", "201"]),
    );

    // 5. Set DNS
    let original_dns = std::fs::read_to_string("/etc/resolv.conf").ok();
    let content = format!("nameserver {}\n", config.dns_ip);
    let _ = mgr.apply(
        format!("Set DNS to {}", config.dns_ip),
        || {
            std::fs::write("/etc/resolv.conf", &content)
                .map_err(|e| format!("write resolv.conf: {e}"))
        },
        SideEffectUndo::WriteFile {
            path: "/etc/resolv.conf".to_string(),
            content: original_dns.unwrap_or_else(|| "nameserver 1.1.1.1\n".to_string()),
        },
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn setup_macos(mgr: &mut SideEffectManager, config: &RouteSetupConfig) -> Result<(), String> {
    let (gw, iface) = get_default_route_macos()?;
    debug!("Original gateway: {gw} via {iface}");

    let service = get_network_service_macos(&iface).unwrap_or_else(|| "Wi-Fi".to_string());
    debug!("Network service: {service}");

    let original_dns = get_dns_macos(&service);

    // 1. Delete existing default route (undo: restore it)
    mgr.apply(
        "route delete default",
        || {
            let _ = run("route", &["delete", "default"]);
            Ok(()) // always succeed — might not exist
        },
        cmd_undo("route", &["add", "default", &gw]),
    )?;

    // 2. Add default route through TUN (undo: delete it)
    mgr.apply(
        format!(
            "route add default {} -interface {}",
            config.tun_gateway, config.tun_name
        ),
        || {
            run(
                "route",
                &[
                    "add",
                    "default",
                    &config.tun_gateway,
                    "-interface",
                    &config.tun_name,
                ],
            )
        },
        cmd_undo("route", &["delete", "default"]),
    )?;

    // 3. Scoped route to original gateway (undo: delete it)
    let _ = mgr.apply(
        format!("route add default {gw} -ifscope {iface}"),
        || run("route", &["add", "default", &gw, "-ifscope", &iface]),
        cmd_undo("route", &["delete", "default", "-ifscope", &iface]),
    );

    // 4. Set DNS (undo: restore original)
    let _ = mgr.apply(
        format!("networksetup -setdnsservers {service} {}", config.dns_ip),
        || {
            run(
                "networksetup",
                &["-setdnsservers", &service, &config.dns_ip],
            )
        },
        if original_dns.is_empty() {
            cmd_undo("networksetup", &["-setdnsservers", &service, "Empty"])
        } else {
            let mut args = vec!["-setdnsservers", &service];
            // Can't borrow original_dns items in cmd_undo, build full args
            SideEffectUndo::Command {
                cmd: "networksetup".to_string(),
                args: std::iter::once("-setdnsservers".to_string())
                    .chain(std::iter::once(service.clone()))
                    .chain(original_dns.iter().cloned())
                    .collect(),
            }
        },
    );

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
