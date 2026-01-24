use std::{
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    task::{self, Poll},
    time::Duration,
};

use futures::{ready, stream::FuturesUnordered, Future, FutureExt, StreamExt};
use itertools::Itertools;
use parking_lot::Mutex;
use rd_interface::{
    async_trait, config::NetRef, impl_async_read_write, prelude::*, registry::Builder, Address,
    INet, IntoDyn, Net, ReadBuf, Result, TcpListener, TcpStream, UdpSocket,
};
use socket2::{Domain, SockRef, Socket, Type};
use tokio::{
    net,
    time::{sleep, timeout},
};
use tracing::instrument;

/// A local network.
#[rd_config]
#[derive(Debug, Clone, Default)]
pub struct LocalNetConfig {
    /// set ttl
    #[serde(default)]
    pub ttl: Option<u32>,

    /// set nodelay. default is true
    #[serde(default)]
    pub nodelay: Option<bool>,

    /// set SO_MARK on linux
    pub mark: Option<u32>,

    /// bind to device
    pub bind_device: Option<String>,

    /// bind to address
    pub bind_addr: Option<IpAddr>,

    /// timeout of TCP connect, in seconds.
    pub connect_timeout: Option<u64>,

    /// enable keepalive on TCP socket, in seconds.
    /// default is 600s. 0 means disable.
    #[serde(default)]
    pub tcp_keepalive: Option<f64>,

    /// change the system receive buffer size of the socket.
    /// by default it remains unchanged.
    pub recv_buffer_size: Option<usize>,
    /// change the system send buffer size of the socket.
    /// by default it remains unchanged.
    pub send_buffer_size: Option<usize>,

    /// Change the default system DNS resolver to custom one.
    #[serde(default)]
    pub lookup_host: Option<NetRef>,
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
enum UdpState {
    Idle,
    LookupHost(Mutex<BoxFuture<io::Result<Vec<SocketAddr>>>>),
    Sending(SocketAddr),
}

#[derive(Default)]
pub struct LocalNet {
    cfg: LocalNetConfig,
    resolver: Resolver,
}
pub struct CompatTcp(pub(crate) net::TcpStream);
pub struct Listener(net::TcpListener, LocalNetConfig);
pub struct Udp {
    inner: net::UdpSocket,
    state: UdpState,
    resolver: Resolver,
}

#[derive(Clone, Default)]
struct Resolver {
    net: Option<Net>,
}

impl LocalNetConfig {
    fn set_socket(
        &self,
        socket: SockRef,
        _addr: SocketAddr,
        is_tcp: bool,
        is_accept: bool,
    ) -> Result<()> {
        socket.set_nonblocking(true)?;

        if let Some(size) = self.recv_buffer_size {
            socket.set_recv_buffer_size(size)?;
        }
        if let Some(size) = self.send_buffer_size {
            socket.set_send_buffer_size(size)?;
        }

        if let (Some(local_addr), false) = (self.bind_addr, is_accept) {
            socket.bind(&SocketAddr::new(local_addr, 0).into())?;
        }

        if let Some(ttl) = self.ttl {
            socket.set_ttl_v4(ttl)?;
        }

        if is_tcp {
            socket.set_tcp_nodelay(self.nodelay.unwrap_or(true))?;

            let keepalive_duration = self.tcp_keepalive.unwrap_or(600.0);
            if keepalive_duration > 0.0 {
                let keepalive = socket2::TcpKeepalive::new()
                    .with_time(Duration::from_secs_f64(keepalive_duration));
                socket.set_tcp_keepalive(&keepalive)?;
            }
        }

        #[cfg(target_os = "linux")]
        if let Some(mark) = self.mark {
            socket.set_mark(mark)?;
        }

        #[cfg(target_os = "linux")]
        if let (Some(device), false) = (&self.bind_device, is_accept) {
            socket.bind_device(Some(device.as_bytes()))?;
        }

        #[cfg(target_os = "macos")]
        if let (Some(device), false) = (&self.bind_device, is_accept) {
            let device = std::ffi::CString::new(device.as_bytes())
                .map_err(rd_interface::error::map_other)?;
            unsafe {
                let idx = libc::if_nametoindex(device.as_ptr());
                if idx == 0 {
                    return Err(io::Error::last_os_error().into());
                }

                socket.bind_device_by_index_v4(std::num::NonZeroU32::new(idx))?;
            }
        }

        Ok(())
    }
}

impl Resolver {
    fn new(net: Option<Net>) -> Self {
        Resolver { net }
    }
    async fn lookup_host(self, domain: String, port: u16) -> io::Result<Vec<SocketAddr>> {
        Ok(match self.net {
            Some(net) => net.lookup_host(&Address::Domain(domain, port)).await?,
            None => tokio::net::lookup_host((domain, port)).await?.collect(),
        })
    }
}

impl LocalNet {
    pub fn new(cfg: LocalNetConfig) -> LocalNet {
        let net = cfg.lookup_host.as_ref().map(|n| n.value_cloned());
        LocalNet {
            cfg,
            resolver: Resolver::new(net),
        }
    }
    async fn tcp_connect_single(&self, addr: SocketAddr) -> Result<net::TcpStream> {
        let socket = match addr {
            SocketAddr::V4(_) => Socket::new(Domain::IPV4, Type::STREAM, None)?,
            SocketAddr::V6(_) => Socket::new(Domain::IPV6, Type::STREAM, None)?,
        };

        self.cfg
            .set_socket(SockRef::from(&socket), addr, true, false)?;

        let socket = net::TcpSocket::from_std_stream(socket.into());

        let tcp = match self.cfg.connect_timeout {
            None => socket.connect(addr).await?,
            Some(secs) => timeout(Duration::from_secs(secs), socket.connect(addr)).await??,
        };

        Ok(tcp)
    }
    async fn tcp_connect_happy_eyeballs(&self, addr: &Address) -> Result<TcpStream> {
        // TODO: resolve A, AAAA separately
        let addrs = addr
            .resolve(|d, p| self.resolver.clone().lookup_host(d, p))
            .await?;
        let mut last_err = None;

        // interleave the addresses, v6 first
        let v4_addrs = addrs.iter().filter(|addr| addr.is_ipv4());
        let v6_addrs = addrs.iter().filter(|addr| addr.is_ipv6());
        let addrs = v6_addrs.interleave(v4_addrs);

        let mut unordered = addrs
            .enumerate()
            .map(|(i, addr)| async move {
                sleep(Duration::from_millis(i as u64 * 250)).await;
                self.tcp_connect_single(*addr).await
            })
            .collect::<FuturesUnordered<_>>();

        while let Some(res) = unordered.next().await {
            match res {
                Ok(stream) => return Ok(CompatTcp::new(stream).into_dyn()),
                Err(err) => last_err = Some(err),
            }
        }

        Err(last_err.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "could not resolve to any address",
            )
            .into()
        }))
    }
    async fn tcp_bind_single(&self, addr: SocketAddr) -> Result<net::TcpListener> {
        let listener = net::TcpListener::bind(addr).await?;

        Ok(listener)
    }
    async fn udp_bind_single(&self, addr: SocketAddr) -> Result<net::UdpSocket> {
        let udp = match addr {
            SocketAddr::V4(_) => Socket::new(Domain::IPV4, Type::DGRAM, None)?,
            SocketAddr::V6(_) => Socket::new(Domain::IPV6, Type::DGRAM, None)?,
        };

        self.cfg
            .set_socket(SockRef::from(&udp), addr, false, false)?;

        if self.cfg.bind_addr.is_none() {
            udp.bind(&addr.into())?;
        }

        let udp = net::UdpSocket::from_std(udp.into())?;

        Ok(udp)
    }
}

impl std::fmt::Debug for LocalNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalNet").finish()
    }
}

#[async_trait]
impl rd_interface::ITcpStream for CompatTcp {
    async fn peer_addr(&self) -> Result<SocketAddr> {
        self.0.peer_addr().map_err(Into::into)
    }
    async fn local_addr(&self) -> Result<SocketAddr> {
        self.0.local_addr().map_err(Into::into)
    }

    impl_async_read_write!(0);
}
impl CompatTcp {
    fn new(t: net::TcpStream) -> CompatTcp {
        CompatTcp(t)
    }
}

#[async_trait]
impl rd_interface::ITcpListener for Listener {
    async fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        let (socket, addr) = self.0.accept().await?;

        self.1
            .set_socket(SockRef::from(&socket), addr, true, true)?;

        Ok((CompatTcp::new(socket).into_dyn(), addr))
    }

    async fn local_addr(&self) -> Result<SocketAddr> {
        self.0.local_addr().map_err(Into::into)
    }
}

impl Udp {
    fn new(socket: net::UdpSocket, resolver: Resolver) -> Udp {
        Udp {
            inner: socket,
            state: UdpState::Idle,
            resolver,
        }
    }
    fn poll_send_to_ready(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<()>> {
        let Udp { inner, state, .. } = self;

        loop {
            match state {
                UdpState::Idle => return Poll::Ready(Ok(())),
                UdpState::LookupHost(fut) => {
                    let addr = *ready!(fut.get_mut().poll_unpin(cx))?
                        .first()
                        .ok_or_else(|| io::Error::from(io::ErrorKind::AddrNotAvailable))?;
                    *state = UdpState::Sending(addr)
                }
                UdpState::Sending(addr) => {
                    ready!(inner.poll_send_to(cx, buf, *addr)?);
                    *state = UdpState::Idle;
                }
            }
        }
    }
}

#[async_trait]
impl rd_interface::IUdpSocket for Udp {
    async fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.local_addr().map_err(Into::into)
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<SocketAddr>> {
        let Udp { inner, .. } = &mut *self;

        inner.poll_recv_from(cx, buf)
    }

    fn poll_send_to(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        target: &Address,
    ) -> Poll<io::Result<usize>> {
        match self.state {
            UdpState::Idle => match target {
                Address::SocketAddr(s) => {
                    self.state = UdpState::Sending(*s);
                }
                Address::Domain(domain, port) => {
                    let fut = Mutex::new(
                        self.resolver
                            .clone()
                            .lookup_host(domain.clone(), *port)
                            .boxed(),
                    );
                    self.state = UdpState::LookupHost(fut);
                }
            },
            _ => {}
        }
        ready!(self.poll_send_to_ready(cx, buf))?;
        Poll::Ready(Ok(buf.len()))
    }
}

#[async_trait]
impl rd_interface::TcpConnect for LocalNet {
    #[instrument(err)]
    async fn tcp_connect(
        &self,
        _ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> Result<TcpStream> {
        self.tcp_connect_happy_eyeballs(addr).await
    }
}

#[async_trait]
impl rd_interface::TcpBind for LocalNet {
    #[instrument(err)]
    async fn tcp_bind(
        &self,
        _ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> Result<TcpListener> {
        let addrs = addr
            .resolve(|d, p| self.resolver.clone().lookup_host(d, p))
            .await?;
        let mut last_err = None;

        for addr in addrs {
            match self.tcp_bind_single(addr).await {
                Ok(listener) => return Ok(Listener(listener, self.cfg.clone()).into_dyn()),
                Err(e) => last_err = Some(e),
            }
        }

        Err(last_err.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "could not resolve to any address",
            )
            .into()
        }))
    }
}

#[async_trait]
impl rd_interface::UdpBind for LocalNet {
    #[instrument(err)]
    async fn udp_bind(
        &self,
        _ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> Result<UdpSocket> {
        let addrs = addr
            .resolve(|d, p| self.resolver.clone().lookup_host(d, p))
            .await?;
        let mut last_err = None;

        for addr in addrs {
            match self.udp_bind_single(addr).await {
                Ok(udp) => return Ok(Udp::new(udp, self.resolver.clone()).into_dyn()),
                Err(e) => last_err = Some(e),
            }
        }

        Err(last_err.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "could not resolve to any address",
            )
            .into()
        }))
    }
}

#[async_trait]
impl rd_interface::LookupHost for LocalNet {
    #[instrument(err)]
    async fn lookup_host(&self, addr: &Address) -> Result<Vec<SocketAddr>> {
        let addr = addr
            .resolve(|d, p| self.resolver.clone().lookup_host(d, p))
            .await?;
        Ok(addr)
    }
}

impl INet for LocalNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_tcp_bind(&self) -> Option<&dyn rd_interface::TcpBind> {
        Some(self)
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        Some(self)
    }

    fn provide_lookup_host(&self) -> Option<&dyn rd_interface::LookupHost> {
        Some(self)
    }
}

impl Builder<Net> for LocalNet {
    const NAME: &'static str = "local";
    type Config = LocalNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        Ok(LocalNet::new(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{
        assert_echo, assert_echo_udp, assert_net_provider, spawn_echo_server,
        spawn_echo_server_udp, ProviderCapability,
    };

    #[tokio::test]
    async fn test_local_net() {
        let net = LocalNet::new(LocalNetConfig::default()).into_dyn();

        spawn_echo_server(&net, "127.0.0.1:26666").await;
        assert_echo(&net, "127.0.0.1:26666").await;

        spawn_echo_server_udp(&net, "127.0.0.1:26666").await;
        assert_echo_udp(&net, "127.0.0.1:26666").await;
    }

    #[test]
    fn test_provider() {
        let net = LocalNet::new(LocalNetConfig::default()).into_dyn();

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
