/// Pick a /24 TUN interface IP that doesn't conflict with existing host interfaces.
///
/// Candidates are tried in order of least likely to conflict:
/// 1. RFC 5737 TEST-NET ranges (reserved for documentation, never routed)
/// 2. RFC 2544 benchmarking (198.18-19.x, same family Clash uses for fake-ip)
/// 3. RFC 6598 CGNAT space (100.64.x, rarely on local interfaces)
/// 4. 10.x private fallback
pub fn suggest_tun_ip() -> String {
    let used: Vec<(u32, u32)> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .iter()
        .filter_map(|iface| match iface.addr.ip() {
            std::net::IpAddr::V4(ip) => {
                let bits = u32::from(ip);
                // Assume /24 for each interface to be conservative
                Some((bits & 0xFFFFFF00, 0xFFFFFF00))
            }
            _ => None,
        })
        .collect();

    const CANDIDATES: &[(u8, u8, u8)] = &[
        // TEST-NET-1/2/3 (RFC 5737)
        (192, 0, 2),    // 192.0.2.0/24
        (198, 51, 100), // 198.51.100.0/24
        (203, 0, 113),  // 203.0.113.0/24
        // RFC 2544 benchmarking (198.18.0.0/15)
        (198, 18, 0),
        (198, 18, 1),
        (198, 19, 0),
        // CGNAT (100.64.0.0/10)
        (100, 64, 0),
        (100, 65, 0),
        // 10.x private fallback
        (10, 89, 0),
        (10, 88, 0),
        (10, 0, 1),
    ];

    for &(a, b, c) in CANDIDATES {
        let net = ((a as u32) << 24) | ((b as u32) << 16) | ((c as u32) << 8);
        let mask = 0xFFFFFF00u32;
        let conflicts = used.iter().any(|&(used_net, used_mask)| {
            let common_mask = mask & used_mask;
            (net & common_mask) == (used_net & common_mask)
        });
        if !conflicts {
            return format!("{a}.{b}.{c}.1/24");
        }
    }

    "10.0.0.1/24".to_string()
}
