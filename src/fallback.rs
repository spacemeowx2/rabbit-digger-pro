use rd_interface::prelude::*;
use rd_interface::{
    async_trait, registry::Builder, Address, Context, INet, Net, Registry, Result, TcpBind,
    TcpConnect, TcpListener, TcpStream, UdpBind, UdpSocket,
};

use crate::auto_select::{
    default_test_url, AutoSelectCore, AutoSelectMode, DEFAULT_MAX_FAILED_TIMES,
    DEFAULT_TEST_TIMEOUT_MS,
};

fn default_test_timeout() -> u64 {
    DEFAULT_TEST_TIMEOUT_MS
}

fn default_max_failed_times() -> u32 {
    DEFAULT_MAX_FAILED_TIMES
}

#[rd_config]
#[derive(Debug, Clone)]
pub struct FallbackNetConfig {
    selected: rd_interface::registry::NetRef,
    list: Vec<rd_interface::registry::NetRef>,
    #[serde(default = "default_test_url")]
    url: String,
    #[serde(default)]
    interval: u64,
    #[serde(default = "default_test_timeout")]
    test_timeout: u64,
    #[serde(default = "default_max_failed_times")]
    max_failed_times: u32,
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
                config.test_timeout,
                config.max_failed_times,
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
        let net = self.inner.current_net().await?;
        match net.tcp_connect(ctx, addr).await {
            Ok(stream) => {
                self.inner.on_operation_success().await;
                Ok(stream)
            }
            Err(err) => {
                self.inner.on_operation_failure(&err).await;
                Err(err)
            }
        }
    }
}

#[async_trait]
impl TcpBind for FallbackNet {
    async fn tcp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<TcpListener> {
        let net = self.inner.current_net().await?;
        match net.tcp_bind(ctx, addr).await {
            Ok(listener) => {
                self.inner.on_operation_success().await;
                Ok(listener)
            }
            Err(err) => {
                self.inner.on_operation_failure(&err).await;
                Err(err)
            }
        }
    }
}

#[async_trait]
impl UdpBind for FallbackNet {
    async fn udp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<UdpSocket> {
        let net = self.inner.current_net().await?;
        match net.udp_bind(ctx, addr).await {
            Ok(socket) => {
                self.inner.on_operation_success().await;
                Ok(socket)
            }
            Err(err) => {
                self.inner.on_operation_failure(&err).await;
                Err(err)
            }
        }
    }
}

#[async_trait]
impl rd_interface::LookupHost for FallbackNet {
    async fn lookup_host(&self, addr: &Address) -> Result<Vec<std::net::SocketAddr>> {
        let net = self.inner.current_net().await?;
        match net.lookup_host(addr).await {
            Ok(addrs) => {
                self.inner.on_operation_success().await;
                Ok(addrs)
            }
            Err(err) => {
                self.inner.on_operation_failure(&err).await;
                Err(err)
            }
        }
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
    use rd_interface::{registry::NetRef, Error, IntoAddress, IntoDyn};
    use rd_std::tests::{spawn_echo_server, TestNet};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
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

    struct ToggleNet {
        inner: Net,
        alive: Arc<AtomicBool>,
        delay: Duration,
    }

    impl ToggleNet {
        fn new(inner: Net, alive: Arc<AtomicBool>, delay: Duration) -> Self {
            Self {
                inner,
                alive,
                delay,
            }
        }

        fn ensure_alive(&self) -> Result<()> {
            if self.alive.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(Error::from(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "connection refused",
                )))
            }
        }
    }

    #[async_trait]
    impl rd_interface::TcpConnect for ToggleNet {
        async fn tcp_connect(&self, ctx: &mut Context, addr: &Address) -> Result<TcpStream> {
            self.ensure_alive()?;
            tokio::time::sleep(self.delay).await;
            self.inner.tcp_connect(ctx, addr).await
        }
    }

    #[async_trait]
    impl rd_interface::TcpBind for ToggleNet {
        async fn tcp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<TcpListener> {
            self.ensure_alive()?;
            self.inner.tcp_bind(ctx, addr).await
        }
    }

    #[async_trait]
    impl rd_interface::UdpBind for ToggleNet {
        async fn udp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<UdpSocket> {
            self.ensure_alive()?;
            self.inner.udp_bind(ctx, addr).await
        }
    }

    #[async_trait]
    impl rd_interface::LookupHost for ToggleNet {
        async fn lookup_host(&self, addr: &Address) -> Result<Vec<std::net::SocketAddr>> {
            self.ensure_alive()?;
            self.inner.lookup_host(addr).await
        }
    }

    #[async_trait]
    impl INet for ToggleNet {
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

    fn toggle_net(name: &str, alive: Arc<AtomicBool>, delay: Duration) -> NetRef {
        let inner = TestNet::new().into_dyn();
        NetRef::new_with_value(name.into(), ToggleNet::new(inner, alive, delay).into_dyn())
    }

    #[tokio::test]
    async fn test_fallback_failed_request_does_not_retry_but_refreshes_health() {
        let first_alive = Arc::new(AtomicBool::new(true));
        let second_alive = Arc::new(AtomicBool::new(true));
        let first = toggle_net("first", first_alive.clone(), Duration::from_millis(5));
        let second = toggle_net("second", second_alive, Duration::from_millis(10));
        spawn_echo_server(&first.value_cloned(), "127.0.0.1:18085").await;
        spawn_echo_server(&second.value_cloned(), "127.0.0.1:18085").await;

        let net = FallbackNet::new(FallbackNetConfig {
            selected: first.clone(),
            list: vec![first.clone(), second.clone()],
            url: "http://127.0.0.1:18085/".to_string(),
            interval: 60,
            test_timeout: DEFAULT_TEST_TIMEOUT_MS,
            max_failed_times: DEFAULT_MAX_FAILED_TIMES,
        })
        .unwrap();

        assert_eq!(net.inner.current_index().await.unwrap(), 0);
        first_alive.store(false, Ordering::SeqCst);

        let mut ctx = Context::new();
        let err = net
            .tcp_connect(&mut ctx, &("127.0.0.1", 18085).into_address().unwrap())
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("connection refused"));

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if net.inner.current_index().await.unwrap() == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
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
            test_timeout: DEFAULT_TEST_TIMEOUT_MS,
            max_failed_times: DEFAULT_MAX_FAILED_TIMES,
        })
        .unwrap();

        assert_eq!(net.inner.current_index().await.unwrap(), 1);
    }
}
