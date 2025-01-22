pub use self::{client::HttpClient, server::HttpServer};

use rd_interface::{
    prelude::*,
    registry::{Builder, NetRef},
    Address, Net, Registry, Result, Server,
};

mod client;
mod server;
#[cfg(test)]
mod tests;

#[rd_config]
#[derive(Debug)]
pub struct HttpNetConfig {
    server: Address,

    #[serde(default)]
    net: NetRef,
}

#[rd_config]
#[derive(Debug)]
pub struct AuthConfig {
    username: String,
    password: String,
}

#[rd_config]
#[derive(Debug)]
pub struct HttpServerConfig {
    bind: Address,
    #[serde(default)]
    net: NetRef,
    #[serde(default)]
    listen: NetRef,
    #[serde(default)]
    auth: Option<AuthConfig>,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            bind: Address::SocketAddr("127.0.0.1:0".parse().unwrap()),
            net: Default::default(),
            listen: Default::default(),
            auth: None,
        }
    }
}

impl Builder<Net> for HttpClient {
    const NAME: &'static str = "http";
    type Config = HttpNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        Ok(HttpClient::new(config.net.value_cloned(), config.server))
    }
}

impl Builder<Server> for server::Http {
    const NAME: &'static str = "http";
    type Config = HttpServerConfig;
    type Item = Self;

    fn build(
        Self::Config {
            listen,
            net,
            bind,
            auth,
        }: Self::Config,
    ) -> Result<Self> {
        if let Some(auth) = auth {
            Ok(server::Http::with_auth(
                listen.value_cloned(),
                net.value_cloned(),
                bind,
                auth.username,
                auth.password,
            ))
        } else {
            Ok(server::Http::new(
                listen.value_cloned(),
                net.value_cloned(),
                bind,
            ))
        }
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<HttpClient>();
    registry.add_server::<server::Http>();
    Ok(())
}
