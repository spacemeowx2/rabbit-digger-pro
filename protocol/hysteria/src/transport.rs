use std::{
    fmt,
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{ready, Context, Poll},
};

use quinn::{udp::RecvMeta, AsyncUdpSocket, UdpPoller};
use tokio::io::Interest;

use crate::salamander;

pub(crate) fn make_quinn_socket(
    bind: SocketAddr,
    salamander_key: Option<Arc<Vec<u8>>>,
) -> io::Result<Arc<dyn AsyncUdpSocket>> {
    let std_sock = std::net::UdpSocket::bind(bind)?;
    std_sock.set_nonblocking(true)?;
    let io = tokio::net::UdpSocket::from_std(std_sock)?;
    Ok(Arc::new(Hy2UdpSocket {
        io: Arc::new(io),
        salamander_key,
    }))
}

struct Hy2UdpSocket {
    io: Arc<tokio::net::UdpSocket>,
    salamander_key: Option<Arc<Vec<u8>>>,
}

impl fmt::Debug for Hy2UdpSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hy2UdpSocket")
            .field("local_addr", &self.io.local_addr())
            .field("salamander", &self.salamander_key.as_ref().map(|_| "<set>"))
            .finish()
    }
}

impl AsyncUdpSocket for Hy2UdpSocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        Box::pin(TokioWritablePoller {
            io: self.io.clone(),
            fut: None,
        })
    }

    fn try_send(&self, transmit: &quinn::udp::Transmit<'_>) -> io::Result<()> {
        if transmit.segment_size.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "segment_size not supported",
            ));
        }

        self.io.try_io(Interest::WRITABLE, || {
            if let Some(key) = &self.salamander_key {
                let payload = salamander::encode_packet(key.as_slice(), transmit.contents);
                self.io
                    .try_send_to(&payload, transmit.destination)
                    .map(|_| ())
            } else {
                self.io
                    .try_send_to(transmit.contents, transmit.destination)
                    .map(|_| ())
            }
        })
    }

    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [io::IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        if bufs.is_empty() || meta.is_empty() {
            return Poll::Ready(Ok(0));
        }

        loop {
            ready!(self.io.poll_recv_ready(cx))?;
            let res = self.io.try_io(Interest::READABLE, || {
                let buf0 = bufs[0].as_mut();
                self.io.try_recv_from(buf0)
            });

            match res {
                Ok((n, addr)) => {
                    let mut len = n;
                    if let Some(key) = &self.salamander_key {
                        if n < 16 {
                            continue;
                        }
                        let buf0 = bufs[0].as_mut();
                        let used = &mut buf0[..n];
                        match salamander::decode_in_place(key.as_slice(), used) {
                            Some(new_len) => len = new_len,
                            None => continue,
                        }
                    }

                    meta[0].addr = addr;
                    meta[0].len = len;
                    meta[0].stride = len;
                    meta[0].ecn = None;
                    meta[0].dst_ip = None;
                    return Poll::Ready(Ok(1));
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => return Poll::Ready(Err(e)),
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.local_addr()
    }
}

struct TokioWritablePoller {
    io: Arc<tokio::net::UdpSocket>,
    fut: Option<Pin<Box<dyn Future<Output = io::Result<()>> + Send + Sync>>>,
}

impl fmt::Debug for TokioWritablePoller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokioWritablePoller").finish()
    }
}

impl UdpPoller for TokioWritablePoller {
    fn poll_writable(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.fut.is_none() {
            let io = this.io.clone();
            this.fut = Some(Box::pin(async move { io.writable().await }));
        }
        let fut = this.fut.as_mut().unwrap();
        match fut.as_mut().poll(cx) {
            Poll::Ready(r) => {
                this.fut = None;
                Poll::Ready(r)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
