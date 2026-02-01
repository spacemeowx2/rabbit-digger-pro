use std::net::SocketAddr;

use rd_derive::rd_config;
use rd_interface::{
    async_trait, config::NetRef, prelude::*, registry::Builder, Address, INet, IntoDyn, Net, Result,
};
use trust_dns_resolver::{
    config::{NameServerConfig, Protocol, ResolverConfig, ResolverOpts},
    name_server::GenericConnector,
    AsyncResolver,
};

use self::rd_runtime::RDRuntime;

use super::local::{LocalNet, LocalNetConfig};

/// A net refering to another net.
#[rd_config]
#[derive(Debug)]
#[serde(rename_all = "camelCase")]
pub enum DnsServer {
    Google,
    Cloudflare,
    Custom { nameserver: Vec<SocketAddr> },
}

#[rd_config]
#[derive(Debug)]
pub struct DnsConfig {
    server: DnsServer,
    #[serde(default)]
    net: Option<NetRef>,
}

pub struct DnsNet {
    resolver: AsyncResolver<GenericConnector<RDRuntime>>,
}

#[async_trait]
impl rd_interface::LookupHost for DnsNet {
    async fn lookup_host(&self, addr: &Address) -> Result<Vec<SocketAddr>> {
        // TODO: is it cheap?
        let resolver = self.resolver.clone();
        addr.resolve(move |host, port| async move {
            let response = resolver
                .lookup_ip(host)
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

            let addrs: Vec<SocketAddr> = response
                .into_iter()
                .map(|ip| SocketAddr::new(ip, port))
                .collect();

            Ok(addrs)
        })
        .await
        .map_err(Into::into)
    }
}

impl INet for DnsNet {
    fn provide_lookup_host(&self) -> Option<&dyn rd_interface::LookupHost> {
        Some(self)
    }
}

impl Builder<Net> for DnsNet {
    const NAME: &'static str = "dns";
    type Config = DnsConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        let net = config
            .net
            .map(|i| i.value_cloned())
            .unwrap_or_else(|| LocalNet::new(LocalNetConfig::default()).into_dyn());
        let resolver_config = match config.server {
            DnsServer::Google => ResolverConfig::google(),
            DnsServer::Cloudflare => ResolverConfig::cloudflare(),
            DnsServer::Custom { nameserver } => ResolverConfig::from_parts(
                None,
                vec![],
                nameserver
                    .into_iter()
                    .map(nameserver_config)
                    .collect::<Vec<_>>(),
            ),
        };
        let resolver = AsyncResolver::new(
            resolver_config,
            ResolverOpts::default(),
            GenericConnector::new(RDRuntime { net: net.clone() }),
        );

        Ok(Self { resolver })
    }
}

fn nameserver_config(socket_addr: SocketAddr) -> NameServerConfig {
    NameServerConfig {
        socket_addr,
        protocol: Protocol::Udp,
        tls_dns_name: None,
        trust_negative_responses: false,
        bind_addr: None,
    }
}

mod rd_runtime {
    use std::{
        io,
        pin::Pin,
        task::{Context, Poll},
    };

    use super::*;
    use futures::{ready, Future, TryFutureExt};
    use parking_lot::Mutex;
    use trust_dns_resolver::{
        name_server::{RuntimeProvider, Spawn},
        proto::{error::ProtoError, iocompat::AsyncIoTokioAsStd, udp::DnsUdpSocket, TokioTime},
    };

    tokio::task_local! {
        pub(super) static NET:  Net;
    }

    enum Inner {
        Connecting(Pin<Box<dyn Future<Output = io::Result<rd_interface::UdpSocket>> + Send>>),
        Connected(rd_interface::UdpSocket),
    }

    // TODO: remove this workaround: https://github.com/bluejekyll/trust-dns/issues/1669
    pub struct RDUdpSocket {
        inner: Mutex<Inner>,
    }

    impl RDUdpSocket {
        fn poll_udp<F, R>(&self, cx: &mut Context<'_>, f: F) -> Poll<io::Result<R>>
        where
            F: FnOnce(&mut Context<'_>, &mut rd_interface::UdpSocket) -> Poll<io::Result<R>>,
        {
            loop {
                let inner = &mut *self.inner.lock();
                match inner {
                    Inner::Connecting(fut) => {
                        let socket = ready!(fut.as_mut().poll(cx))?;
                        *inner = Inner::Connected(socket);
                    }
                    Inner::Connected(ref mut socket) => return f(cx, socket),
                }
            }
        }
    }

    #[async_trait]
    impl DnsUdpSocket for RDUdpSocket {
        type Time = TokioTime;

        fn poll_recv_from(
            &self,
            cx: &mut std::task::Context<'_>,
            buf: &mut [u8],
        ) -> Poll<io::Result<(usize, SocketAddr)>> {
            self.poll_udp(cx, |cx, s| {
                let mut buf = tokio::io::ReadBuf::new(buf);
                let addr = ready!(s.poll_recv_from(cx, &mut buf))?;
                let len = buf.filled().len();

                Poll::Ready(Ok((len, addr)))
            })
        }

        fn poll_send_to(
            &self,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
            target: SocketAddr,
        ) -> Poll<io::Result<usize>> {
            self.poll_udp(cx, |cx, s| s.poll_send_to(cx, buf, &target.into()))
        }
    }

    #[derive(Clone)]
    pub struct RDHandle(pub Net);
    impl Spawn for RDHandle {
        fn spawn_bg<F>(&mut self, future: F)
        where
            F: Future<Output = Result<(), ProtoError>> + Send + 'static,
        {
            let net = self.0.clone();
            let _join = tokio::spawn(async move {
                if let Err(e) = NET.scope(net, future).await {
                    eprintln!("spawn return error {e:?}");
                }
            });
        }
    }

    #[derive(Clone)]
    pub struct RDRuntime {
        pub(super) net: Net,
    }

    impl RuntimeProvider for RDRuntime {
        type Handle = RDHandle;
        type Tcp = AsyncIoTokioAsStd<tokio::net::TcpStream>;
        type Timer = TokioTime;
        type Udp = RDUdpSocket;

        fn create_handle(&self) -> Self::Handle {
            RDHandle(self.net.clone())
        }

        fn connect_tcp(
            &self,
            server_addr: SocketAddr,
        ) -> Pin<Box<dyn Send + Future<Output = io::Result<Self::Tcp>>>> {
            Box::pin(async move {
                let stream = tokio::net::TcpStream::connect(server_addr).await?;
                Ok(AsyncIoTokioAsStd(stream))
            })
        }

        fn bind_udp(
            &self,
            local_addr: SocketAddr,
            _server_addr: SocketAddr,
        ) -> Pin<Box<dyn Send + Future<Output = io::Result<Self::Udp>>>> {
            let net = self.net.clone();
            Box::pin(async move {
                let fut = async move {
                    net.udp_bind(&mut rd_interface::Context::new(), &local_addr.into())
                        .map_err(|e| e.to_io_err())
                        .await
                };

                Ok(RDUdpSocket {
                    inner: Mutex::new(Inner::Connecting(Box::pin(fut))),
                })
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::{assert_net_provider, ProviderCapability};
    use rd_interface::IntoDyn;

    use super::*;

    #[test]
    fn test_provider() {
        let dns = DnsNet::build(DnsConfig {
            server: DnsServer::Google,
            net: None,
        })
        .unwrap()
        .into_dyn();

        assert_net_provider(
            &dns,
            ProviderCapability {
                lookup_host: true,
                ..Default::default()
            },
        );
    }
}
