pub use drop_abort::DropAbort;
pub use forward_udp::forward_udp;
pub use lru_cache::LruCache;
pub use net::{CombineNet, NotImplementedNet};
pub use peekable_tcpstream::PeekableTcpStream;
pub use poll_future::PollFuture;
pub use udp_connector::UdpConnector;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

pub mod async_fn;
mod drop_abort;
pub mod forward_udp;
mod lru_cache;
mod net;
mod peekable_tcpstream;
mod poll_future;
mod udp_connector;

/// Helper function for converting IPv4 mapped IPv6 address
///
/// This is the same as `Ipv6Addr::to_ipv4_mapped`, but it is still unstable in the current libstd
fn to_ipv4_mapped(ipv6: &Ipv6Addr) -> Option<Ipv4Addr> {
    match ipv6.octets() {
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, a, b, c, d] => Some(Ipv4Addr::new(a, b, c, d)),
        _ => None,
    }
}

pub fn resolve_mapped_socket_addr(addr: SocketAddr) -> SocketAddr {
    if let SocketAddr::V6(ref a) = addr {
        if let Some(v4) = to_ipv4_mapped(a.ip()) {
            return SocketAddr::new(v4.into(), a.port());
        }
    }

    addr
}

/// If the given address is reserved.
pub fn is_reserved(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(a) => {
            let [a0, a1, ..] = a.octets();
            a.is_unspecified()
                || a.is_loopback()
                || a.is_private()
                || (a0 == 169 && a1 == 254) // 169.254.0.0/16
                || (224..=239).contains(&a0) // 224.0.0.0/4
                || (240..=255).contains(&a0) // 240.0.0.0/4 (incl. 255.255.255.255)
        }
        IpAddr::V6(a) => {
            a.is_unspecified()
                || a.is_loopback()
                || {
                    // fc00::/7 unique local
                    let seg0 = a.segments()[0];
                    (seg0 & 0xfe00) == 0xfc00
                }
                || a.is_unicast_link_local()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_mapped_socket_addr() {
        assert_eq!(
            resolve_mapped_socket_addr(SocketAddr::from(([127, 0, 0, 1], 1))),
            SocketAddr::from(([127, 0, 0, 1], 1))
        );
        assert_eq!(
            resolve_mapped_socket_addr(SocketAddr::from((
                [0, 0, 0, 0, 0, 0xffff, 0x1122, 0x3344],
                1
            ))),
            SocketAddr::from(([0x11, 0x22, 0x33, 0x44], 1))
        );
        assert_eq!(
            resolve_mapped_socket_addr(SocketAddr::from((
                [0, 0, 0, 0, 0, 0xfffc, 0x1122, 0x3344],
                1
            ))),
            SocketAddr::from(([0, 0, 0, 0, 0, 0xfffc, 0x1122, 0x3344], 1))
        );
    }

    #[test]
    fn test_is_reserved() {
        assert!(is_reserved(IpAddr::from([0, 0, 0, 0])));
        assert!(is_reserved(IpAddr::from([0, 0, 0, 255])));
        assert!(!is_reserved(IpAddr::from([1, 0, 0, 0])));
        assert!(is_reserved(IpAddr::from([127, 0, 0, 1])));
        assert!(is_reserved(IpAddr::from([10, 0, 0, 1])));
        assert!(is_reserved(IpAddr::from([169, 254, 0, 1])));
        assert!(is_reserved(IpAddr::from([192, 168, 0, 1])));
        assert!(is_reserved(IpAddr::from([172, 16, 0, 1])));
        assert!(is_reserved(IpAddr::from([224, 0, 0, 1])));
        assert!(is_reserved(IpAddr::from([240, 0, 0, 1])));

        // ::1
        assert!(is_reserved(IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1])));
        assert!(is_reserved(IpAddr::from([0xfc00, 0, 0, 0, 0, 0, 0, 1])));
        assert!(is_reserved(IpAddr::from([0xfc80, 0, 0, 0, 0, 0, 0, 1])));
    }
}
