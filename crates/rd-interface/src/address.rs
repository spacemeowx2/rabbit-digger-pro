use serde::{de, ser};
use std::{
    fmt,
    io::{Error, ErrorKind, Result},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    str::FromStr,
};

/// An address represents a network address using domain.
#[derive(Debug, PartialEq, Clone, PartialOrd, Eq, Ord)]
pub struct AddressDomain {
    pub domain: String,
    pub port: u16,
}

impl From<AddressDomain> for Address {
    fn from(AddressDomain { domain, port }: AddressDomain) -> Self {
        Address::Domain(domain, port)
    }
}

impl FromStr for AddressDomain {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut parts = s.rsplitn(2, ':');
        let port: u16 = parts
            .next()
            .ok_or_else(no_addr)?
            .parse()
            .map_err(|_| no_addr())?;
        let host = parts.next().ok_or_else(no_addr)?;
        Ok(AddressDomain {
            domain: host.to_string(),
            port,
        })
    }
}

impl fmt::Display for AddressDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.domain, self.port)
    }
}

impl<'de> de::Deserialize<'de> for AddressDomain {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}

impl ser::Serialize for AddressDomain {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

/// Address can be IPv4, IPv6 address or a domain with port.
#[derive(Debug, PartialEq, Clone, PartialOrd, Eq, Ord, Hash)]
pub enum Address {
    SocketAddr(SocketAddr),
    Domain(String, u16),
}

/// Converts to address value.
pub trait IntoAddress: Send {
    fn into_address(self) -> Result<Address>;
    fn into_socket_addr(self) -> Result<SocketAddr>
    where
        Self: Sized,
    {
        self.into_address()?.to_socket_addr()
    }
}

fn no_addr() -> Error {
    ErrorKind::AddrNotAvailable.into()
}

fn strip_brackets(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

fn host_to_address(host: &str, port: u16) -> Address {
    match strip_brackets(host).parse::<IpAddr>() {
        Ok(ip) => {
            let addr = SocketAddr::new(ip, port);
            addr.into()
        }
        Err(_) => Address::Domain(host.to_string(), port),
    }
}

impl IntoAddress for &Address {
    fn into_address(self) -> Result<Address> {
        Ok(self.clone())
    }
}

impl IntoAddress for &str {
    fn into_address(self) -> Result<Address> {
        let mut parts = self.rsplitn(2, ':');
        let port: u16 = parts
            .next()
            .ok_or_else(no_addr)?
            .parse()
            .map_err(|_| no_addr())?;
        let host = parts.next().ok_or_else(no_addr)?;
        Ok(host_to_address(host, port))
    }
}

impl FromStr for Address {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        IntoAddress::into_address(s)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Address::Domain(domain, port) => write!(f, "{domain}:{port}"),
            Address::SocketAddr(s) => write!(f, "{s}"),
        }
    }
}

impl IntoAddress for (&str, u16) {
    fn into_address(self) -> Result<Address> {
        Ok(host_to_address(self.0, self.1))
    }
}

impl IntoAddress for (String, u16) {
    fn into_address(self) -> Result<Address> {
        Ok(host_to_address(&self.0, self.1))
    }
}

impl IntoAddress for (IpAddr, u16) {
    fn into_address(self) -> Result<Address> {
        let addr = SocketAddr::new(self.0, self.1);
        Ok(addr.into())
    }
}

impl IntoAddress for SocketAddr {
    fn into_address(self) -> Result<Address> {
        Ok(self.into())
    }
}

impl IntoAddress for Address {
    fn into_address(self) -> Result<Address> {
        Ok(self)
    }
}

impl From<SocketAddr> for Address {
    fn from(addr: SocketAddr) -> Self {
        Address::SocketAddr(addr)
    }
}

impl From<(IpAddr, u16)> for Address {
    fn from((ip, port): (IpAddr, u16)) -> Self {
        Address::SocketAddr(SocketAddr::new(ip, port))
    }
}

impl Address {
    /// Return `0.0.0.0:0` or `[::]:0` by given `addr` address family.
    pub fn any_addr_port(addr: &SocketAddr) -> Self {
        match addr {
            SocketAddr::V4(_) => {
                Address::SocketAddr(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0))
            }
            SocketAddr::V6(_) => {
                Address::SocketAddr(SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 0))
            }
        }
    }

    pub fn to_any_addr_port(&self) -> Result<Self> {
        match self.normalize() {
            Either::Left(s) => Ok(Address::any_addr_port(&s)),
            Either::Right(_) => Err(no_addr()),
        }
    }

    /// Converts to SocketAddr if Address can be convert to.
    /// Otherwise [AddrNotAvailable](std::io::ErrorKind::AddrNotAvailable) is returned.
    pub fn to_socket_addr(&self) -> Result<SocketAddr> {
        match self.normalize() {
            Either::Left(s) => Ok(s),
            Either::Right(_) => Err(no_addr()),
        }
    }

    /// Resolve domain to SocketAddr using `f`.
    pub async fn resolve<Fut>(&self, f: impl FnOnce(String, u16) -> Fut) -> Result<Vec<SocketAddr>>
    where
        Fut: std::future::Future<Output = Result<Vec<SocketAddr>>>,
    {
        match self.normalize() {
            Either::Left(s) => Ok(vec![s]),
            Either::Right((d, p)) => f(d.to_string(), p).await,
        }
    }

    /// Get host part of the Address
    pub fn host(&self) -> String {
        match self {
            Address::SocketAddr(SocketAddr::V4(s)) => s.ip().to_string(),
            Address::SocketAddr(SocketAddr::V6(s)) => format!("[{}]", s.ip()),
            Address::Domain(d, _) => d.to_string(),
        }
    }

    /// Get port of the Address
    pub fn port(&self) -> u16 {
        match self {
            Address::SocketAddr(s) => s.port(),
            Address::Domain(_, p) => *p,
        }
    }

    /// parse domain first, if it can be parsed as IP address,
    /// then return it, otherwise return the original domain.
    fn normalize(&self) -> Either<SocketAddr, (&String, u16)> {
        match self {
            Address::SocketAddr(s) => Either::Left(*s),
            Address::Domain(d, p) => match strip_brackets(d).parse::<IpAddr>() {
                Ok(ip) => Either::Left(SocketAddr::new(ip, *p)),
                Err(_) => Either::Right((d, *p)),
            },
        }
    }

    /// parse domain first, if it can be parsed as IP address,
    /// then return it, otherwise return the original domain.
    pub fn to_normalized(&self) -> Address {
        match self.normalize() {
            Either::Left(s) => Address::SocketAddr(s),
            Either::Right((d, p)) => Address::Domain(d.to_string(), p),
        }
    }

    /// parse domain first, if it can be parsed as IP address,
    /// then return it, otherwise return the original domain.
    pub fn into_normalized(self) -> Address {
        match self {
            Address::SocketAddr(_) => self,
            Address::Domain(d, p) => match strip_brackets(&d).parse::<IpAddr>() {
                Ok(ip) => Address::SocketAddr(SocketAddr::new(ip, p)),
                Err(_) => Address::Domain(d, p),
            },
        }
    }

    /// Returns true if the address is domain.
    pub fn is_domain(&self) -> bool {
        match self {
            Address::Domain(_, _) => true,
            _ => false,
        }
    }

    /// Returns true if the address is IP address.
    pub fn is_socket_addr(&self) -> bool {
        match self {
            Address::SocketAddr(_) => true,
            _ => false,
        }
    }
}

enum Either<T, U> {
    Left(T),
    Right(U),
}

impl<'de> de::Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}

impl ser::Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    const IPV4_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
    const IPV6_ADDR: IpAddr = IpAddr::V6(Ipv6Addr::new(1, 2, 3, 4, 5, 6, 7, 8));
    const DOMAIN: &str = "example.com";
    const IP_DOMAIN: &str = "[1:2:3:4:5:6:7:8]";

    #[test]
    fn test_any_addr_port() {
        let ipv4_addr = SocketAddr::new(IPV4_ADDR, 1234);
        let ipv6_addr = SocketAddr::new(IPV6_ADDR, 1234);

        assert_eq!(
            Address::any_addr_port(&ipv4_addr),
            "0.0.0.0:0".into_address().unwrap()
        );
        assert_eq!(
            Address::any_addr_port(&ipv6_addr),
            "[::]:0".into_address().unwrap()
        );
    }

    #[test]
    fn test_address_domain_convert() {
        let domain_addr = AddressDomain {
            domain: DOMAIN.to_string(),
            port: 1234,
        };

        assert_eq!(&domain_addr.to_string(), "example.com:1234");
        assert_eq!(
            "example.com:1234".parse::<AddressDomain>().unwrap(),
            domain_addr
        );
    }

    #[test]
    fn test_address_convert() {
        let ipv4_addr = Address::SocketAddr(SocketAddr::new(IPV4_ADDR, 1234));
        let ipv6_addr = Address::SocketAddr(SocketAddr::new(IPV6_ADDR, 1234));
        let domain_addr = Address::Domain(DOMAIN.to_string(), 1234);

        // &str
        assert_eq!(ipv4_addr, "1.2.3.4:1234".into_address().unwrap());
        assert_eq!(ipv6_addr, "[1:2:3:4:5:6:7:8]:1234".into_address().unwrap());
        assert_eq!(domain_addr, "example.com:1234".into_address().unwrap());

        // (&str, u16)
        assert_eq!(ipv4_addr, ("1.2.3.4", 1234).into_address().unwrap());
        assert_eq!(
            ipv6_addr,
            ("[1:2:3:4:5:6:7:8]", 1234).into_address().unwrap()
        );
        assert_eq!(domain_addr, ("example.com", 1234).into_address().unwrap());

        // (String, u16)
        assert_eq!(
            ipv4_addr,
            ("1.2.3.4".to_string(), 1234).into_address().unwrap()
        );
        assert_eq!(
            ipv6_addr,
            ("[1:2:3:4:5:6:7:8]".to_string(), 1234)
                .into_address()
                .unwrap()
        );
        assert_eq!(
            domain_addr,
            ("example.com".to_string(), 1234).into_address().unwrap()
        );

        // (IpAddr, u16)
        assert_eq!(ipv4_addr, (IPV4_ADDR, 1234).into_address().unwrap());
        assert_eq!(ipv6_addr, (IPV6_ADDR, 1234).into_address().unwrap());
    }

    #[test]
    fn test_serde() {
        let ipv4_addr = Address::SocketAddr(SocketAddr::new(IPV4_ADDR, 1234));
        let ipv6_addr = Address::SocketAddr(SocketAddr::new(IPV6_ADDR, 1234));
        let domain_addr = Address::Domain(DOMAIN.to_string(), 1234);

        assert_eq!(
            serde_json::to_string(&ipv4_addr).unwrap(),
            "\"1.2.3.4:1234\""
        );
        assert_eq!(
            serde_json::to_string(&ipv6_addr).unwrap(),
            "\"[1:2:3:4:5:6:7:8]:1234\""
        );
        assert_eq!(
            serde_json::to_string(&domain_addr).unwrap(),
            "\"example.com:1234\""
        );

        assert_eq!(
            serde_json::from_str::<Address>(&serde_json::to_string(&ipv4_addr).unwrap()).unwrap(),
            ipv4_addr
        );
        assert_eq!(
            serde_json::from_str::<Address>(&serde_json::to_string(&ipv6_addr).unwrap()).unwrap(),
            ipv6_addr
        );
        assert_eq!(
            serde_json::from_str::<Address>(&serde_json::to_string(&domain_addr).unwrap()).unwrap(),
            domain_addr
        );
    }

    #[tokio::test]
    async fn test_methods() {
        let ipv4_addr = Address::SocketAddr(SocketAddr::new(IPV4_ADDR, 1234));
        let ipv6_addr = Address::SocketAddr(SocketAddr::new(IPV6_ADDR, 1234));
        let domain_addr = Address::Domain(DOMAIN.to_string(), 1234);
        let domain_ip_addr = Address::Domain(IP_DOMAIN.to_string(), 1234);

        assert_eq!(
            ipv4_addr.to_any_addr_port().unwrap(),
            Address::SocketAddr(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0)),
        );
        assert_eq!(
            ipv6_addr.to_any_addr_port().unwrap(),
            Address::SocketAddr(SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 0)),
        );
        assert!(domain_addr.to_any_addr_port().is_err());
        assert_eq!(
            domain_ip_addr.to_any_addr_port().unwrap(),
            Address::SocketAddr(SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 0))
        );

        assert_eq!(
            ipv4_addr.to_socket_addr().unwrap(),
            SocketAddr::new(IPV4_ADDR, 1234)
        );
        assert_eq!(
            ipv6_addr.to_socket_addr().unwrap(),
            SocketAddr::new(IPV6_ADDR, 1234)
        );
        assert!(domain_addr.to_socket_addr().is_err());
        assert_eq!(
            domain_ip_addr.to_socket_addr().unwrap(),
            SocketAddr::new(IPV6_ADDR, 1234)
        );

        assert_eq!(
            ipv4_addr.resolve(dummy_resolve).await.unwrap(),
            vec![SocketAddr::new(IPV4_ADDR, 1234)]
        );
        assert_eq!(
            domain_ip_addr.resolve(dummy_resolve).await.unwrap(),
            vec![SocketAddr::new(IPV6_ADDR, 1234)]
        );

        assert_eq!(ipv4_addr.host(), "1.2.3.4");
        assert_eq!(ipv6_addr.host(), "[1:2:3:4:5:6:7:8]");
        assert_eq!(domain_addr.host(), "example.com");
        assert_eq!(domain_ip_addr.host(), "[1:2:3:4:5:6:7:8]");

        assert_eq!(ipv4_addr.port(), 1234);
        assert_eq!(ipv6_addr.port(), 1234);
        assert_eq!(domain_addr.port(), 1234);
        assert_eq!(domain_ip_addr.port(), 1234);
    }

    async fn dummy_resolve(_host: String, _port: u16) -> Result<Vec<SocketAddr>> {
        panic!("dummy_resolve shouldn't be called")
    }
}
