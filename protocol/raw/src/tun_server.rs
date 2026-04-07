use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    str::FromStr,
};

use futures::ready;
use rd_interface::{
    async_trait, config::NetRef, prelude::*, rd_config, registry::Builder, Arc, Context, IServer,
    IntoAddress, Net, Result, Server,
};
use rd_std::ContextExt;
use tokio::select;
use tokio_smoltcp::{
    smoltcp::{
        iface::Config as IfaceConfig,
        wire::{HardwareAddress, IpAddress, IpCidr},
    },
    BufferSize, Net as SmoltcpNet, NetConfig, RawSocket, TcpListener,
};

use crate::{
    device,
    fake_ip::{generate_fake_response, FakeIpPool},
    forward::source,
    gateway::{GatewayDevice, MapTable},
    route_setup::{RouteSetupConfig, RouteState},
};

#[rd_config]
#[derive(Debug, Clone, Copy)]
pub enum DnsMode {
    #[serde(rename = "fake-ip")]
    FakeIp,
    #[serde(rename = "redir-host")]
    RedirHost,
}

impl Default for DnsMode {
    fn default() -> Self {
        DnsMode::FakeIp
    }
}

#[rd_config]
pub struct TunServerConfig {
    /// TUN device name (e.g. "utun100" on macOS, "tun-rdp" on Linux)
    pub device: String,
    /// IP CIDR for the TUN interface itself (e.g. "10.0.0.1/24").
    #[serde(default = "default_tun_ip")]
    pub ip_addr: String,
    /// Fake-IP pool CIDR range (e.g. "198.18.0.0/15"). Only used in fake-ip mode.
    #[serde(default = "default_fake_ip_range")]
    pub fake_ip_range: String,
    /// MTU (default 1500)
    #[serde(default = "default_mtu")]
    pub mtu: u16,
    /// DNS mode: fake-ip (intercept and respond with fake IPs) or
    /// redir-host (forward DNS through the net chain as normal UDP)
    #[serde(default)]
    pub dns_mode: DnsMode,
    /// Outbound proxy net — all intercepted traffic is forwarded through this.
    pub net: NetRef,
    /// Socket fwmark for outbound traffic to bypass TUN (Linux).
    /// The outbound net should set `mark` to the same value.
    #[serde(default = "default_fwmark")]
    pub fwmark: u32,
    /// Proxy server IPs to bypass TUN (prevent routing loop)
    #[serde(default)]
    pub bypass_ips: Vec<String>,
}

fn default_tun_ip() -> String {
    "10.0.0.1/24".to_string()
}

fn default_fake_ip_range() -> String {
    "198.18.0.0/15".to_string()
}

fn default_fwmark() -> u32 {
    255
}

fn default_mtu() -> u16 {
    1500
}

pub struct TunServer {
    config: TunServerConfig,
    net: Net,
}

#[async_trait]
impl IServer for TunServer {
    async fn start(&self) -> Result<()> {
        let config = &self.config;

        // Parse TUN interface IP
        let ip_cidr = IpCidr::from_str(&config.ip_addr)
            .map_err(|_| rd_interface::Error::other("Invalid ip_addr"))?;
        let ip_addr = match IpAddr::from(ip_cidr.address()) {
            IpAddr::V4(v4) => v4,
            _ => return Err(rd_interface::Error::other("TUN only supports IPv4")),
        };
        let gateway = ip_addr;

        // 1. Create TUN device via tun crate (gets the fd for smoltcp)
        let raw_config = crate::config::RawNetConfig {
            device: crate::config::MaybeString::Other(crate::config::TunTapConfig {
                tuntap: crate::config::TunTap::Tun,
                name: Some(config.device.clone()),
                host_addr: ip_addr.to_string(),
            }),
            gateway: Some(gateway.to_string()),
            ip_addr: config.ip_addr.clone(),
            ethernet_addr: None,
            mtu: config.mtu as usize,
            forward: true,
        };

        let (ethernet_addr, tun_device) = device::get_device(&raw_config)
            .map_err(|e| rd_interface::Error::other(format!("Failed to create TUN: {e}")))?;

        // 2. Set up smoltcp network stack with GatewayDevice.
        //    Use 0.0.0.0/0 as the gateway CIDR so ALL incoming traffic is rewritten.
        let gateway_cidr = IpCidr::new(IpAddress::v4(0, 0, 0, 0), 0);
        let override_addr = SocketAddrV4::new(ip_addr, 1);
        let gw_device = GatewayDevice::new(
            tun_device,
            ethernet_addr,
            65536,
            gateway_cidr,
            override_addr,
        );
        let map = gw_device.get_map();

        let hw_addr = HardwareAddress::Ip; // TUN is L3
        let gw_ip = IpAddress::from_str(&gateway.to_string()).ok();
        let mut net_config = NetConfig::new(
            IfaceConfig::new(hw_addr),
            ip_cidr,
            gw_ip.into_iter().collect(),
        );
        net_config.buffer_size = BufferSize {
            tcp_rx_size: 65536,
            tcp_tx_size: 65536,
            udp_rx_size: 65536,
            udp_tx_size: 65536,
            udp_rx_meta_size: 256,
            udp_tx_meta_size: 256,
            ..Default::default()
        };
        let smoltcp_net = Arc::new(SmoltcpNet::new(gw_device, net_config));

        // 3. Set up system routes.
        //    Linux: policy routing with fwmark (not fwmark → TUN table)
        //    macOS: route replace + scoped fallback
        let _route_state = RouteState::setup(RouteSetupConfig {
            tun_name: config.device.clone(),
            tun_gateway: gateway.to_string(),
            fwmark: config.fwmark,
            dns_ip: gateway.to_string(),
        })
        .map_err(|e| rd_interface::Error::other(format!("Route setup failed: {e}")))?;

        tracing::info!(
            "TUN global mode active: device={}, dns={:?}",
            config.device,
            config.dns_mode
        );

        // 4. Initialize fake IP pool
        let fake_ip = Arc::new(FakeIpPool::new(&config.fake_ip_range));

        // 5. Bind listeners and serve traffic
        let tcp_listener = smoltcp_net
            .tcp_bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 1).into())
            .await
            .map_err(|e| rd_interface::Error::other(format!("TCP bind: {e}")))?;

        let raw_socket = smoltcp_net
            .raw_socket(
                tokio_smoltcp::smoltcp::wire::IpVersion::Ipv4,
                tokio_smoltcp::smoltcp::wire::IpProtocol::Udp,
            )
            .await
            .map_err(|e| rd_interface::Error::other(format!("UDP raw: {e}")))?;

        let handler = TunHandler {
            net: self.net.clone(),
            map,
            ip_cidr,
            fake_ip,
            dns_mode: config.dns_mode,
        };

        let tcp_task = handler.serve_tcp(tcp_listener);
        let udp_task = handler.serve_udp(raw_socket);

        select! {
            r = tcp_task => r?,
            r = udp_task => r?,
        };

        // _tproxy_state dropped here → routes cleaned up
        Ok(())
    }
}

struct TunHandler {
    net: Net,
    map: MapTable,
    ip_cidr: IpCidr,
    fake_ip: Arc<FakeIpPool>,
    dns_mode: DnsMode,
}

impl TunHandler {
    async fn serve_tcp(&self, mut listener: TcpListener) -> Result<()> {
        loop {
            let (tcp, addr) = listener.accept().await?;
            let orig_addr = self.map.get(&match addr {
                SocketAddr::V4(v4) => v4,
                _ => continue,
            });

            if let Some(orig_addr) = orig_addr {
                let net = self.net.clone();
                let fake_ip = self.fake_ip.clone();

                tokio::spawn(async move {
                    let ctx = &mut Context::from_socketaddr(addr);

                    let target_addr = if fake_ip.contains(&IpAddr::V4(*orig_addr.ip())) {
                        if let Some(domain) = fake_ip.lookup(*orig_addr.ip()) {
                            (domain.as_str(), orig_addr.port()).into_address()?
                        } else {
                            SocketAddr::from(orig_addr).into_address()?
                        }
                    } else {
                        SocketAddr::from(orig_addr).into_address()?
                    };

                    let target = net.tcp_connect(ctx, &target_addr).await?;
                    tracing::debug!("Bridging TUN TCP {} ↔ {}", addr, target_addr);
                    match ctx
                        .connect_tcp(rd_interface::TcpStream::from(tcp), target)
                        .await
                    {
                        Ok(()) => tracing::debug!("TCP bridge closed normally for {}", addr),
                        Err(e) => tracing::warn!("TCP bridge error for {}: {e}", addr),
                    }
                    Ok(()) as Result<()>
                });
            }
        }
    }

    async fn serve_udp(&self, raw: RawSocket) -> Result<()> {
        let dns_source = DnsInterceptSource {
            inner: source::Source::new(raw, self.ip_cidr),
            fake_ip: self.fake_ip.clone(),
            dns_mode: self.dns_mode,
        };

        rd_std::util::forward_udp::forward_udp(dns_source, self.net.clone(), None).await?;
        Ok(())
    }
}

/// Wraps the raw UDP source to intercept DNS queries in fake-ip mode.
struct DnsInterceptSource {
    inner: source::Source,
    fake_ip: Arc<FakeIpPool>,
    dns_mode: DnsMode,
}

impl rd_std::util::forward_udp::RawUdpSource for DnsInterceptSource {
    fn poll_recv(
        &mut self,
        cx: &mut std::task::Context<'_>,
        buf: &mut rd_interface::ReadBuf,
    ) -> std::task::Poll<std::io::Result<rd_std::util::forward_udp::UdpEndpoint>> {
        loop {
            let endpoint = ready!(self.inner.poll_recv(cx, buf))?;

            if endpoint.to.port() == 53 {
                if let DnsMode::FakeIp = self.dns_mode {
                    let dns_request = buf.filled();
                    if let Some(response) = generate_fake_response(&self.fake_ip, dns_request) {
                        let reply_endpoint = rd_std::util::forward_udp::UdpEndpoint {
                            from: endpoint.to,
                            to: endpoint.from,
                        };
                        buf.clear();

                        if let std::task::Poll::Ready(_) =
                            self.inner.poll_send(cx, &response, &reply_endpoint)
                        {
                            tracing::debug!("Fake DNS response for query from {}", endpoint.from);
                        }
                        continue;
                    }
                }
            }

            return std::task::Poll::Ready(Ok(endpoint));
        }
    }

    fn poll_send(
        &mut self,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
        endpoint: &rd_std::util::forward_udp::UdpEndpoint,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.inner.poll_send(cx, buf, endpoint)
    }
}

impl TunServer {
    fn new(config: TunServerConfig) -> Result<Self> {
        let net = config.net.value_cloned();
        Ok(TunServer { config, net })
    }
}

impl Builder<Server> for TunServer {
    const NAME: &'static str = "tun";
    type Config = TunServerConfig;
    type Item = TunServer;

    fn build(config: Self::Config) -> Result<Self> {
        TunServer::new(config)
    }
}
