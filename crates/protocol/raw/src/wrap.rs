use std::{
    io,
    net::SocketAddr,
    task::{self, Poll},
};

use futures::ready;
use rd_interface::{
    async_trait, impl_async_read_write, Address, ITcpListener, ITcpStream, IUdpSocket, IntoDyn,
    Result,
};
use tokio::sync::Mutex;
use tokio_smoltcp::{TcpListener, TcpStream, UdpSocket};

pub struct TcpStreamWrap(TcpStream);

impl TcpStreamWrap {
    pub(crate) fn new(stream: TcpStream) -> Self {
        Self(stream)
    }
}

#[async_trait]
impl ITcpStream for TcpStreamWrap {
    async fn peer_addr(&self) -> Result<SocketAddr> {
        Ok(self.0.peer_addr()?)
    }

    async fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.0.local_addr()?)
    }

    impl_async_read_write!(0);
}

pub struct TcpListenerWrap(pub(crate) Mutex<TcpListener>, pub(crate) SocketAddr);

#[async_trait]
impl ITcpListener for TcpListenerWrap {
    async fn accept(&self) -> Result<(rd_interface::TcpStream, SocketAddr)> {
        let (tcp, addr) = self.0.lock().await.accept().await?;
        Ok((TcpStreamWrap::new(tcp).into_dyn(), addr))
    }

    async fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.1)
    }
}

pub struct UdpSocketWrap {
    inner: UdpSocket,
}

impl UdpSocketWrap {
    pub(crate) fn new(inner: UdpSocket) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl IUdpSocket for UdpSocketWrap {
    async fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.inner.local_addr()?)
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut rd_interface::ReadBuf,
    ) -> Poll<io::Result<SocketAddr>> {
        let UdpSocketWrap { inner, .. } = &mut *self;
        let (size, from) = ready!(inner.poll_recv_from(cx, buf.initialize_unfilled()))?;
        buf.advance(size);
        Poll::Ready(Ok(from))
    }

    fn poll_send_to(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        target: &Address,
    ) -> Poll<io::Result<usize>> {
        let UdpSocketWrap { inner, .. } = &mut *self;

        // TODO: support domain
        let size = ready!(inner.poll_send_to(cx, buf, target.to_socket_addr()?))?;
        if size != buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "failed to send all bytes",
            ))
            .into();
        }

        Poll::Ready(Ok(size))
    }
}
