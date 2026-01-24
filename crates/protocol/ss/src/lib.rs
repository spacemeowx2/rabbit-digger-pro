use client::{SSNet, SSNetConfig};
use rd_interface::{registry::Builder, Net, Registry, Result, Server};
use server::{SSServer, SSServerConfig};

mod client;
mod server;
#[cfg(test)]
mod tests;
mod udp;
mod wrapper;

impl Builder<Net> for SSNet {
    const NAME: &'static str = "shadowsocks";
    type Config = SSNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        SSNet::new(config)
    }
}

impl Builder<Server> for SSServer {
    const NAME: &'static str = "shadowsocks";
    type Config = SSServerConfig;
    type Item = Self;

    fn build(cfg: Self::Config) -> Result<Self> {
        SSServer::new(cfg)
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<SSNet>();
    registry.add_server::<SSServer>();

    Ok(())
}
