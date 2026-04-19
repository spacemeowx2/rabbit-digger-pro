use rd_interface::prelude::*;
use rd_interface::{
    async_trait, registry::Builder, Address, Context, INet, Net, Registry, Result, TcpBind,
    TcpConnect, TcpListener, TcpStream, UdpBind, UdpSocket,
};

use crate::auto_select::{default_test_url, AutoSelectCore, AutoSelectMode};

#[rd_config]
#[derive(Debug, Clone)]
pub struct FallbackNetConfig {
    selected: rd_interface::registry::NetRef,
    list: Vec<rd_interface::registry::NetRef>,
    #[serde(default = "default_test_url")]
    url: String,
    #[serde(default)]
    interval: u64,
}

pub struct FallbackNet {
    inner: AutoSelectCore,
}

impl FallbackNet {
    pub fn new(config: FallbackNetConfig) -> Result<Self> {
        Ok(Self {
            inner: AutoSelectCore::new(
                AutoSelectMode::Fallback,
                config.selected,
                config.list,
                config.url,
                config.interval,
                0,
            )?,
        })
    }
}

#[async_trait]
impl INet for FallbackNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_tcp_bind(&self) -> Option<&dyn rd_interface::TcpBind> {
        Some(self)
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        Some(self)
    }

    fn provide_lookup_host(&self) -> Option<&dyn rd_interface::LookupHost> {
        Some(self)
    }
}

#[async_trait]
impl TcpConnect for FallbackNet {
    async fn tcp_connect(&self, ctx: &mut Context, addr: &Address) -> Result<TcpStream> {
        self.inner.current_net().await?.tcp_connect(ctx, addr).await
    }
}

#[async_trait]
impl TcpBind for FallbackNet {
    async fn tcp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<TcpListener> {
        self.inner.current_net().await?.tcp_bind(ctx, addr).await
    }
}

#[async_trait]
impl UdpBind for FallbackNet {
    async fn udp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<UdpSocket> {
        self.inner.current_net().await?.udp_bind(ctx, addr).await
    }
}

#[async_trait]
impl rd_interface::LookupHost for FallbackNet {
    async fn lookup_host(&self, addr: &Address) -> Result<Vec<std::net::SocketAddr>> {
        self.inner.current_net().await?.lookup_host(addr).await
    }
}

impl Builder<Net> for FallbackNet {
    const NAME: &'static str = "fallback";
    type Config = FallbackNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        FallbackNet::new(config)
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<FallbackNet>();
    Ok(())
}

#[cfg(test)]
mod tests {
    use rd_interface::{registry::NetRef, IntoDyn};
    use rd_std::tests::{spawn_echo_server, TestNet};
    use tokio::time::Duration;

    use super::*;

    struct DelayNet {
        inner: Net,
        delay: Duration,
    }

    impl DelayNet {
        fn new(inner: Net, delay: Duration) -> Self {
            Self { inner, delay }
        }
    }

    #[async_trait]
    impl rd_interface::TcpConnect for DelayNet {
        async fn tcp_connect(&self, ctx: &mut Context, addr: &Address) -> Result<TcpStream> {
            tokio::time::sleep(self.delay).await;
            self.inner.tcp_connect(ctx, addr).await
        }
    }

    #[async_trait]
    impl rd_interface::TcpBind for DelayNet {
        async fn tcp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<TcpListener> {
            self.inner.tcp_bind(ctx, addr).await
        }
    }

    #[async_trait]
    impl rd_interface::UdpBind for DelayNet {
        async fn udp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<UdpSocket> {
            self.inner.udp_bind(ctx, addr).await
        }
    }

    #[async_trait]
    impl rd_interface::LookupHost for DelayNet {
        async fn lookup_host(&self, addr: &Address) -> Result<Vec<std::net::SocketAddr>> {
            self.inner.lookup_host(addr).await
        }
    }

    #[async_trait]
    impl INet for DelayNet {
        fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
            Some(self)
        }
        fn provide_tcp_bind(&self) -> Option<&dyn rd_interface::TcpBind> {
            Some(self)
        }
        fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
            Some(self)
        }
        fn provide_lookup_host(&self) -> Option<&dyn rd_interface::LookupHost> {
            Some(self)
        }
    }

    fn delay_net(name: &str, delay: Duration) -> NetRef {
        let inner = TestNet::new().into_dyn();
        NetRef::new_with_value(name.into(), DelayNet::new(inner, delay).into_dyn())
    }

    #[tokio::test]
    async fn test_fallback_net_uses_first_healthy_proxy() {
        let dead = delay_net("dead", Duration::from_millis(0));
        let healthy = delay_net("healthy", Duration::from_millis(5));
        spawn_echo_server(&healthy.value_cloned(), "127.0.0.1:18082").await;

        let net = FallbackNet::new(FallbackNetConfig {
            selected: dead.clone(),
            list: vec![dead.clone(), healthy.clone()],
            url: "http://127.0.0.1:18082/".to_string(),
            interval: 60,
        })
        .unwrap();

        assert_eq!(net.inner.current_index().await.unwrap(), 1);
    }
}
