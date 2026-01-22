use std::{net::Ipv4Addr, str::FromStr};

use crate::config::{DeviceConfig, RawNetConfig, TunTap, TunTapSetup};
use boxed::BoxedAsyncDevice;
#[allow(unused_imports)]
pub use interface_info::get_interface_info;
use rd_interface::{Error, Result};
use tokio_smoltcp::smoltcp::wire::{EthernetAddress, IpCidr};

mod boxed;
mod interface_info;
#[cfg(unix)]
mod unix;
#[cfg(unix)]
use crate::device::unix::get_tun;

#[cfg(feature = "libpcap")]
mod pcap_dev;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use crate::device::windows::get_tun;

pub fn get_device(config: &RawNetConfig) -> Result<(EthernetAddress, BoxedAsyncDevice)> {
    let destination_addr = IpCidr::from_str(&config.ip_addr)
        .map_err(|_| Error::Other("Failed to parse server_addr".into()))?;

    let (ethernet_address, device) = match &config.device {
        #[cfg(feature = "libpcap")]
        DeviceConfig::String(dev) => {
            use rd_interface::ErrorContext;

            let interface_info = match crate::device::get_interface_info(dev) {
                Ok(info) => info,
                Err(e) => {
                    tracing::debug!(
                        "Failed to get interface info: {:?}, try to find by friendly name",
                        e
                    );
                    // find by friendly name
                    let interface = pcap::Device::list()
                        .context("Failed to get device list")?
                        .into_iter()
                        .map(|d| crate::device::get_interface_info(&d.name))
                        .flat_map(Result::ok)
                        .find(|i| i.friendly_name.as_ref() == Some(dev));

                    interface.ok_or_else(|| {
                        Error::Other(format!("Failed to find the interface: {}", dev).into())
                    })?
                }
            };

            let device = Box::new(pcap_dev::get_by_device(
                pcap_dev::pcap_device_by_name(&interface_info.name)?,
                pcap_dev::get_filter(&destination_addr),
                config,
            )?);

            (interface_info.ethernet_address, BoxedAsyncDevice(device))
        }
        DeviceConfig::Other(cfg) => {
            let host_addr = Ipv4Addr::from_str(&cfg.host_addr)
                .map_err(|_| Error::Other("Failed to parse host_addr".into()))?;

            let setup = TunTapSetup {
                name: cfg.name.clone(),
                addr: host_addr,
                destination_addr,
                mtu: config.mtu,
                layer: cfg.tuntap.into(),
            };
            let device = Box::new(get_tun(setup)?);
            let ethernet_addr = match cfg.tuntap {
                TunTap::Tun => EthernetAddress::BROADCAST,
                #[cfg(unix)]
                TunTap::Tap => {
                    use rd_interface::ErrorContext;

                    crate::device::get_interface_info(device.name())
                        .context("Failed to get interface info")?
                        .ethernet_address
                }
                #[cfg(windows)]
                TunTap::Tap => unreachable!(),
            };

            (ethernet_addr, BoxedAsyncDevice(device))
        }
    };

    Ok((ethernet_address, device))
}
