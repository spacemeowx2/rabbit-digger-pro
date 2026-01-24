use std::{
    io,
    net::SocketAddr,
    task::{self, Poll},
};

use super::UdpPacket;
use futures::{ready, SinkExt};
use rd_interface::{Address, IUdpChannel};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::PollSender;

pub(super) struct BackChannel {
    to: SocketAddr,
    sender: PollSender<UdpPacket>,
    receiver: Receiver<(Vec<u8>, SocketAddr)>,
    flushing: bool,
}

impl BackChannel {
    pub(super) fn new(
        to: SocketAddr,
        sender: Sender<UdpPacket>,
        receiver: Receiver<(Vec<u8>, SocketAddr)>,
    ) -> BackChannel {
        BackChannel {
            to,
            sender: PollSender::new(sender),
            receiver,
            flushing: false,
        }
    }
}

impl IUdpChannel for BackChannel {
    fn poll_send_to(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut rd_interface::ReadBuf,
    ) -> Poll<io::Result<Address>> {
        let (item, addr) = match ready!(self.receiver.poll_recv(cx)) {
            Some(item) => item,
            None => {
                return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, "channel closed")))
            }
        };

        let to_copy = item.len().min(buf.remaining());
        buf.initialize_unfilled_to(to_copy)
            .copy_from_slice(&item[..to_copy]);
        buf.advance(to_copy);

        Poll::Ready(Ok(addr.into()))
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        target: &SocketAddr,
    ) -> Poll<io::Result<usize>> {
        loop {
            if self.flushing {
                ready!(self.sender.poll_flush_unpin(cx))
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
                self.flushing = false;
                return Poll::Ready(Ok(buf.len()));
            }

            ready!(self.sender.poll_ready_unpin(cx))
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            let to = self.to;

            self.sender
                .start_send_unpin(UdpPacket {
                    from: *target,
                    to,
                    data: buf.to_vec(),
                })
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            self.flushing = true;
        }
    }
}
