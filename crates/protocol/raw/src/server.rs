use crate::{
    forward::forward_net,
    net::{NetParams, RawNet},
};
use rd_interface::{
    async_trait, config::NetRef, prelude::*, rd_config, registry::Builder, Error, IServer, Net,
    Result, Server,
};

#[rd_config]
pub struct RawServerConfig {
    #[serde(default)]
    net: NetRef,
    /// Must be raw net.
    listen: NetRef,
}

pub struct RawServer {
    net: Net,
    params: NetParams,
}

#[async_trait]
impl IServer for RawServer {
    async fn start(&self) -> rd_interface::Result<()> {
        let params = &self.params;
        forward_net(
            self.net.clone(),
            params.smoltcp_net.clone(),
            params.map.clone(),
            params.ip_cidr,
        )
        .await?;

        Ok(())
    }
}

impl RawServer {
    fn new(config: RawServerConfig) -> Result<Self> {
        let net = config.net.value_cloned();
        let listen = config.listen.value_cloned();
        let raw_net = listen
            .get_inner_net_by::<RawNet>()
            .ok_or_else(|| Error::other("net must be `raw` type."))?;
        let params = raw_net
            .get_params()
            .ok_or_else(|| Error::other("The `raw` net must has forward enabled."))?;

        Ok(RawServer { net, params })
    }
}

impl Builder<Server> for RawServer {
    const NAME: &'static str = "raw";

    type Config = RawServerConfig;
    type Item = RawServer;

    fn build(config: Self::Config) -> Result<Self> {
        RawServer::new(config)
    }
}
