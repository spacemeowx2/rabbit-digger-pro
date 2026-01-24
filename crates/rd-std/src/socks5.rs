pub use self::{client::Socks5Client, server::Socks5Server};

use rd_interface::{
    prelude::*,
    registry::{Builder, NetRef},
    Address, Net, Registry, Result, Server,
};

mod client;
mod common;
mod server;
#[cfg(test)]
mod tests;

#[rd_config]
#[derive(Debug)]
pub struct Socks5NetConfig {
    server: Address,

    #[serde(default)]
    net: NetRef,
}

#[rd_config]
#[derive(Debug)]
pub struct Socks5ServerConfig {
    bind: Address,

    #[serde(default)]
    net: NetRef,
    #[serde(default)]
    listen: NetRef,
}

impl Builder<Net> for Socks5Client {
    const NAME: &'static str = "socks5";
    type Config = Socks5NetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        Ok(Socks5Client::new(config.net.value_cloned(), config.server))
    }
}

impl Builder<Server> for server::Socks5 {
    const NAME: &'static str = "socks5";
    type Config = Socks5ServerConfig;
    type Item = Self;

    fn build(Self::Config { listen, net, bind }: Self::Config) -> Result<Self> {
        Ok(server::Socks5::new(
            listen.value_cloned(),
            net.value_cloned(),
            bind,
        ))
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<Socks5Client>();
    registry.add_server::<server::Socks5>();
    Ok(())
}
