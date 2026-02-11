use client::{HysteriaNet, HysteriaNetConfig};
use rd_interface::{registry::Builder, Net, Registry, Result, Server};
use server::{HysteriaServer, HysteriaServerConfig};

mod client;
mod codec;
mod salamander;
mod server;
mod stream;
mod transport;
mod udp;

impl Builder<Net> for HysteriaNet {
    const NAME: &'static str = "hysteria";
    type Config = HysteriaNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        HysteriaNet::new(config)
    }
}

impl Builder<Server> for HysteriaServer {
    const NAME: &'static str = "hysteria";
    type Config = HysteriaServerConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        HysteriaServer::new(config)
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<HysteriaNet>();
    registry.add_server::<server::HysteriaServer>();
    Ok(())
}
