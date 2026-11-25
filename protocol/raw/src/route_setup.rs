//! System route setup for TUN global mode.
//!
//! All side effects are registered with the SideEffectManager so they
//! get rolled back on normal shutdown (Drop) or crash recovery.

use std::process::Command;

use rd_interface::side_effect::{SideEffectManager, SideEffectUndo};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use tracing::debug;

#[cfg(any(target_os = "linux", target_os = "macos"))]
const TUN_TABLE: &str = "2468";

pub struct RouteSetupConfig {
    pub tun_name: String,
    pub tun_gateway: String,
    pub fwmark: u32,
    pub dns_ip: String,
    /// Proxy server addresses that must stay on the physical route (Windows
    /// adds explicit /32 routes for them).
    pub bypass_ips: Vec<String>,
}

/// Set up system routes for TUN global mode.
/// All side effects are registered in the manager for automatic rollback.
pub fn setup_routes(mgr: &mut SideEffectManager, config: &RouteSetupConfig) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    setup_linux(mgr, config)?;
    #[cfg(target_os = "macos")]
    setup_macos(mgr, config)?;
    #[cfg(target_os = "windows")]
    setup_windows(mgr, config)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cmd_undo(cmd: &str, args: &[&str]) -> SideEffectUndo {
    SideEffectUndo::Command {
        cmd: cmd.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

    // 2. fwmark → main table (priority 50) — host-side sockets that are
    //    intentionally marked must bypass both the TUN default route and
    //    DNS hijack rules. Relying only on the later "not fwmark" rule is
    //    not enough for real TCP/UDP sockets with L4 selectors.
    mgr.apply(
        format!("ip rule add fwmark {mark_str} table main priority 50"),
        || {
            run(
                "ip",
                &[
                    "rule", "add", "fwmark", &mark_str, "table", "main", "priority", "50",
                ],
            )
        },
        cmd_undo("ip", &["rule", "del", "priority", "50"]),
    )?;

    // 3. suppress_prefixlength 0 (priority 100) — lets response packets
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

    // 4. not fwmark → TUN table (priority 200) — unmarked outbound goes through TUN
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

    // 5. DNS hijack (priority 201) — unmarked port 53 goes through TUN.
    //    Marked bootstrap DNS must bypass this rule or it recursively enters
    //    the TUN path and cannot resolve upstream proxy nodes.
    let _ = mgr.apply(
        format!("ip rule add not fwmark {mark_str} dport 53 table {TUN_TABLE} priority 201"),
        || {
            run(
                "ip",
                &[
                    "rule", "add", "not", "fwmark", &mark_str, "dport", "53", "table", TUN_TABLE,
                    "priority", "201",
                ],
            )
        },
        cmd_undo("ip", &["rule", "del", "priority", "201"]),
    );

    // 6. Set DNS
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

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn setup_windows(mgr: &mut SideEffectManager, config: &RouteSetupConfig) -> Result<(), String> {
    use std::net::{Ipv4Addr, ToSocketAddrs};

    if !is_elevated()? {
        return Err("TUN mode on Windows requires running as administrator".to_string());
    }

    let tun_index = get_tun_interface_index()?;
    let original_metric = get_interface_metric(tun_index)?;

    // Resolve bypass targets before the split default routes are installed, so
    // a hostname resolution cannot be captured by the TUN itself.
    let mut bypass_ips = Vec::new();
    for entry in &config.bypass_ips {
        if let Ok(ip) = entry.parse::<Ipv4Addr>() {
            bypass_ips.push(ip);
            continue;
        }
        match (entry.as_str(), 0).to_socket_addrs() {
            Ok(addrs) => {
                for addr in addrs {
                    if let std::net::SocketAddr::V4(v4) = addr {
                        bypass_ips.push(*v4.ip());
                    }
                }
            }
            Err(error) => {
                tracing::warn!("TUN bypass host {entry} did not resolve: {error}");
            }
        }
    }

    add_split_route(mgr, "0.0.0.0/1", tun_index)?;
    add_split_route(mgr, "128.0.0.0/1", tun_index)?;

    if !bypass_ips.is_empty() {
        let (physical_index, gateway) = get_default_route_windows()?;
        for ip in bypass_ips {
            add_bypass_route(mgr, ip, physical_index, &gateway)?;
        }
    }

    // Prefer the TUN interface for DNS so Windows uses the TUN DNS handler.
    let metric_script = format!(
        "Set-NetIPInterface -InterfaceIndex {tun_index} -AddressFamily IPv4 -InterfaceMetric 1 -ErrorAction Stop"
    );
    let metric_undo = format!(
        "Set-NetIPInterface -InterfaceIndex {tun_index} -AddressFamily IPv4 -InterfaceMetric {original_metric} -ErrorAction SilentlyContinue"
    );
    mgr.apply(
        format!("Windows TUN interface metric ({tun_index})"),
        || run_powershell(&metric_script).map(|_| ()),
        ps_undo(&metric_undo),
    )?;

    let dns_script = format!(
        "Set-DnsClientServerAddress -InterfaceIndex {tun_index} -ServerAddresses @('{}') -ErrorAction Stop",
        config.dns_ip
    );
    let dns_undo = format!(
        "Set-DnsClientServerAddress -InterfaceIndex {tun_index} -ResetServerAddresses -ErrorAction SilentlyContinue"
    );
    mgr.apply(
        format!("Windows TUN DNS ({})", config.dns_ip),
        || run_powershell(&dns_script).map(|_| ()),
        ps_undo(&dns_undo),
    )?;

    tracing::info!(
        "Windows TUN routes installed on interface {tun_index}, dns={}",
        config.dns_ip
    );
    Ok(())
}

#[cfg(target_os = "windows")]
fn add_split_route(
    mgr: &mut SideEffectManager,
    prefix: &str,
    tun_index: u32,
) -> Result<(), String> {
    let script = format!(
        "Remove-NetRoute -DestinationPrefix '{prefix}' -InterfaceIndex {tun_index} -Confirm:$false -ErrorAction SilentlyContinue; \
         New-NetRoute -DestinationPrefix '{prefix}' -InterfaceIndex {tun_index} -NextHop '0.0.0.0' -RouteMetric 1 -PolicyStore ActiveStore -ErrorAction Stop"
    );
    let undo = format!(
        "Remove-NetRoute -DestinationPrefix '{prefix}' -InterfaceIndex {tun_index} -Confirm:$false -ErrorAction SilentlyContinue"
    );
    mgr.apply(
        format!("Windows TUN route {prefix}"),
        || run_powershell(&script).map(|_| ()),
        ps_undo(&undo),
    )
}

#[cfg(target_os = "windows")]
fn add_bypass_route(
    mgr: &mut SideEffectManager,
    ip: std::net::Ipv4Addr,
    physical_index: u32,
    gateway: &str,
) -> Result<(), String> {
    let prefix = format!("{ip}/32");
    let script = format!(
        "Remove-NetRoute -DestinationPrefix '{prefix}' -InterfaceIndex {physical_index} -Confirm:$false -ErrorAction SilentlyContinue; \
         New-NetRoute -DestinationPrefix '{prefix}' -InterfaceIndex {physical_index} -NextHop '{gateway}' -RouteMetric 1 -PolicyStore ActiveStore -ErrorAction Stop"
    );
    let undo = format!(
        "Remove-NetRoute -DestinationPrefix '{prefix}' -InterfaceIndex {physical_index} -Confirm:$false -ErrorAction SilentlyContinue"
    );
    mgr.apply(
        format!("Windows TUN bypass {prefix}"),
        || run_powershell(&script).map(|_| ()),
        ps_undo(&undo),
    )
}

#[cfg(target_os = "windows")]
fn get_tun_interface_index() -> Result<u32, String> {
    if let Some(index) = crate::device::tun_adapter_index() {
        return Ok(index);
    }

    let name = crate::device::WINDOWS_TUN_ADAPTER_NAME;
    let script = format!(
        "$i=0; do {{ $a=Get-NetAdapter -Name '{name}' -ErrorAction SilentlyContinue; \
         if ($a) {{ $a.ifIndex; break }}; Start-Sleep -Milliseconds 200; $i++ }} while ($i -lt 50); \
         if (-not $a) {{ throw 'TUN adapter not found' }}"
    );
    run_powershell(&script)?
        .trim()
        .parse::<u32>()
        .map_err(|error| format!("invalid TUN interface index: {error}"))
}

#[cfg(target_os = "windows")]
fn get_interface_metric(index: u32) -> Result<u32, String> {
    let script = format!(
        "(Get-NetIPInterface -InterfaceIndex {index} -AddressFamily IPv4 -ErrorAction Stop).InterfaceMetric"
    );
    run_powershell(&script)?
        .trim()
        .parse::<u32>()
        .map_err(|error| format!("invalid interface metric: {error}"))
}

#[cfg(target_os = "windows")]
fn get_default_route_windows() -> Result<(u32, String), String> {
    let script = r#"
$r = Get-NetRoute -AddressFamily IPv4 -DestinationPrefix '0.0.0.0/0' |
     Where-Object { $_.NextHop -ne '0.0.0.0' } |
     Sort-Object RouteMetric | Select-Object -First 1
if (-not $r) { throw 'No IPv4 default route' }
"$($r.InterfaceIndex)|$($r.NextHop)"
"#;
    let output = run_powershell(script)?;
    let mut parts = output.trim().split('|');
    let index = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| format!("invalid default route output: {output}"))?;
    let gateway = parts
        .next()
        .ok_or_else(|| format!("missing default gateway: {output}"))?
        .to_string();
    Ok((index, gateway))
}

#[cfg(target_os = "windows")]
pub(crate) fn is_elevated() -> Result<bool, String> {
    let output = run_powershell(
        "([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)",
    )?;
    Ok(output.trim().eq_ignore_ascii_case("true"))
}

#[cfg(target_os = "windows")]
fn ps_undo(script: &str) -> SideEffectUndo {
    SideEffectUndo::Command {
        cmd: "powershell".to_string(),
        args: vec![
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-Command".to_string(),
            script.to_string(),
        ],
    }
}

#[cfg(target_os = "windows")]
fn run_powershell(script: &str) -> Result<String, String> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .map_err(|error| format!("powershell: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "powershell failed: {} {}",
            stderr.trim(),
            stdout.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
