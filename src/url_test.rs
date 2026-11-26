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
pub struct UrlTestNetConfig {
    selected: rd_interface::registry::NetRef,
    list: Vec<rd_interface::registry::NetRef>,
    #[serde(default = "default_test_url")]
    url: String,
    #[serde(default)]
    interval: u64,
    #[serde(default)]
    tolerance: u64,
    #[serde(default = "default_test_timeout")]
    test_timeout: u64,
    #[serde(default = "default_max_failed_times")]
    max_failed_times: u32,
}

pub struct UrlTestNet {
    inner: AutoSelectCore,
}

impl UrlTestNet {
    pub fn new(config: UrlTestNetConfig) -> Result<Self> {
        Ok(Self {
            inner: AutoSelectCore::new(
                AutoSelectMode::UrlTest,
                config.selected,
                config.list,
                config.url,
                config.interval,
                config.tolerance,
                config.test_timeout,
                config.max_failed_times,
            )?,
        })
    }
}

#[async_trait]
impl INet for UrlTestNet {
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
impl TcpConnect for UrlTestNet {
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
impl TcpBind for UrlTestNet {
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
impl UdpBind for UrlTestNet {
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
impl rd_interface::LookupHost for UrlTestNet {
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

impl Builder<Net> for UrlTestNet {
    const NAME: &'static str = "url-test";
    type Config = UrlTestNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        UrlTestNet::new(config)
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<UrlTestNet>();
    Ok(())
}

#[cfg(test)]
mod tests {
    use rd_interface::{registry::NetRef, Error, IntoAddress, IntoDyn};
    use rd_std::tests::{assert_net_provider, spawn_echo_server, ProviderCapability, TestNet};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
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

    async fn assert_stream_echo(stream: &mut TcpStream, payload: &[u8]) {
        stream.write_all(payload).await.unwrap();
        stream.flush().await.unwrap();
        let mut buf = vec![0u8; payload.len()];
        stream.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, payload);
    }

    #[test]
    fn test_provider() {
        let net = NetRef::new_with_value("test".into(), TestNet::new().into_dyn());
        let url_test = UrlTestNet::new(UrlTestNetConfig {
            selected: net.clone(),
            list: vec![net],
            url: "http://127.0.0.1:80/".to_string(),
            interval: 0,
            tolerance: 0,
            test_timeout: DEFAULT_TEST_TIMEOUT_MS,
            max_failed_times: DEFAULT_MAX_FAILED_TIMES,
        })
        .unwrap()
        .into_dyn();

        assert_net_provider(
            &url_test,
            ProviderCapability {
                tcp_connect: true,
                tcp_bind: true,
                udp_bind: true,
                lookup_host: true,
            },
        );
    }

    #[tokio::test]
    async fn test_url_test_net_prefers_lower_delay() {
        let fast = delay_net("fast", Duration::from_millis(5));
        let slow = delay_net("slow", Duration::from_millis(40));
        spawn_echo_server(&fast.value_cloned(), "127.0.0.1:18080").await;
        spawn_echo_server(&slow.value_cloned(), "127.0.0.1:18080").await;

        let net = UrlTestNet::new(UrlTestNetConfig {
            selected: slow.clone(),
            list: vec![slow.clone(), fast.clone()],
            url: "http://127.0.0.1:18080/".to_string(),
            interval: 60,
            tolerance: 0,
            test_timeout: DEFAULT_TEST_TIMEOUT_MS,
            max_failed_times: DEFAULT_MAX_FAILED_TIMES,
        })
        .unwrap();

        assert_eq!(net.inner.current_index().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_url_test_net_respects_tolerance() {
        let current = delay_net("current", Duration::from_millis(20));
        let challenger = delay_net("challenger", Duration::from_millis(10));
        spawn_echo_server(&current.value_cloned(), "127.0.0.1:18081").await;
        spawn_echo_server(&challenger.value_cloned(), "127.0.0.1:18081").await;

        let net = UrlTestNet::new(UrlTestNetConfig {
            selected: current.clone(),
            list: vec![current.clone(), challenger.clone()],
            url: "http://127.0.0.1:18081/".to_string(),
            interval: 60,
            tolerance: 15,
            test_timeout: DEFAULT_TEST_TIMEOUT_MS,
            max_failed_times: DEFAULT_MAX_FAILED_TIMES,
        })
        .unwrap();

        assert_eq!(net.inner.current_index().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_url_test_net_failed_request_does_not_retry_but_refreshes_health() {
        let current_alive = Arc::new(AtomicBool::new(true));
        let backup_alive = Arc::new(AtomicBool::new(true));
        let current = toggle_net("current", current_alive.clone(), Duration::from_millis(5));
        let backup = toggle_net("backup", backup_alive, Duration::from_millis(20));
        spawn_echo_server(&current.value_cloned(), "127.0.0.1:18084").await;
        spawn_echo_server(&backup.value_cloned(), "127.0.0.1:18084").await;

        let net = UrlTestNet::new(UrlTestNetConfig {
            selected: current.clone(),
            list: vec![current.clone(), backup.clone()],
            url: "http://127.0.0.1:18084/".to_string(),
            interval: 60,
            tolerance: 0,
            test_timeout: DEFAULT_TEST_TIMEOUT_MS,
            max_failed_times: DEFAULT_MAX_FAILED_TIMES,
        })
        .unwrap();

        assert_eq!(net.inner.current_index().await.unwrap(), 0);
        current_alive.store(false, Ordering::SeqCst);

        let mut ctx = Context::new();
        let err = net
            .tcp_connect(&mut ctx, &("127.0.0.1", 18084).into_address().unwrap())
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
    async fn test_url_test_net_routes_through_selected_proxy() {
        let fast = delay_net("fast", Duration::from_millis(5));
        let slow = delay_net("slow", Duration::from_millis(40));
        spawn_echo_server(&fast.value_cloned(), "127.0.0.1:18083").await;
        spawn_echo_server(&slow.value_cloned(), "127.0.0.1:18083").await;

        let net = UrlTestNet::new(UrlTestNetConfig {
            selected: slow.clone(),
            list: vec![slow.clone(), fast.clone()],
            url: "http://127.0.0.1:18083/".to_string(),
            interval: 60,
            tolerance: 0,
            test_timeout: DEFAULT_TEST_TIMEOUT_MS,
            max_failed_times: DEFAULT_MAX_FAILED_TIMES,
        })
        .unwrap()
        .into_dyn();

        let mut ctx = Context::new();
        let mut stream = net
            .tcp_connect(&mut ctx, &("127.0.0.1", 18083).into_address().unwrap())
            .await
            .unwrap();
        assert_stream_echo(&mut stream, b"hello").await;
    }
}
