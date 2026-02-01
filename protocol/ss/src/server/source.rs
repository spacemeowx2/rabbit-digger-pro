use std::{
    io,
    task::{self, Poll},
};

use bytes::BytesMut;
use futures::ready;
use rd_interface::UdpSocket;
use rd_std::util::forward_udp::{RawUdpSource, UdpEndpoint};
use shadowsocks::crypto::CipherKind;
use socks5_protocol::Address as S5Addr;

use crate::udp::{decrypt_payload, encrypt_payload};

pub struct UdpSource {
    udp: UdpSocket,

    method: CipherKind,
    key: Box<[u8]>,
    send_buf: BytesMut,
}

impl UdpSource {
    pub fn new(method: CipherKind, key: Box<[u8]>, udp: UdpSocket) -> UdpSource {
        UdpSource {
            udp,
            method,
            key,
            send_buf: BytesMut::new(),
        }
    }
}

impl RawUdpSource for UdpSource {
    fn poll_recv(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut rd_interface::ReadBuf,
    ) -> Poll<io::Result<UdpEndpoint>> {
        let packet = loop {
            let from = ready!(self.udp.poll_recv_from(cx, buf))?;
            let (n, addr) = match decrypt_payload(self.method, &self.key, buf.filled_mut()) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Failed to decode udp packet: {:?}", e);
                    continue;
                }
            };
            buf.set_filled(n);
            // drop the packet if it's sent to domain silently
            let to = match addr {
                S5Addr::Domain(_, _) => continue,
                S5Addr::SocketAddr(s) => s,
            };

            break UdpEndpoint { from, to };
        };

        Poll::Ready(Ok(packet))
    }

    fn poll_send(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        endpoint: &UdpEndpoint,
    ) -> Poll<io::Result<()>> {
        if self.send_buf.is_empty() {
            encrypt_payload(
                self.method,
                &self.key,
                &endpoint.from.into(),
                buf,
                &mut self.send_buf,
            )?;
        }

        ready!(self
            .udp
            .poll_send_to(cx, &self.send_buf, &endpoint.to.into()))?;
        self.send_buf.clear();

        Poll::Ready(Ok(()))
    }
}
