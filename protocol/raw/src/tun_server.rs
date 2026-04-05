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
};

#[rd_config]
#[serde(rename_all = "lowercase")]
#[derive(Debug, Clone, Copy)]
pub enum DnsMode {
    FakeIp,
    RedirHost,
}

impl Default for DnsMode {
    fn default() -> Self {
        DnsMode::FakeIp
    }
}

#[rd_config]
pub struct TunServerConfig {
    /// TUN device name (e.g. "utun100")
    pub device: String,
    /// IP CIDR for the TUN interface (e.g. "198.18.0.1/15")
    pub ip_addr: String,
    /// MTU (default 1500)
    #[serde(default = "default_mtu")]
    pub mtu: u16,
    /// DNS mode
    #[serde(default)]
    pub dns_mode: DnsMode,
    /// Outbound proxy net
    pub net: NetRef,
    /// Net for real DNS resolution (should bind to physical interface)
    #[serde(default)]
    pub resolve_net: Option<NetRef>,
    /// Proxy server IPs to bypass TUN (prevent routing loop)
    #[serde(default)]
    pub bypass_ips: Vec<String>,
}

fn default_mtu() -> u16 {
    1500
}

pub struct TunServer {
    config: TunServerConfig,
    net: Net,
    resolve_net: Option<Net>,
}

#[async_trait]
impl IServer for TunServer {
    async fn start(&self) -> Result<()> {
        let config = &self.config;

        // Parse IP config
        let ip_cidr = IpCidr::from_str(&config.ip_addr)
            .map_err(|_| rd_interface::Error::other("Invalid ip_addr"))?;
        let ip_addr = match IpAddr::from(ip_cidr.address()) {
            IpAddr::V4(v4) => v4,
            _ => return Err(rd_interface::Error::other("TUN only supports IPv4")),
        };
        let gateway = ip_addr; // TUN gateway is itself

        // Create TUN device
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

        // Set up smoltcp network stack with GatewayDevice
        let smoltcp_ip_cidr = ip_cidr;
        let override_addr = SocketAddrV4::new(ip_addr, 1);
        let gw_device = GatewayDevice::new(
            tun_device,
            ethernet_addr,
            65536,
            smoltcp_ip_cidr,
            override_addr,
        );
        let map = gw_device.get_map();

        let gw_ip = IpAddress::from_str(&gateway.to_string()).ok();
        let mut net_config = NetConfig::new(
            IfaceConfig::new(HardwareAddress::Ethernet(ethernet_addr)),
            smoltcp_ip_cidr,
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

        // Set up system routes via tproxy-config
        let bypass: Vec<tproxy_config::IpCidr> = config
            .bypass_ips
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();

        let tproxy_args = tproxy_config::TproxyArgs::new()
            .tun_name(&config.device)
            .tun_ip(IpAddr::V4(ip_addr))
            .tun_gateway(IpAddr::V4(gateway))
            .tun_dns(IpAddr::V4(gateway))
            .tun_mtu(config.mtu)
            .bypass_ips(&bypass)
            .ipv4_default_route(true)
            .ipv6_default_route(false);

        let _tproxy_state = tproxy_config::tproxy_setup(&tproxy_args)
            .map_err(|e| rd_interface::Error::other(format!("Route setup failed: {e}")))?;

        tracing::info!(
            "TUN global mode active: device={}, dns={:?}",
            config.device,
            config.dns_mode
        );

        // Initialize fake IP pool
        let fake_ip = Arc::new(FakeIpPool::new(&config.ip_addr));

        // Bind listeners
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

        // Serve traffic (blocks until server stops)
        let handler = TunHandler {
            net: self.net.clone(),
            map,
            ip_cidr: smoltcp_ip_cidr,
            fake_ip,
            dns_mode: config.dns_mode,
        };

        let tcp_task = handler.serve_tcp(tcp_listener);
        let udp_task = handler.serve_udp(raw_socket);

        select! {
            r = tcp_task => r?,
            r = udp_task => r?,
        };

        // _tproxy_state dropped here → routes automatically restored
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

                    // If destination is a fake IP, resolve back to domain
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
                    ctx.connect_tcp(rd_interface::TcpStream::from(tcp), target)
                        .await?;
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

/// A RawUdpSource wrapper that intercepts DNS queries (port 53)
/// and responds with fake IPs when in FakeIp mode.
/// Non-DNS packets are passed through to the inner source.
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

            // Intercept DNS queries (destination port 53)
            if endpoint.to.port() == 53 {
                if let DnsMode::FakeIp = self.dns_mode {
                    let dns_request = buf.filled();
                    if let Some(response) = generate_fake_response(&self.fake_ip, dns_request) {
                        // Clear the recv buffer and send response back via the inner source
                        let reply_endpoint = rd_std::util::forward_udp::UdpEndpoint {
                            from: endpoint.to,
                            to: endpoint.from,
                        };
                        // We need to send the response back through the raw socket.
                        // Use poll_send on inner to write the DNS response packet.
                        // For now, we do this synchronously-ish by storing it and
                        // continuing the poll loop.
                        buf.clear();

                        // Send DNS response back
                        if let std::task::Poll::Ready(_) =
                            self.inner.poll_send(cx, &response, &reply_endpoint)
                        {
                            tracing::debug!(
                                "Fake DNS response sent for query from {}",
                                endpoint.from
                            );
                        }
                        continue; // Don't forward to the net, loop for next packet
                    }
                }
                // redir-host mode or failed parse: forward DNS as normal UDP
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
        let resolve_net = config.resolve_net.as_ref().map(|r| r.value_cloned());
        Ok(TunServer {
            config,
            net,
            resolve_net,
        })
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
