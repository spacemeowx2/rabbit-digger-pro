use rd_interface::{config::NetRef, prelude::*, registry::Builder, INet, Net, Result};

pub struct CombineNet {
    tcp_connect: Net,
    tcp_bind: Net,
    udp_bind: Net,
    lookup_host: Net,
}

impl INet for CombineNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        self.tcp_connect.provide_tcp_connect()
    }

    fn provide_tcp_bind(&self) -> Option<&dyn rd_interface::TcpBind> {
        self.tcp_bind.provide_tcp_bind()
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        self.udp_bind.provide_udp_bind()
    }

    fn provide_lookup_host(&self) -> Option<&dyn rd_interface::LookupHost> {
        self.lookup_host.provide_lookup_host()
    }
}

/// CombineNet merges multiple nets into one.
#[rd_config]
#[derive(Debug)]
pub struct CombineNetConfig {
    tcp_connect: NetRef,
    tcp_bind: NetRef,
    udp_bind: NetRef,
    lookup_host: NetRef,
}

impl Builder<Net> for CombineNet {
    const NAME: &'static str = "combine";
    type Config = CombineNetConfig;
    type Item = Self;

    fn build(
        CombineNetConfig {
            tcp_connect,
            tcp_bind,
            udp_bind,
            lookup_host,
        }: Self::Config,
    ) -> Result<Self> {
        Ok(CombineNet {
            tcp_connect: tcp_connect.value_cloned(),
            tcp_bind: tcp_bind.value_cloned(),
            udp_bind: udp_bind.value_cloned(),
            lookup_host: lookup_host.value_cloned(),
        })
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
        let net1 = TestNet::new().into_dyn();
        let net2 = TestNet::new().into_dyn();
        let net3 = TestNet::new().into_dyn();
        let net = CombineNet {
            tcp_connect: net1.clone(),
            tcp_bind: net2,
            udp_bind: net3,
            lookup_host: net1,
        }
        .into_dyn();

        assert_net_provider(
            &net,
            ProviderCapability {
                tcp_connect: true,
                tcp_bind: true,
                udp_bind: true,
                lookup_host: true,
            },
        );
    }

    #[tokio::test]
    async fn test_combine_net() {
        let net1 = TestNet::new().into_dyn();
        let net2 = TestNet::new().into_dyn();
        let net3 = TestNet::new().into_dyn();
        let net = CombineNet {
            tcp_connect: net1.clone(),
            tcp_bind: net2.clone(),
            udp_bind: net3.clone(),
            lookup_host: net1.clone(),
        }
        .into_dyn();

        spawn_echo_server(&net, "127.0.0.1:26666").await;
        assert_echo(&net2, "127.0.0.1:26666").await;

        spawn_echo_server_udp(&net, "127.0.0.1:26666").await;
        assert_echo_udp(&net3, "127.0.0.1:26666").await;
    }
}
