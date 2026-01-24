// UDP: https://github.com/shadowsocks/shadowsocks-rust/blob/0433b3ec09bcaa26f7460a50287b56c67b687a34/crates/shadowsocks-service/src/local/redir/udprelay/sys/unix/linux.rs#L56

use std::{
    io,
    net::SocketAddr,
    task::{self, Poll},
    time::Duration,
};

use super::socket::{create_tcp_listener, TransparentUdp};
use crate::{
    builtin::local::CompatTcp,
    util::{
        forward_udp::{forward_udp, RawUdpSource, UdpEndpoint},
        is_reserved, LruCache,
    },
    ContextExt,
};
use futures::ready;
use rd_derive::rd_config;
use rd_interface::{
    async_trait, config::NetRef, error::ErrorContext, registry::Builder, schemars, Address,
    Context, IServer, IntoAddress, IntoDyn, Net, Result, Server,
};
use tokio::{
    net::{TcpListener, TcpStream},
    select,
};
use tracing::instrument;

#[rd_config]
#[derive(Debug)]
pub struct TProxyServerConfig {
    bind: Address,
    mark: Option<u32>,
    #[serde(default)]
    net: NetRef,
}

pub struct TProxyServer {
    bind: Address,
    mark: Option<u32>,
    net: Net,
}

#[async_trait]
impl IServer for TProxyServer {
    async fn start(&self) -> Result<()> {
        let tcp_listener = create_tcp_listener(self.bind.to_socket_addr()?).await?;
        let udp_listener = TransparentUdp::listen(self.bind.to_socket_addr()?)?;

        select! {
            r = self.serve_listener(tcp_listener) => r,
            r = self.serve_udp(udp_listener) => r,
        }
    }
}

impl TProxyServer {
    pub fn new(TProxyServerConfig { bind, mark, net }: TProxyServerConfig) -> Self {
        TProxyServer {
            bind,
            mark,
            net: net.value_cloned(),
        }
    }

    async fn serve_udp(&self, listener: TransparentUdp) -> Result<()> {
        let source = UdpSource::new(listener, self.mark);

        forward_udp(source, self.net.clone(), None)
            .await
            .context("forward udp")?;

        Ok(())
    }

    async fn serve_listener(&self, listener: TcpListener) -> Result<()> {
        loop {
            let (socket, addr) = listener.accept().await?;

            let net = self.net.clone();
            let _ = tokio::spawn(async move {
                if let Err(e) = Self::serve_connection(net, socket, addr).await {
                    tracing::error!("Error when serve_connection: {:?}", e);
                }
            });
        }
    }

    #[instrument(err, skip(net, socket))]
    async fn serve_connection(net: Net, socket: TcpStream, addr: SocketAddr) -> Result<()> {
        let target = socket.local_addr()?;

        let ctx = &mut Context::from_socketaddr(addr);
        let target_tcp = net.tcp_connect(ctx, &target.into_address()?).await?;
        let socket = CompatTcp(socket).into_dyn();

        ctx.connect_tcp(socket, target_tcp).await?;

        Ok(())
    }
}

impl Builder<Server> for TProxyServer {
    const NAME: &'static str = "tproxy";
    type Config = TProxyServerConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        Ok(TProxyServer::new(config))
    }
}

struct UdpSource {
    tudp: TransparentUdp,

    mark: Option<u32>,
    cache: LruCache<SocketAddr, TransparentUdp>,
}

impl UdpSource {
    fn new(tudp: TransparentUdp, mark: Option<u32>) -> Self {
        UdpSource {
            tudp,
            mark,
            cache: LruCache::with_expiry_duration_and_capacity(Duration::from_secs(30), 128),
        }
    }
}

impl RawUdpSource for UdpSource {
    fn poll_recv(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut rd_interface::ReadBuf,
    ) -> Poll<io::Result<UdpEndpoint>> {
        let UdpSource { tudp, cache, .. } = self;
        cache.poll_clear_expired(cx);

        let (size, from, to) = ready!(tudp.poll_recv(cx, buf.initialize_unfilled()))?;
        buf.advance(size);

        Poll::Ready(Ok(UdpEndpoint { from, to }))
    }

    fn poll_send(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        endpoint: &UdpEndpoint,
    ) -> Poll<io::Result<()>> {
        let UdpSource { cache, .. } = self;
        let UdpEndpoint { from, to } = endpoint;

        let udp = match cache.get(from) {
            Some(udp) => udp,
            None => {
                let result = TransparentUdp::bind_any(*from, self.mark);
                let udp = match result {
                    Ok(udp) => udp,
                    Err(e) => {
                        tracing::error!("Failed to bind any addr: {}. Reason: {:?}", from, e);

                        return Poll::Ready(Ok(()));
                    }
                };
                cache.insert(*from, udp);
                cache
                    .get(from)
                    .expect("impossible: failed to get by from_addr")
            }
        };

        if let Err(e) = ready!(udp.poll_send_to(cx, buf, *to)) {
            tracing::error!(
                "Failed to send from {} to addr: {}. Reason: {:?}",
                from,
                to,
                e
            );
        }

        // Don't cache reserved address
        if is_reserved(from.ip()) {
            cache.remove(from);
        }

        Poll::Ready(Ok(()))
    }
}
