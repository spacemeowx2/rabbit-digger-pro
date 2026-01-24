use rd_interface::{INet, Net};

/// A no-op Net returns [`Error::NotImplemented`](crate::Error::NotImplemented) for every method.
pub struct NotImplementedNet;

impl INet for NotImplementedNet {}

/// A new Net calls [`tcp_connect()`](crate::INet::tcp_connect()), [`tcp_bind()`](crate::INet::tcp_bind()), [`udp_bind()`](crate::INet::udp_bind()) from different Net.
pub struct CombineNet {
    pub tcp_connect: Net,
    pub tcp_bind: Net,
    pub udp_bind: Net,
    pub lookup_host: Net,
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

#[cfg(test)]
mod tests {
    use rd_interface::IntoDyn;
    use tokio::task::yield_now;

    use crate::tests::{
        assert_echo, assert_echo_udp, assert_net_provider, spawn_echo_server,
        spawn_echo_server_udp, ProviderCapability, TestNet,
    };

    use super::*;

    #[test]
    fn test_provider() {
        let tcp_connect = TestNet::new().into_dyn();
        let tcp_bind = TestNet::new().into_dyn();
        let udp_bind = TestNet::new().into_dyn();
        let lookup_host = TestNet::new().into_dyn();

        let net = CombineNet {
            tcp_connect,
            tcp_bind,
            udp_bind,
            lookup_host,
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
        let tcp_connect = TestNet::new().into_dyn();
        let tcp_bind = TestNet::new().into_dyn();
        let udp_bind = TestNet::new().into_dyn();
        let lookup_host = TestNet::new().into_dyn();
        let tcp_bind2 = tcp_bind.clone();
        spawn_echo_server(&tcp_connect, "127.0.0.1:12345").await;
        spawn_echo_server_udp(&udp_bind, "127.0.0.1:12346").await;

        yield_now().await;

        let net = CombineNet {
            tcp_connect,
            tcp_bind,
            udp_bind,
            lookup_host,
        }
        .into_dyn();

        assert_echo(&net, "127.0.0.1:12345").await;
        assert_echo_udp(&net, "127.0.0.1:12346").await;

        spawn_echo_server(&net, "127.0.0.1:12346").await;
        yield_now().await;
        assert_echo(&tcp_bind2, "127.0.0.1:12346").await;
    }
}
