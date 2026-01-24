use std::{io, net::SocketAddr};

use futures::{future::BoxFuture, FutureExt};
use rd_interface::{
    async_trait,
    prelude::*,
    registry::{Builder, NetRef},
    Address, Arc, INet, Net, Result, TcpStream, UdpSocket,
};

type Resolver =
    Arc<dyn Fn(String, u16) -> BoxFuture<'static, io::Result<Vec<SocketAddr>>> + Send + Sync>;

// Resolves domain names to IP addresses before connecting.
#[rd_config]
#[derive(Debug)]
pub struct ResolveConfig {
    net: NetRef,
    resolve_net: NetRef,
    #[serde(default = "bool_true")]
    ipv4: bool,
    #[serde(default = "bool_true")]
    ipv6: bool,
}

fn bool_true() -> bool {
    true
}

pub struct ResolveNet {
    net: Net,
    resolver: Resolver,
}

impl ResolveNet {
    pub fn new(net: Net, resolve_net: Net, ipv4: bool, ipv6: bool) -> ResolveNet {
        let resolver: Resolver = Arc::new(move |domain: String, port: u16| {
            let resolve_net = resolve_net.clone();
            async move {
                Ok(resolve_net
                    .lookup_host(&Address::Domain(domain, port))
                    .await?
                    .into_iter()
                    .filter(|i| (ipv4 && i.is_ipv4()) || (ipv6 && i.is_ipv6()))
                    .collect())
            }
            .boxed()
        });
        ResolveNet { net, resolver }
    }
}

#[async_trait]
impl rd_interface::TcpConnect for ResolveNet {
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> Result<TcpStream> {
        let addrs = addr.resolve(&*self.resolver).await?;
        let mut last_err = None;

        for addr in addrs {
            match self.net.tcp_connect(ctx, &addr.into()).await {
                Ok(stream) => return Ok(stream),
                Err(e) => last_err = Some(e),
            }
        }

        Err(last_err.unwrap_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "could not resolve to any address").into()
        }))
    }
}

#[async_trait]
impl rd_interface::UdpBind for ResolveNet {
    async fn udp_bind(&self, ctx: &mut rd_interface::Context, addr: &Address) -> Result<UdpSocket> {
        let addrs = addr.resolve(&*self.resolver).await?;
        let mut last_err = None;

        for addr in addrs {
            match self.net.udp_bind(ctx, &addr.into()).await {
                Ok(udp) => return Ok(udp),
                Err(e) => last_err = Some(e),
            }
        }

        Err(last_err.unwrap_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "could not resolve to any address").into()
        }))
    }
}

impl INet for ResolveNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_tcp_bind(&self) -> Option<&dyn rd_interface::TcpBind> {
        self.net.provide_tcp_bind()
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        Some(self)
    }

    fn provide_lookup_host(&self) -> Option<&dyn rd_interface::LookupHost> {
        self.net.provide_lookup_host()
    }
}

impl Builder<Net> for ResolveNet {
    const NAME: &'static str = "resolve";
    type Config = ResolveConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        Ok(ResolveNet::new(
            config.net.value_cloned(),
            config.resolve_net.value_cloned(),
            config.ipv4,
            config.ipv6,
        ))
    }
}

#[cfg(test)]
mod tests {
    use rd_interface::IntoDyn;

    use crate::tests::{
        assert_echo, assert_net_provider, spawn_echo_server, ProviderCapability, TestNet,
    };

    use super::*;

    #[tokio::test]
    async fn test_resolve_net() {
        let test_net = TestNet::new().into_dyn();
        let net = ResolveNet::new(test_net.clone(), test_net, true, true).into_dyn();

        let addr = Address::Domain("localhost".to_string(), 80);
        let addrs = net.lookup_host(&addr).await.unwrap();
        let wanted = vec![SocketAddr::from(([127, 0, 0, 1], 80))];

        assert_eq!(addrs, wanted);

        spawn_echo_server(&net, "127.0.0.1:1234").await;
        assert_echo(&net, "localhost:1234").await;
    }

    #[test]
    fn test_provider() {
        let test_net = TestNet::new().into_dyn();
        let net = ResolveNet::new(test_net.clone(), test_net, true, true).into_dyn();

        assert_net_provider(
            &net,
            ProviderCapability {
                tcp_connect: true,
                tcp_bind: true,
                udp_bind: true,
                lookup_host: true,
            },
        );
    }
}
