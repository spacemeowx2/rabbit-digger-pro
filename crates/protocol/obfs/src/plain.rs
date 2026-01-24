use crate::Obfs;
use rd_interface::{prelude::*, Address, Result, TcpStream};

#[rd_config]
#[derive(Debug, Default)]
pub struct Plain;

impl Obfs for Plain {
    fn tcp_connect(
        &self,
        tcp: TcpStream,
        _ctx: &mut rd_interface::Context,
        _addr: &Address,
    ) -> Result<TcpStream> {
        Ok(tcp)
    }

    fn tcp_accept(&self, tcp: TcpStream, _addr: std::net::SocketAddr) -> Result<TcpStream> {
        Ok(tcp)
    }
}
