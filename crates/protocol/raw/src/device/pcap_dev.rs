use std::io;

use crate::config::RawNetConfig;
use pcap::{Capture, Device};
use rd_interface::{Error, ErrorContext, Result};
use tokio_smoltcp::{
    device::{AsyncDevice, DeviceCapabilities},
    smoltcp::{phy::Checksum, wire::IpCidr},
};

pub(super) fn get_filter(ip_cidr: &IpCidr) -> Option<String> {
    match ip_cidr {
        IpCidr::Ipv4(v4) => Some(format!("net {}", v4.network())),
        _ => None,
    }
}

pub(super) fn pcap_device_by_name(name: &str) -> Result<Device> {
    let mut devices = Device::list().context("Failed to list device")?;

    if let Some(id) = devices.iter().position(|d| d.name == name) {
        Ok(devices.remove(id))
    } else {
        Err(Error::Other(
            format!("Failed to find device: {}", name,).into(),
        ))
    }
}

#[cfg(unix)]
pub(super) fn get_by_device(
    device: Device,
    filter: Option<String>,
    config: &RawNetConfig,
) -> Result<impl AsyncDevice> {
    use tokio_smoltcp::device::AsyncCapture;

    let mut cap = Capture::from_device(device)
        .context("Failed to capture device")?
        .promisc(true)
        .immediate_mode(true)
        .timeout(500)
        .open()
        .context("Failed to open device")?;

    if let Some(filter) = filter {
        cap.filter(&filter, true).context("Failed to add filter")?;
    }

    fn map_err(e: pcap::Error) -> io::Error {
        match e {
            pcap::Error::IoError(e) => e.into(),
            pcap::Error::TimeoutExpired => io::ErrorKind::WouldBlock.into(),
            other => io::Error::new(io::ErrorKind::Other, other),
        }
    }
    let mut caps = DeviceCapabilities::default();
    caps.max_transmission_unit = config.mtu;
    caps.checksum.ipv4 = Checksum::Tx;
    caps.checksum.tcp = Checksum::Tx;
    caps.checksum.udp = Checksum::Tx;
    caps.checksum.icmpv4 = Checksum::Tx;
    caps.checksum.icmpv6 = Checksum::Tx;

    AsyncCapture::new(
        cap.setnonblock().context("Failed to set nonblock")?,
        |d| d.next().map_err(map_err).map(|p| p.to_vec()),
        |d, pkt| d.sendpacket(pkt).map_err(map_err),
        caps,
    )
    .context("Failed to create async capture")
}

#[cfg(windows)]
fn get_by_device(
    device: Device,
    filter: Option<String>,
    config: &RawNetConfig,
) -> Result<impl AsyncDevice> {
    use tokio::sync::mpsc::{Receiver, Sender};
    use tokio_smoltcp::device::ChannelCapture;

    let mut cap = Capture::from_device(device.clone())
        .context("Failed to capture device")?
        .promisc(true)
        .immediate_mode(true)
        .timeout(5)
        .open()
        .context("Failed to open device")?;
    let mut send = Capture::from_device(device)
        .context("Failed to capture device")?
        .promisc(true)
        .immediate_mode(true)
        .timeout(5)
        .open()
        .context("Failed to open device")?;

    if let Some(filter) = filter {
        cap.filter(&filter, true).context("Failed to add filter")?;
    }
    // don't accept any packets from the send device
    send.filter("less 0", true)
        .context("Failed to add filter")?;

    let recv = move |tx: Sender<io::Result<Vec<u8>>>| loop {
        let p = match cap.next().map(|p| p.to_vec()) {
            Ok(p) => p,
            Err(pcap::Error::TimeoutExpired) => continue,
            Err(e) => {
                eprintln!("Error: {:?}", e);
                break;
            }
        };

        tx.blocking_send(Ok(p)).unwrap();
    };
    let send = move |mut rx: Receiver<Vec<u8>>| {
        while let Some(pkt) = rx.blocking_recv() {
            send.sendpacket(pkt).unwrap();
        }
    };
    let mut caps = DeviceCapabilities::default();
    caps.max_transmission_unit = config.mtu;
    caps.checksum.ipv4 = Checksum::Tx;
    caps.checksum.tcp = Checksum::Tx;
    caps.checksum.udp = Checksum::Tx;
    caps.checksum.icmpv4 = Checksum::Tx;
    caps.checksum.icmpv6 = Checksum::Tx;

    let capture = ChannelCapture::new(recv, send, caps);
    Ok(capture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_smoltcp::smoltcp::wire::Ipv4Address;

    #[test]
    fn test_get_filter() {
        let filter = get_filter(&IpCidr::new(Ipv4Address::new(192, 168, 1, 1).into(), 24));

        assert_eq!(filter, Some("net 192.168.1.0/24".to_string()));
    }
}
