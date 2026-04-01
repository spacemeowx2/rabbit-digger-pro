use client::{VlessNet, VlessNetConfig};
use rd_interface::{registry::Builder, Net, Registry, Result, Server};
use server::{VlessServer, VlessServerConfig};

mod client;
mod common;
#[cfg(test)]
mod interop_tests;
mod reality;
mod server;
#[cfg(test)]
mod tests;

impl Builder<Net> for VlessNet {
    const NAME: &'static str = "vless";
    type Config = VlessNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        VlessNet::new(config)
    }
}

impl Builder<Server> for VlessServer {
    const NAME: &'static str = "vless";
    type Config = VlessServerConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        VlessServer::new(config)
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<VlessNet>();
    registry.add_server::<VlessServer>();
    Ok(())
}
