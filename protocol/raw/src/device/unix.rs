use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    pin::Pin,
    task::{Context, Poll},
};

use crate::config::TunTapSetup;
use futures::{ready, Sink, SinkExt, Stream, StreamExt};
use rd_interface::{error::map_other, Result};
use tokio_smoltcp::{
    device::{AsyncDevice, DeviceCapabilities, Packet},
    smoltcp::{phy::Checksum, wire::IpAddress},
};
use tokio_util::codec::Framed;
use tun_crate::{create_as_async, AbstractDevice, Configuration, TunPacketCodec};

pub struct TunAsyncDevice {
    device_name: String,
    dev: Framed<tun_crate::AsyncDevice, TunPacketCodec>,
    caps: DeviceCapabilities,
}

pub fn get_tun(cfg: TunTapSetup) -> Result<TunAsyncDevice> {
    let mut config = Configuration::default();
    let netmask = !0u32 >> (32 - cfg.destination_addr.prefix_len());

    config
        .address(IpAddr::from(cfg.addr))
        .destination(match cfg.destination_addr.address() {
            IpAddress::Ipv4(v4) => IpAddr::from(Ipv4Addr::from(v4)),
            IpAddress::Ipv6(v6) => IpAddr::from(Ipv6Addr::from(v6)),
            _ => unreachable!(),
        })
        .netmask(netmask)
        .layer(cfg.layer.into())
        .up();

    if let Some(name) = &cfg.name {
        config.tun_name(name);
    }

    let dev = create_as_async(&config).map_err(map_other)?;
    let device_name = dev.tun_name().map_err(map_other)?;
    let dev = dev.into_framed();

    tracing::info!("tun created: {}", device_name);

    let mut caps = DeviceCapabilities::default();
    caps.medium = cfg.layer.into();
    caps.max_transmission_unit = cfg.mtu;
    caps.checksum.ipv4 = Checksum::Tx;
    caps.checksum.tcp = Checksum::Tx;
    caps.checksum.udp = Checksum::Tx;
    caps.checksum.icmpv4 = Checksum::Tx;
    caps.checksum.icmpv6 = Checksum::Tx;

    Ok(TunAsyncDevice {
        device_name,
        dev,
        caps,
    })
}

impl Stream for TunAsyncDevice {
    type Item = io::Result<Packet>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let p = ready!(self.dev.poll_next_unpin(cx));

        // TunPacketCodec yields Vec<u8>
        Poll::Ready(p.map(|i| i.map(|p| p)))
    }
}

impl Sink<Packet> for TunAsyncDevice {
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.dev.poll_ready_unpin(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Packet) -> Result<(), Self::Error> {
        self.dev.start_send_unpin(item)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.dev.poll_flush_unpin(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.dev.poll_close_unpin(cx)
    }
}

impl AsyncDevice for TunAsyncDevice {
    fn capabilities(&self) -> &DeviceCapabilities {
        &self.caps
    }
}

impl TunAsyncDevice {
    pub fn name(&self) -> &str {
        &self.device_name
    }
}
