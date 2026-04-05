use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    sync::atomic::{AtomicU32, Ordering},
};

use parking_lot::RwLock;

/// A pool that allocates fake IPv4 addresses for domain names.
///
/// Used in TUN mode to map domains to IPs in the `198.18.0.0/15` range.
/// The TUN server intercepts DNS queries, returns a fake IP, then
/// reverse-looks-up the domain when the connection arrives.
pub struct FakeIpPool {
    min: u32,
    max: u32,
    cursor: AtomicU32,
    domain_to_ip: RwLock<HashMap<String, Ipv4Addr>>,
    ip_to_domain: RwLock<HashMap<Ipv4Addr, String>>,
}

impl FakeIpPool {
    /// Create a new pool from a CIDR range string, e.g. `"198.18.0.0/15"`.
    pub fn new(cidr: &str) -> Self {
        let (base, prefix_len) = parse_cidr(cidr);
        let base_u32 = u32::from(base);
        let host_bits = 32 - prefix_len;
        let network_size = 1u32 << host_bits;

        // Skip network address and gateway (first two), exclude broadcast (last one)
        let min = base_u32 + 2;
        let max = base_u32 + network_size - 2;

        FakeIpPool {
            min,
            max,
            cursor: AtomicU32::new(min),
            domain_to_ip: RwLock::new(HashMap::new()),
            ip_to_domain: RwLock::new(HashMap::new()),
        }
    }

    /// Get or allocate a fake IP for the given domain.
    pub fn allocate(&self, domain: &str) -> Ipv4Addr {
        // Fast path: already allocated
        if let Some(&ip) = self.domain_to_ip.read().get(domain) {
            return ip;
        }

        // Slow path: allocate new
        let mut d2i = self.domain_to_ip.write();
        let mut i2d = self.ip_to_domain.write();

        // Double-check after acquiring write lock
        if let Some(&ip) = d2i.get(domain) {
            return ip;
        }

        let ip_u32 = self.next_ip();
        let ip = Ipv4Addr::from(ip_u32);

        // If this IP was previously allocated to another domain, evict it
        if let Some(old_domain) = i2d.remove(&ip) {
            d2i.remove(&old_domain);
        }

        d2i.insert(domain.to_string(), ip);
        i2d.insert(ip, domain.to_string());

        ip
    }

    /// Look up the domain for a fake IP. Returns `None` if not a fake IP.
    pub fn lookup(&self, ip: Ipv4Addr) -> Option<String> {
        self.ip_to_domain.read().get(&ip).cloned()
    }

    /// Check if the given address is within the fake IP range.
    pub fn contains(&self, ip: &IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => {
                let n = u32::from(*v4);
                n >= self.min && n <= self.max
            }
            IpAddr::V6(_) => false,
        }
    }

    /// The gateway IP (first usable address in the range).
    pub fn gateway(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.min - 1)
    }

    fn next_ip(&self) -> u32 {
        loop {
            let current = self.cursor.fetch_add(1, Ordering::Relaxed);
            let ip = self.min + (current - self.min) % (self.max - self.min + 1);
            return ip;
        }
    }
}

/// Try to generate a fake DNS response for a DNS query packet.
/// Returns `None` if the packet is not a valid A query.
pub fn generate_fake_response(pool: &FakeIpPool, request: &[u8]) -> Option<Vec<u8>> {
    use simple_dns::{Packet, ResourceRecord, CLASS, QTYPE};

    let req = Packet::parse(request).ok()?;
    let question = req.questions.first()?;

    // Only handle A record queries (IPv4)
    if question.qtype != QTYPE::TYPE(simple_dns::TYPE::A) {
        return None;
    }

    let domain = question.qname.to_string();
    let domain = domain.trim_end_matches('.');
    let fake_ip = pool.allocate(domain);

    let mut resp = Packet::new_reply(req.id());
    resp.questions.push(question.clone());
    resp.answers.push(ResourceRecord::new(
        question.qname.clone(),
        CLASS::IN,
        1, // TTL = 1 to prevent caching
        simple_dns::rdata::RData::A(simple_dns::rdata::A {
            address: u32::from(fake_ip),
        }),
    ));

    Some(resp.build_bytes_vec().ok()?)
}

fn parse_cidr(cidr: &str) -> (Ipv4Addr, u32) {
    let parts: Vec<&str> = cidr.split('/').collect();
    let ip: Ipv4Addr = parts[0].parse().expect("Invalid CIDR IP");
    let prefix: u32 = parts[1].parse().expect("Invalid CIDR prefix");
    (ip, prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_and_lookup() {
        let pool = FakeIpPool::new("198.18.0.0/15");

        let ip1 = pool.allocate("google.com");
        let ip2 = pool.allocate("github.com");
        let ip3 = pool.allocate("google.com"); // same domain → same IP

        assert_ne!(ip1, ip2);
        assert_eq!(ip1, ip3);
        assert_eq!(pool.lookup(ip1), Some("google.com".to_string()));
        assert_eq!(pool.lookup(ip2), Some("github.com".to_string()));
    }

    #[test]
    fn test_contains() {
        let pool = FakeIpPool::new("198.18.0.0/15");

        assert!(pool.contains(&IpAddr::V4(Ipv4Addr::new(198, 18, 0, 5))));
        assert!(pool.contains(&IpAddr::V4(Ipv4Addr::new(198, 19, 255, 200))));
        assert!(!pool.contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(!pool.contains(&IpAddr::V6("::1".parse().unwrap())));
    }

    #[test]
    fn test_gateway() {
        let pool = FakeIpPool::new("198.18.0.0/15");
        assert_eq!(pool.gateway(), Ipv4Addr::new(198, 18, 0, 1));
    }

    #[test]
    fn test_wraparound_evicts() {
        // Small range: 10.0.0.0/30 → 4 IPs, usable: 10.0.0.2 and 10.0.0.3 (minus network+gw and broadcast)
        // Actually /30 = 4 addrs: .0 (net), .1 (gw), .2 (usable), .3 (broadcast)
        // min=2, max=2 → only 1 usable. Use /29 instead.
        // /29 = 8 addrs: .0-.7, min=.2, max=.6 → 5 usable
        let pool = FakeIpPool::new("10.0.0.0/29");

        let ip_a = pool.allocate("a.com");
        let ip_b = pool.allocate("b.com");
        let ip_c = pool.allocate("c.com");
        let ip_d = pool.allocate("d.com");
        let ip_e = pool.allocate("e.com");

        // All unique
        let ips = vec![ip_a, ip_b, ip_c, ip_d, ip_e];
        let unique: std::collections::HashSet<_> = ips.iter().collect();
        assert_eq!(unique.len(), 5);

        // Next allocation wraps around and evicts "a.com"
        let ip_f = pool.allocate("f.com");
        assert_eq!(ip_f, ip_a); // reused IP
        assert_eq!(pool.lookup(ip_f), Some("f.com".to_string()));
        assert_eq!(pool.lookup(ip_a), Some("f.com".to_string())); // same IP
                                                                  // "a.com" is evicted
        assert_ne!(pool.allocate("a.com"), ip_a); // gets a new IP
    }
}
