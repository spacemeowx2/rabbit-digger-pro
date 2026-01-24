use std::{
    future::pending,
    io,
    net::SocketAddr,
    task::{self, Poll},
    time::{Duration, Instant},
};

use crate::{
    util::{
        forward_udp::{forward_udp, RawUdpSource, UdpEndpoint},
        PollFuture,
    },
    ContextExt,
};
use futures::{ready, TryFutureExt};
use rd_interface::{
    async_trait, config::NetRef, prelude::*, registry::Builder, Address, Context, IServer, Net,
    Result, Server, TcpStream, UdpSocket,
};
use tokio::select;
use tracing::instrument;

/// A server that forwards all connections to target.
#[rd_config]
#[derive(Debug)]
pub struct ForwardServerConfig {
    bind: Address,
    udp_bind: Option<Address>,
    target: Address,
    #[serde(default)]
    tcp: Option<bool>,
    #[serde(default)]
    udp: bool,
    /// The interval to resolve the target address. Used in UDP mode.
    /// If not set, the target address will be resolved only once.
    /// If set to 0, the target address will be resolved every time.
    /// the unit is second.
    #[serde(default)]
    resolve_interval: Option<u64>,
    #[serde(default)]
    net: NetRef,
    #[serde(default)]
    resolve_net: NetRef,
    #[serde(default)]
    listen: NetRef,
}

pub struct ForwardServer {
    listen_net: Net,
    net: Net,
    resolve_net: Net,
    bind: Address,
    udp_bind: Address,
    target: Address,
    tcp: bool,
    udp: bool,
    resolve_interval: Option<Duration>,
}

impl ForwardServer {
    fn new(cfg: ForwardServerConfig) -> ForwardServer {
        ForwardServer {
            listen_net: cfg.listen.value_cloned(),
            net: cfg.net.value_cloned(),
            resolve_net: cfg.resolve_net.value_cloned(),
            bind: cfg.bind.clone(),
            udp_bind: cfg.udp_bind.unwrap_or(cfg.bind),
            target: cfg.target,
            tcp: cfg.tcp.unwrap_or(true),
            udp: cfg.udp,
            resolve_interval: cfg.resolve_interval.map(Duration::from_secs),
        }
    }
}
#[async_trait]
impl IServer for ForwardServer {
    async fn start(&self) -> Result<()> {
        let tcp_task = self.serve_listener();
        let udp_task = self.serve_udp();

        select! {
            r = tcp_task => r?,
            r = udp_task => r?,
        }

        Ok(())
    }
}

impl ForwardServer {
    #[instrument(err, skip(net, socket))]
    async fn serve_connection(
        target: Address,
        socket: TcpStream,
        net: Net,
        addr: SocketAddr,
    ) -> Result<()> {
        let ctx = &mut Context::from_socketaddr(addr);
        let target = net.tcp_connect(ctx, &target).await?;
        ctx.connect_tcp(socket, target).await?;
        Ok(())
    }
    pub async fn serve_listener(&self) -> Result<()> {
        if !self.tcp {
            pending::<()>().await;
            return Ok(());
        }
        let listener = self
            .listen_net
            .tcp_bind(&mut Context::new(), &self.bind)
            .await?;
        loop {
            let (socket, addr) = listener.accept().await?;
            let net = self.net.clone();
            let target = self.target.clone();
            let _ = tokio::spawn(async move {
                if let Err(e) = Self::serve_connection(target, socket, net, addr).await {
                    tracing::error!("Error when serve_connection: {:?}", e);
                }
            });
        }
    }
    async fn serve_udp(&self) -> Result<()> {
        if !self.udp {
            pending::<()>().await;
        }

        let mut ctx = Context::new();
        let listen_udp = self.listen_net.udp_bind(&mut ctx, &self.udp_bind).await?;

        let source = UdpSource::new(
            self.resolve_net.clone(),
            self.target.clone(),
            listen_udp,
            self.resolve_interval,
        );

        forward_udp(source, self.net.clone(), None).await?;

        Ok(())
    }
}

impl Builder<Server> for ForwardServer {
    const NAME: &'static str = "forward";
    type Config = ForwardServerConfig;
    type Item = Self;

    fn build(cfg: Self::Config) -> Result<Self> {
        Ok(ForwardServer::new(cfg))
    }
}

async fn resolve_target(resolve_net: Net, target: Address) -> Result<SocketAddr, io::ErrorKind> {
    let addrs = target
        .resolve(move |d, p| async move {
            resolve_net
                .lookup_host(&Address::Domain(d, p))
                .map_err(|e| e.to_io_err())
                .await
        })
        .await
        .map_err(|e| e.kind())?
        .into_iter()
        .next();
    addrs.ok_or(io::ErrorKind::NotFound)
}

struct UdpSource {
    resolve_net: Net,
    listen_udp: UdpSocket,
    target: Address,
    resolve_interval: Option<Duration>,
    resolve_future: PollFuture<Result<SocketAddr, io::ErrorKind>>,
    resolved: Option<SocketAddr>,
    resolve_at: Option<Instant>,
}

impl UdpSource {
    fn new(
        resolve_net: Net,
        target: Address,
        udp: UdpSocket,
        resolve_interval: Option<Duration>,
    ) -> UdpSource {
        UdpSource {
            resolve_net: resolve_net.clone(),
            listen_udp: udp,
            resolve_future: PollFuture::new(resolve_target(resolve_net, target.clone())),
            resolved: None,
            resolve_at: None,
            target,
            resolve_interval,
        }
    }
}

impl RawUdpSource for UdpSource {
    fn poll_recv(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut rd_interface::ReadBuf,
    ) -> Poll<io::Result<UdpEndpoint>> {
        if let (Some(resolve_at), Some(resolve_interval)) = (self.resolve_at, self.resolve_interval)
        {
            if resolve_at.elapsed() >= resolve_interval {
                self.resolve_future = PollFuture::new(resolve_target(
                    self.resolve_net.clone(),
                    self.target.clone(),
                ));
                self.resolve_at = None;
                self.resolved = None;
            }
        }

        let to = match self.resolved {
            Some(to) => to,
            None => {
                let to = ready!(self.resolve_future.poll(cx))?;
                self.resolved = Some(to);
                to
            }
        };

        if self.resolve_at.is_none() {
            self.resolve_at = Some(Instant::now());
        }

        let packet = loop {
            let from = ready!(self.listen_udp.poll_recv_from(cx, buf))?;

            break UdpEndpoint { from, to };
        };

        Poll::Ready(Ok(packet))
    }

    fn poll_send(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        endpoint: &UdpEndpoint,
    ) -> Poll<io::Result<()>> {
        ready!(self.listen_udp.poll_send_to(cx, buf, &endpoint.to.into()))?;

        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rd_interface::{IntoAddress, IntoDyn};
    use tokio::time::sleep;

    use super::*;
    use crate::tests::{
        assert_echo, assert_echo_udp, spawn_echo_server, spawn_echo_server_udp, TestNet,
    };

    #[tokio::test]
    async fn test_forward_server() {
        let net = TestNet::new().into_dyn();

        let server = ForwardServer {
            listen_net: net.clone(),
            net: net.clone(),
            resolve_net: net.clone(),
            bind: "127.0.0.1:1234".into_address().unwrap(),
            udp_bind: "127.0.0.1:1234".into_address().unwrap(),
            target: "localhost:4321".into_address().unwrap(),
            tcp: true,
            udp: true,
            resolve_interval: None,
        };
        tokio::spawn(async move { server.start().await.unwrap() });
        spawn_echo_server(&net, "127.0.0.1:4321").await;
        spawn_echo_server_udp(&net, "127.0.0.1:4321").await;

        sleep(Duration::from_millis(10)).await;

        assert_echo(&net, "127.0.0.1:1234").await;
        assert_echo_udp(&net, "127.0.0.1:1234").await;
    }
}
