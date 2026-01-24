use std::{
    io,
    net::{IpAddr, SocketAddr, SocketAddrV4},
    task::{self, Poll},
};

use futures::ready;
use rd_std::util::forward_udp::{self, RawUdpSource};
use tokio_smoltcp::{
    smoltcp::{
        self,
        phy::ChecksumCapabilities,
        wire::{IpCidr, IpProtocol, Ipv4Address, Ipv4Packet, Ipv4Repr, UdpPacket, UdpRepr},
    },
    RawSocket,
};

pub struct Source {
    raw: RawSocket,
    send_buf: Vec<u8>,
    ip_cidr: IpCidr,
}

impl Source {
    pub fn new(raw: RawSocket, ip_cidr: IpCidr) -> Source {
        Source {
            raw,
            send_buf: Vec::new(),
            ip_cidr,
        }
    }
}

impl RawUdpSource for Source {
    fn poll_recv(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut rd_interface::ReadBuf,
    ) -> Poll<io::Result<forward_udp::UdpEndpoint>> {
        let Source { raw, ip_cidr, .. } = &mut *self;

        let (from, to, data) = loop {
            let u8buf = buf.initialize_unfilled();
            let size = ready!(raw.poll_recv(cx, u8buf))?;

            if let Ok(v) = parse_udp(&u8buf[..size]) {
                let broadcast = match ip_cidr {
                    IpCidr::Ipv4(v4) => v4.broadcast().map(Into::into).map(IpAddr::V4),
                    _ => None,
                };

                let to = v.1;
                if broadcast == Some(to.ip())
                    || to.ip().is_multicast()
                    || to.ip() == IpAddr::from(ip_cidr.address())
                {
                    continue;
                }

                break v;
            };
        };

        buf.initialize_unfilled_to(data.len())
            .copy_from_slice(&data);
        buf.advance(data.len());

        Poll::Ready(Ok(forward_udp::UdpEndpoint { from, to }))
    }

    fn poll_send(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        endpoint: &forward_udp::UdpEndpoint,
    ) -> Poll<io::Result<()>> {
        if self.send_buf.is_empty() {
            if let Some(ip_packet) = pack_udp(endpoint.from, endpoint.to, buf) {
                self.send_buf = ip_packet;
            } else {
                tracing::debug!("Unsupported src/dst");
                return Poll::Ready(Ok(()));
            }
        }
        ready!(self.raw.poll_send(cx, &self.send_buf))?;
        self.send_buf.clear();
        Poll::Ready(Ok(()))
    }
}

/// buf is a ip packet
fn parse_udp(buf: &[u8]) -> smoltcp::wire::Result<(SocketAddr, SocketAddr, Vec<u8>)> {
    let ipv4 = Ipv4Packet::new_checked(buf)?;
    let udp = UdpPacket::new_checked(ipv4.payload())?;

    let src = SocketAddrV4::new(ipv4.src_addr().into(), udp.src_port());
    let dst = SocketAddrV4::new(ipv4.dst_addr().into(), udp.dst_port());

    // TODO: avoid to_vec
    Ok((src.into(), dst.into(), udp.payload().to_vec()))
}

fn pack_udp(src: SocketAddr, dst: SocketAddr, payload: &[u8]) -> Option<Vec<u8>> {
    match (src, dst) {
        (SocketAddr::V4(src_v4), SocketAddr::V4(dst_v4)) => {
            let checksum = &ChecksumCapabilities::default();
            let udp_repr = UdpRepr {
                src_port: src.port(),
                dst_port: dst.port(),
            };
            let ipv4_repr = Ipv4Repr {
                src_addr: Ipv4Address::from(*src_v4.ip()),
                dst_addr: Ipv4Address::from(*dst_v4.ip()),
                next_header: IpProtocol::Udp,
                payload_len: udp_repr.header_len() + payload.len(),
                hop_limit: 64,
            };

            let mut buffer =
                vec![0u8; ipv4_repr.buffer_len() + udp_repr.header_len() + payload.len()];

            let mut udp_packet = UdpPacket::new_unchecked(&mut buffer[ipv4_repr.buffer_len()..]);
            udp_repr.emit(
                &mut udp_packet,
                &src.ip().into(),
                &dst.ip().into(),
                payload.len(),
                |buf| buf.copy_from_slice(payload),
                checksum,
            );

            let mut ipv4_packet = Ipv4Packet::new_unchecked(&mut buffer);
            ipv4_repr.emit(&mut ipv4_packet, checksum);

            Some(buffer)
        }
        _ => None,
    }
}
