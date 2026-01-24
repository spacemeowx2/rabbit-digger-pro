use std::io;

use connect_tcp::connect_tcp;
use connect_udp::connect_udp;
use rd_interface::{async_trait, AsyncRead, AsyncWrite, Context, UdpChannel, UdpSocket};

mod connect_tcp;
mod connect_udp;

#[async_trait]
pub trait ContextExt {
    async fn connect_udp(&mut self, a: UdpChannel, b: UdpSocket) -> io::Result<()>;
    async fn connect_tcp<A, B>(&mut self, a: A, b: B) -> io::Result<()>
    where
        A: AsyncRead + AsyncWrite + Unpin + Send + 'static,
        B: AsyncRead + AsyncWrite + Unpin + Send + 'static;
}

#[async_trait]
impl ContextExt for Context {
    async fn connect_udp(&mut self, a: UdpChannel, b: UdpSocket) -> io::Result<()> {
        connect_udp(self, a, b).await
    }

    async fn connect_tcp<A, B>(&mut self, a: A, b: B) -> io::Result<()>
    where
        A: AsyncRead + AsyncWrite + Unpin + Send + 'static,
        B: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        connect_tcp(self, a, b).await
    }
}
