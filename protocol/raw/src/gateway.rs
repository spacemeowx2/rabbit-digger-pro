use std::{
    net::SocketAddrV4,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use crate::config::Layer;
use futures::{ready, Sink, SinkExt, Stream, StreamExt};
use lru_time_cache::LruCache;
use parking_lot::Mutex;
use rd_interface::Result;
use smoltcp::wire::{
    EthernetAddress, EthernetFrame, EthernetProtocol, IpProtocol, Ipv4Packet, TcpPacket, UdpPacket,
};
use tokio_smoltcp::{
    device::{AsyncDevice, DeviceCapabilities, Packet},
    smoltcp::{
        self,
        wire::{ArpOperation, ArpPacket, IpCidr},
    },
};

#[derive(Debug)]
enum Action {
    Pass,
    Rewrite,
    Drop,
}

pub struct GatewayDevice<I> {
    inner: I,
    map: MapTable,

    ethernet_addr: EthernetAddress,

    ip_cidr: IpCidr,
    override_v4: SocketAddrV4,
    layer: Layer,
}

impl<I> GatewayDevice<I>
where
    I: AsyncDevice,
{
    pub fn new(
        inner: I,
        ethernet_addr: EthernetAddress,
        lru_size: usize,
        ip_cidr: IpCidr,
        override_v4: SocketAddrV4,
    ) -> GatewayDevice<I> {
        let layer = inner.capabilities().medium.into();
        GatewayDevice {
            inner,
            ethernet_addr,
            map: MapTable::new(lru_size),
            ip_cidr,
            override_v4,
            layer,
        }
    }
    pub fn get_map(&self) -> MapTable {
        self.map.clone()
    }
    // get ip packet by self.layer
    fn payload(&self, mut packet: Packet, f: impl FnOnce(Ipv4Packet<&mut [u8]>)) -> Packet {
        let cb = |payload_mut: &mut [u8]| Ipv4Packet::new_checked(payload_mut).map(f).ok();

        match self.layer {
            Layer::L2 => {
                let mut frame = match EthernetFrame::new_checked(&mut packet) {
                    Ok(p) => p,
                    Err(_) => return packet,
                };

                match frame.ethertype() {
                    EthernetProtocol::Ipv4 => cb(frame.payload_mut()),
                    _ => return packet,
                }
            }
            Layer::L3 => cb(&mut packet[..]),
        };

        packet
    }
    fn accept_packet(&self, packet: &Packet) -> Action {
        let ip_cidr = self.ip_cidr;
        let process_ip = |payload_mut: &[u8]| {
            Ipv4Packet::new_checked(payload_mut)
                .map(|f| {
                    let src = f.src_addr().into();
                    let dst = f.dst_addr().into();
                    if get_src_addr(&f) == Some(self.override_v4) {
                        return Action::Rewrite;
                    }
                    if ip_cidr.address() == src || ip_cidr.address() == dst {
                        Action::Pass
                    } else if ip_cidr.contains_addr(&src) || ip_cidr.contains_addr(&dst) {
                        Action::Rewrite
                    } else {
                        Action::Drop
                    }
                })
                .unwrap_or(Action::Drop)
        };

        match self.layer {
            Layer::L2 => {
                let frame = match EthernetFrame::new_checked(&packet) {
                    Ok(p) => p,
                    Err(_) => return Action::Drop,
                };

                let src = frame.src_addr();
                let dst = frame.dst_addr();
                let l2_accept =
                    dst.is_broadcast() || src == self.ethernet_addr || dst == self.ethernet_addr;
                if !l2_accept {
                    return Action::Drop;
                }

                match frame.ethertype() {
                    EthernetProtocol::Ipv4 => process_ip(frame.payload()),
                    EthernetProtocol::Arp => {
                        if src == self.ethernet_addr && dst.is_broadcast() {
                            Action::Rewrite
                        } else {
                            Action::Pass
                        }
                    }
                    _ => Action::Pass,
                }
            }
            Layer::L3 => process_ip(&packet[..]),
        }
    }
    fn map_in(&self, packet: Packet) -> Option<Packet> {
        match self.accept_packet(&packet) {
            Action::Pass => Some(packet),
            Action::Rewrite => Some(self.payload(packet, |mut ipv4| {
                if let Ok(Some((src_addr, ori_addr))) = set_dst_addr(&mut ipv4, self.override_v4) {
                    self.map.insert(src_addr, ori_addr);
                }
            })),
            Action::Drop => None,
        }
    }
    fn map_out(&self, mut packet: Packet) -> Option<Packet> {
        match self.accept_packet(&packet) {
            Action::Pass => Some(packet),
            Action::Rewrite => {
                let _ = self.correct_arp_request(&mut packet);

                Some(self.payload(packet, |mut ipv4| {
                    if let Some(src) = get_dst_addr(&mut ipv4).map(|d| self.map.get(&d)).flatten() {
                        set_src_addr(&mut ipv4, src).ok();
                    }
                }))
            }
            Action::Drop => None,
        }
    }

    fn correct_arp_request(&self, packet: &mut Vec<u8>) -> smoltcp::wire::Result<()> {
        if let Layer::L2 = self.layer {
            // SAFETY: we know that the packet is a valid EthernetFrame
            let mut frame = EthernetFrame::new_unchecked(packet);
            if frame.ethertype() == EthernetProtocol::Arp {
                let mut arp_packet = ArpPacket::new_checked(frame.payload_mut())?;
                if arp_packet.operation() == ArpOperation::Request
                    && arp_packet.source_hardware_addr() == self.ethernet_addr.as_bytes()
                {
                    arp_packet.set_source_protocol_addr(&self.override_v4.ip().octets()[..]);
                }
            }
        };
        Ok(())
    }
}

impl<I> Stream for GatewayDevice<I>
where
    I: AsyncDevice,
{
    type Item = I::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            let item = match ready!(self.inner.poll_next_unpin(cx)) {
                Some(Ok(item)) => self.map_in(item),
                Some(r) => return Poll::Ready(Some(r)),
                None => return Poll::Ready(None),
            };
            if let Some(item) = item {
                return Poll::Ready(Some(Ok(item)));
            }
        }
    }
}

impl<I> Sink<Packet> for GatewayDevice<I>
where
    I: AsyncDevice,
{
    type Error = I::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready_unpin(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Packet) -> Result<(), Self::Error> {
        if let Some(item) = self.map_out(item) {
            self.inner.start_send_unpin(item)
        } else {
            Ok(())
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_flush_unpin(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_close_unpin(cx)
    }
}

impl<I> AsyncDevice for GatewayDevice<I>
where
    I: AsyncDevice,
{
    fn capabilities(&self) -> &DeviceCapabilities {
        self.inner.capabilities()
    }
}

#[derive(Clone)]
pub struct MapTable {
    map: Arc<Mutex<LruCache<SocketAddrV4, SocketAddrV4>>>,
}

impl MapTable {
    fn new(cap: usize) -> MapTable {
        MapTable {
            map: Arc::new(Mutex::new(LruCache::with_capacity(cap))),
        }
    }
    fn insert(&self, key: SocketAddrV4, value: SocketAddrV4) {
        self.map.lock().insert(key, value);
    }
    pub fn get(&self, key: &SocketAddrV4) -> Option<SocketAddrV4> {
        self.map.lock().get(key).copied()
    }
}

fn set_dst_addr<T: AsRef<[u8]> + AsMut<[u8]>>(
    ip: &mut Ipv4Packet<T>,
    dst_addr: SocketAddrV4,
) -> smoltcp::wire::Result<Option<(SocketAddrV4, SocketAddrV4)>> {
    let src_addr = ip.src_addr();
    let orig_addr = ip.dst_addr();

    let (src_port, orig_port) = match ip.next_header() {
        IpProtocol::Tcp => {
            ip.set_dst_addr(dst_addr.ip().to_owned().into());

            let mut tcp = TcpPacket::new_unchecked(ip.payload_mut());
            let dst_port = tcp.dst_port();
            tcp.set_dst_port(dst_addr.port());

            (tcp.src_port(), dst_port)
        }
        _ => return Ok(None),
    };

    Ok(Some((
        SocketAddrV4::new(src_addr.into(), src_port),
        SocketAddrV4::new(orig_addr.into(), orig_port),
    )))
}

fn get_src_addr<T: AsRef<[u8]> + ?Sized>(ip: &Ipv4Packet<&T>) -> Option<SocketAddrV4> {
    let src_addr = ip.src_addr();
    let port = match ip.next_header() {
        IpProtocol::Tcp => TcpPacket::new_checked(ip.payload()).ok()?.src_port(),
        IpProtocol::Udp => UdpPacket::new_checked(ip.payload()).ok()?.src_port(),
        _ => return None,
    };
    Some(SocketAddrV4::new(src_addr.into(), port))
}

fn get_dst_addr<T: AsRef<[u8]> + AsMut<[u8]>>(ip: &mut Ipv4Packet<T>) -> Option<SocketAddrV4> {
    let dst_addr = ip.dst_addr();
    let port = match ip.next_header() {
        IpProtocol::Tcp => TcpPacket::new_checked(ip.payload_mut()).ok()?.dst_port(),
        IpProtocol::Udp => UdpPacket::new_checked(ip.payload_mut()).ok()?.dst_port(),
        _ => return None,
    };
    Some(SocketAddrV4::new(dst_addr.into(), port))
}

fn set_src_addr<T: AsRef<[u8]> + AsMut<[u8]>>(
    ip: &mut Ipv4Packet<T>,
    src_addr_v4: SocketAddrV4,
) -> smoltcp::wire::Result<()> {
    let src_addr = src_addr_v4.ip().to_owned().into();
    let dst_addr = ip.dst_addr();
    let port = src_addr_v4.port();
    if let IpProtocol::Tcp = ip.next_header() {
        ip.set_src_addr(src_addr);

        let mut tcp = TcpPacket::new_checked(ip.payload_mut())?;
        tcp.set_src_port(port);

        tcp.fill_checksum(&src_addr.into(), &dst_addr.into());
    };
    ip.fill_checksum();

    Ok(())
}
