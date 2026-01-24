use std::net::SocketAddr;

use obfs_net::{ObfsNet, ObfsNetConfig};
use rd_interface::{
    prelude::*, registry::Builder, Address, Context, Net, Registry, Result, TcpStream,
};

mod http_simple;
mod obfs_net;
mod plain;

impl Builder<Net> for ObfsNet {
    const NAME: &'static str = "obfs";
    type Config = ObfsNetConfig;
    type Item = ObfsNet;

    fn build(config: Self::Config) -> Result<Self> {
        ObfsNet::new(config)
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<ObfsNet>();

    Ok(())
}

/// An obfs protocol used in front of a `TcpStream`
pub trait Obfs {
    /// Wrap the `TcpStream` with obfs request and response. Used by client.
    fn tcp_connect(&self, tcp: TcpStream, ctx: &mut Context, addr: &Address) -> Result<TcpStream>;
    /// Wrap the `TcpStream` with obfs request and response. Used by server.
    fn tcp_accept(&self, tcp: TcpStream, addr: SocketAddr) -> Result<TcpStream>;
}

#[rd_config]
#[derive(Debug)]
#[serde(rename_all = "snake_case")]
pub enum ObfsType {
    Http(http_simple::HttpSimple),
    Plain(plain::Plain),
}

impl Obfs for ObfsType {
    fn tcp_connect(&self, tcp: TcpStream, ctx: &mut Context, addr: &Address) -> Result<TcpStream> {
        match self {
            ObfsType::Http(i) => i.tcp_connect(tcp, ctx, addr),
            ObfsType::Plain(i) => i.tcp_connect(tcp, ctx, addr),
        }
    }

    fn tcp_accept(&self, tcp: TcpStream, addr: SocketAddr) -> Result<TcpStream> {
        match self {
            ObfsType::Http(i) => i.tcp_accept(tcp, addr),
            ObfsType::Plain(i) => i.tcp_accept(tcp, addr),
        }
    }
}
