use std::{future::pending, io, net::SocketAddr, task::Poll};

use rd_interface::{
    async_trait, config::EmptyConfig, registry::Builder, Address, INet, ITcpListener, ITcpStream,
    IUdpSocket, IntoDyn, Net, ReadBuf, Result, TcpBind, TcpConnect, UdpBind, NOT_IMPLEMENTED,
};
pub struct BlackholeNet;

impl Builder<Net> for BlackholeNet {
    const NAME: &'static str = "blackhole";
    type Config = EmptyConfig;
    type Item = BlackholeNet;

    fn build(_config: Self::Config) -> Result<Self::Item> {
        Ok(BlackholeNet)
    }
}

struct BlackItem;

#[async_trait]
impl ITcpStream for BlackItem {
    fn poll_read(
        &mut self,
        _cx: &mut std::task::Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_write(
        &mut self,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }

    async fn peer_addr(&self) -> Result<std::net::SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }

    async fn local_addr(&self) -> Result<std::net::SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }
}

#[async_trait]
impl ITcpListener for BlackItem {
    async fn accept(&self) -> Result<(rd_interface::TcpStream, std::net::SocketAddr)> {
        pending().await
    }

    async fn local_addr(&self) -> Result<std::net::SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }
}

#[async_trait]
impl IUdpSocket for BlackItem {
    async fn local_addr(&self) -> Result<std::net::SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }

    fn poll_recv_from(
        &mut self,
        _cx: &mut std::task::Context<'_>,
        _buf: &mut ReadBuf,
    ) -> Poll<io::Result<SocketAddr>> {
        Poll::Pending
    }

    fn poll_send_to(
        &mut self,
        _cx: &mut std::task::Context<'_>,
        _buf: &[u8],
        _target: &Address,
    ) -> Poll<io::Result<usize>> {
        Poll::Pending
    }
}

#[async_trait]
impl TcpConnect for BlackholeNet {
    async fn tcp_connect(
        &self,
        _ctx: &mut rd_interface::Context,
        _addr: &Address,
    ) -> Result<rd_interface::TcpStream> {
        Ok(BlackItem.into_dyn())
    }
}

#[async_trait]
impl TcpBind for BlackholeNet {
    async fn tcp_bind(
        &self,
        _ctx: &mut rd_interface::Context,
        _addr: &Address,
    ) -> Result<rd_interface::TcpListener> {
        Ok(BlackItem.into_dyn())
    }
}

#[async_trait]
impl UdpBind for BlackholeNet {
    async fn udp_bind(
        &self,
        _ctx: &mut rd_interface::Context,
        _addr: &Address,
    ) -> Result<rd_interface::UdpSocket> {
        Ok(BlackItem.into_dyn())
    }
}

impl INet for BlackholeNet {
    fn provide_tcp_connect(&self) -> Option<&dyn TcpConnect> {
        Some(self)
    }
    fn provide_tcp_bind(&self) -> Option<&dyn TcpBind> {
        Some(self)
    }
    fn provide_udp_bind(&self) -> Option<&dyn UdpBind> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::{assert_net_provider, ProviderCapability};
    use rd_interface::IntoDyn;

    use super::*;

    #[test]
    fn test_provider() {
        let bh = BlackholeNet.into_dyn();

        assert_net_provider(
            &bh,
            ProviderCapability {
                tcp_connect: true,
                tcp_bind: true,
                udp_bind: true,
                ..Default::default()
            },
        );
    }
}
