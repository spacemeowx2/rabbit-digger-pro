use net::RpcNet;
use rd_interface::{
    config::NetRef, prelude::*, rd_config, registry::Builder, Address, Net, Registry, Result,
    Server,
};
use server::RpcServer;

mod connection;
mod net;
mod server;
mod session;
#[cfg(test)]
mod tests;
mod types;

#[rd_config]
#[derive(Clone, Copy)]
pub enum Codec {
    Json,
    Cbor,
}

impl Default for Codec {
    fn default() -> Self {
        Codec::Cbor
    }
}

impl From<Codec> for connection::Codec {
    fn from(this: Codec) -> Self {
        match this {
            Codec::Json => connection::Codec::Json,
            Codec::Cbor => connection::Codec::Cbor,
        }
    }
}

#[rd_config]
pub struct RpcNetConfig {
    #[serde(default)]
    net: NetRef,
    server: Address,
    #[serde(default)]
    codec: Codec,
}

#[rd_config]
pub struct RpcServerConfig {
    bind: Address,

    #[serde(default)]
    net: NetRef,
    #[serde(default)]
    listen: NetRef,
    #[serde(default)]
    codec: Codec,
}

impl Builder<Net> for RpcNet {
    const NAME: &'static str = "rpc";
    type Config = RpcNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        Ok(RpcNet::new(
            config.net.value_cloned(),
            config.server,
            true,
            config.codec.into(),
        ))
    }
}

impl Builder<Server> for RpcServer {
    const NAME: &'static str = "rpc";
    type Config = RpcServerConfig;
    type Item = Self;

    fn build(
        Self::Config {
            listen,
            net,
            bind,
            codec,
        }: Self::Config,
    ) -> Result<Self> {
        Ok(RpcServer::new(
            listen.value_cloned(),
            net.value_cloned(),
            bind,
            codec.into(),
        ))
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<RpcNet>();
    registry.add_server::<RpcServer>();

    Ok(())
}
