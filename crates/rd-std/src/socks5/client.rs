use futures::ready;
use socks5_protocol::{
    AuthMethod, AuthRequest, AuthResponse, CommandRequest, CommandResponse, Version,
};

use crate::socks5::common::map_err;

use super::common::{pack_udp, parse_udp, ra2sa};
use rd_interface::{
    async_trait, constant::UDP_BUFFER_SIZE, impl_async_read_write, Address, INet, ITcpStream,
    IUdpSocket, IntoAddress, IntoDyn, Net, ReadBuf, Result, TcpStream, UdpSocket, NOT_IMPLEMENTED,
};
use std::{
    io,
    net::SocketAddr,
    task::{self, Poll},
};
use tokio::io::{AsyncWriteExt, BufWriter};

pub struct Socks5Client {
    server: Address,
    net: Net,
}

pub struct Socks5TcpStream(TcpStream);

pub struct Socks5UdpSocket {
    udp: UdpSocket,
    _tcp: TcpStream,
    server_addr: SocketAddr,
    send_buf: Vec<u8>,
}

#[async_trait]
impl IUdpSocket for Socks5UdpSocket {
    async fn local_addr(&self) -> Result<SocketAddr> {
        self.udp.local_addr().await
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<SocketAddr>> {
        loop {
            let addr = ready!(self.udp.poll_recv_from(cx, buf))?;
            if addr == self.server_addr {
                break;
            }
        }

        let addr = parse_udp(buf)?;

        Poll::Ready(Ok(addr.to_socket_addr()?))
    }

    fn poll_send_to(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        target: &Address,
    ) -> Poll<io::Result<usize>> {
        let Socks5UdpSocket {
            udp,
            server_addr,
            send_buf,
            ..
        } = &mut *self;

        if send_buf.is_empty() {
            pack_udp(target.clone(), buf, send_buf)?;
        }

        ready!(udp.poll_send_to(cx, send_buf, &(*server_addr).into()))?;
        send_buf.clear();

        Poll::Ready(Ok(buf.len()))
    }
}

#[async_trait]
impl ITcpStream for Socks5TcpStream {
    async fn peer_addr(&self) -> Result<SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }

    async fn local_addr(&self) -> Result<SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }

    impl_async_read_write!(0);
}

#[async_trait]
impl rd_interface::TcpConnect for Socks5Client {
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &rd_interface::Address,
    ) -> Result<TcpStream> {
        let mut socket = self.net.tcp_connect(ctx, &self.server).await?;

        let req = CommandRequest::connect(ra2sa(addr.clone().into_address()?));
        let _resp = self.send_command(&mut socket, req).await?;

        Ok(Socks5TcpStream(socket).into_dyn())
    }
}

#[async_trait]
impl rd_interface::UdpBind for Socks5Client {
    async fn udp_bind(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &rd_interface::Address,
    ) -> Result<UdpSocket> {
        let server_addr = self
            .net
            .lookup_host(&self.server)
            .await?
            .into_iter()
            .next()
            .ok_or(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "Failed to lookup domain",
            ))?;

        let mut socket = self.net.tcp_connect(ctx, &server_addr.into()).await?;

        let req = CommandRequest::udp_associate(ra2sa(addr.clone().into_address()?));
        let resp = self.send_command(&mut socket, req).await?;
        let client = self
            .net
            .udp_bind(ctx, &rd_interface::Address::any_addr_port(&server_addr))
            .await?;

        let addr = resp.address.to_socket_addr().map_err(map_err)?;

        Ok(Socks5UdpSocket {
            udp: client,
            _tcp: socket,
            server_addr: addr,
            send_buf: Vec::with_capacity(UDP_BUFFER_SIZE),
        }
        .into_dyn())
    }
}

impl INet for Socks5Client {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        Some(self)
    }
}

impl Socks5Client {
    pub fn new(net: Net, server: Address) -> Self {
        Self { server, net }
    }
    async fn send_command(
        &self,
        socket: &mut TcpStream,
        command_req: CommandRequest,
    ) -> Result<CommandResponse> {
        let mut socket = BufWriter::with_capacity(512, socket);

        let version = Version::V5;
        let auth_req = AuthRequest::new(vec![AuthMethod::Noauth]);
        version.write(&mut socket).await.map_err(map_err)?;
        auth_req.write(&mut socket).await.map_err(map_err)?;
        socket.flush().await?;

        Version::read(&mut socket).await.map_err(map_err)?;
        let resp = AuthResponse::read(&mut socket).await.map_err(map_err)?;
        if resp.method() != AuthMethod::Noauth {
            return Err(rd_interface::Error::Other("Auth failed".to_string().into()));
        }

        command_req.write(&mut socket).await.map_err(map_err)?;
        socket.flush().await?;

        let command_resp = CommandResponse::read(socket).await.map_err(map_err)?;

        Ok(command_resp)
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::{assert_net_provider, ProviderCapability, TestNet};
    use rd_interface::{IntoAddress, IntoDyn};

    use super::*;

    #[test]
    fn test_provider() {
        let net = TestNet::new().into_dyn();

        let socks5 = Socks5Client::new(net, "127.0.0.1:12345".into_address().unwrap()).into_dyn();

        assert_net_provider(
            &socks5,
            ProviderCapability {
                tcp_connect: true,
                udp_bind: true,
                ..Default::default()
            },
        );
    }
}
