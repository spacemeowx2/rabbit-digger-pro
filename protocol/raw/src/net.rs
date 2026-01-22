use std::{
    net::{IpAddr, SocketAddrV4},
    str::FromStr,
};

use crate::{
    config::RawNetConfig,
    device,
    gateway::{GatewayDevice, MapTable},
    wrap::{TcpListenerWrap, TcpStreamWrap, UdpSocketWrap},
};
use parking_lot::Mutex as SyncMutex;
use rd_interface::{
    async_trait, registry::Builder, Address, Arc, Context, Error, INet, IntoDyn, Net, Result,
};
use tokio::sync::Mutex;
use tokio_smoltcp::{
    smoltcp::wire::{IpAddress, IpCidr},
    smoltcp::{iface::Config as IfaceConfig, wire::HardwareAddress},
    BufferSize, Net as SmoltcpNet, NetConfig,
};

pub(crate) struct NetParams {
    pub(crate) smoltcp_net: Arc<SmoltcpNet>,
    pub(crate) map: MapTable,
    pub(crate) ip_cidr: IpCidr,
}

pub struct RawNet {
    smoltcp_net: Arc<SmoltcpNet>,
    pub(crate) params: SyncMutex<Option<NetParams>>,
}

impl RawNet {
    fn new(config: RawNetConfig) -> Result<RawNet> {
        let ip_cidr = IpCidr::from_str(&config.ip_addr)
            .map_err(|_| Error::Other("Failed to parse ip_addr".into()))?;
        let ip_addr = match IpAddr::from(ip_cidr.address()) {
            IpAddr::V4(v4) => SocketAddrV4::new(v4, 1),
            IpAddr::V6(_) => return Err(Error::Other("RawNet only support IPv4".into())),
        };
        let gateway = config
            .gateway
            .as_ref()
            .map(|gateway| {
                IpAddress::from_str(gateway)
                    .map_err(|_| Error::Other("Failed to parse gateway".into()))
            })
            .transpose()?;
        let (ethernet_addr, device) = device::get_device(&config)?;

        let mut net_config = NetConfig::new(
            IfaceConfig::new(HardwareAddress::Ethernet(ethernet_addr)),
            ip_cidr,
            gateway.into_iter().collect(),
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

        let mut params = None;
        let smoltcp_net = if config.forward {
            let device = GatewayDevice::new(device, ethernet_addr, 100, ip_cidr, ip_addr);
            let map = device.get_map();
            let smoltcp_net = Arc::new(SmoltcpNet::new(device, net_config));

            params = Some(NetParams {
                smoltcp_net: smoltcp_net.clone(),
                map,
                ip_cidr,
            });
            smoltcp_net
        } else {
            Arc::new(SmoltcpNet::new(device, net_config))
        };

        Ok(RawNet {
            smoltcp_net,
            params: SyncMutex::new(params),
        })
    }
    pub(crate) fn get_params(&self) -> Option<NetParams> {
        self.params.lock().take()
    }
}

impl Builder<Net> for RawNet {
    const NAME: &'static str = "raw";

    type Config = RawNetConfig;
    type Item = RawNet;

    fn build(config: Self::Config) -> Result<Self> {
        RawNet::new(config)
    }
}

#[async_trait]
impl rd_interface::TcpConnect for RawNet {
    async fn tcp_connect(
        &self,
        _ctx: &mut Context,
        addr: &Address,
    ) -> Result<rd_interface::TcpStream> {
        let tcp = TcpStreamWrap::new(self.smoltcp_net.tcp_connect(addr.to_socket_addr()?).await?);

        Ok(tcp.into_dyn())
    }
}

#[async_trait]
impl rd_interface::TcpBind for RawNet {
    async fn tcp_bind(
        &self,
        _ctx: &mut Context,
        addr: &Address,
    ) -> Result<rd_interface::TcpListener> {
        let addr = addr.to_socket_addr()?;
        let listener = TcpListenerWrap(Mutex::new(self.smoltcp_net.tcp_bind(addr).await?), addr);

        Ok(listener.into_dyn())
    }
}

#[async_trait]
impl rd_interface::UdpBind for RawNet {
    async fn udp_bind(
        &self,
        _ctx: &mut Context,
        addr: &Address,
    ) -> Result<rd_interface::UdpSocket> {
        let udp = UdpSocketWrap::new(self.smoltcp_net.udp_bind(addr.to_socket_addr()?).await?);

        Ok(udp.into_dyn())
    }
}

impl INet for RawNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_tcp_bind(&self) -> Option<&dyn rd_interface::TcpBind> {
        Some(self)
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        Some(self)
    }
}
