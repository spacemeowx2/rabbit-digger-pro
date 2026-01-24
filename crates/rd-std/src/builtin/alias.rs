use rd_interface::{config::NetRef, prelude::*, registry::Builder, INet, Net, Result};

pub struct AliasNet(rd_interface::Net);

impl AliasNet {
    fn new(net: rd_interface::Net) -> AliasNet {
        AliasNet(net)
    }
}

impl INet for AliasNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        self.0.provide_tcp_connect()
    }

    fn provide_tcp_bind(&self) -> Option<&dyn rd_interface::TcpBind> {
        self.0.provide_tcp_bind()
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        self.0.provide_udp_bind()
    }

    fn provide_lookup_host(&self) -> Option<&dyn rd_interface::LookupHost> {
        self.0.provide_lookup_host()
    }
}

/// A net refering to another net.
#[rd_config]
#[derive(Debug)]
pub struct AliasNetConfig {
    net: NetRef,
}

impl Builder<Net> for AliasNet {
    const NAME: &'static str = "alias";
    type Config = AliasNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        Ok(AliasNet::new(config.net.value_cloned()))
    }
}

#[cfg(test)]
mod tests {
    use rd_interface::IntoDyn;

    use super::*;
    use crate::tests::{
        assert_echo, assert_echo_udp, assert_net_provider, spawn_echo_server,
        spawn_echo_server_udp, ProviderCapability, TestNet,
    };

    #[test]
    fn test_provider() {
        let net = TestNet::new().into_dyn();

        let alias = AliasNet::new(net).into_dyn();

        assert_net_provider(
            &alias,
            ProviderCapability {
                tcp_connect: true,
                tcp_bind: true,
                udp_bind: true,
                lookup_host: true,
            },
        );
    }

    #[tokio::test]
    async fn test_alias_net() {
        let parent_net = TestNet::new().into_dyn();
        let net = AliasNet::new(parent_net.clone()).into_dyn();

        spawn_echo_server(&net, "127.0.0.1:26666").await;
        assert_echo(&parent_net, "127.0.0.1:26666").await;

        spawn_echo_server_udp(&parent_net, "127.0.0.1:26666").await;
        assert_echo_udp(&net, "127.0.0.1:26666").await;
    }
}
