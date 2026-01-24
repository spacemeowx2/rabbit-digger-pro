use std::{collections::VecDeque, fmt, future::Future, io, pin::Pin, task::Context, task::Poll};

use rd_interface::{Address, ReadBuf, UdpChannel, UdpSocket};
use tracing::instrument;

use crate::util::DropAbort;

pub const UDP_BUFFER_SIZE: usize = 2 * 1024;

#[derive(Clone)]
#[must_use = "Don't waste memory by not using this"]
struct PacketBuffer {
    buf: Box<[u8]>,
    filled: Option<(usize, Address)>,
}

impl fmt::Debug for PacketBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PacketBuffer")
            .field(
                "buf",
                &self.filled.as_ref().map(|(len, _)| &self.buf[..*len]),
            )
            .field("filled", &self.filled)
            .finish()
    }
}

impl PacketBuffer {
    fn new() -> Self {
        Self {
            buf: vec![0; UDP_BUFFER_SIZE].into_boxed_slice(),
            filled: None,
        }
    }
    fn borrow(&self) -> Option<(&[u8], &Address)> {
        self.filled
            .as_ref()
            .map(|(len, addr)| (&self.buf[..*len], addr))
    }
}

struct CopyBuffer {
    pool: Vec<PacketBuffer>,
    queue: VecDeque<PacketBuffer>,
    max_buffer_size: usize,
}

impl CopyBuffer {
    fn new(buffer_size: usize, max_buffer_size: usize) -> Self {
        Self {
            pool: vec![PacketBuffer::new(); buffer_size],
            queue: VecDeque::with_capacity(buffer_size),
            max_buffer_size,
        }
    }
    fn borrow_front(&mut self) -> Option<&mut PacketBuffer> {
        self.queue.front_mut()
    }
    fn pop_front(&mut self) {
        if let Some(mut p) = self.queue.pop_front() {
            p.filled = None;
            self.pool.push(p);
        }
    }
    fn borrow_back(&mut self) -> Option<&mut PacketBuffer> {
        match self.queue.back().map(|i| i.filled.is_some()) {
            Some(true) | None => {
                self.grow();
                self.queue.push_back(self.pool.pop()?);
            }
            Some(false) => {}
        };
        self.queue.back_mut()
    }
    fn grow(&mut self) {
        if self.pool.is_empty() {
            let goal = self.max_buffer_size.min(self.queue.len() * 2);
            let to_add = goal - self.queue.len();
            self.pool.append(&mut vec![PacketBuffer::new(); to_add]);
        }
    }
}

struct CopyBidirectional {
    a: UdpChannel,
    b: UdpSocket,

    send_queue: CopyBuffer,
    recv_queue: CopyBuffer,
}

impl CopyBidirectional {
    fn poll_send_to(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let CopyBidirectional {
            a, b, send_queue, ..
        } = self;
        let mut registered = false;

        loop {
            while let Some(packet) = send_queue.borrow_back() {
                let mut buf = ReadBuf::new(&mut packet.buf);
                match a.poll_send_to(cx, &mut buf) {
                    Poll::Ready(addr) => {
                        packet.filled = Some((buf.filled().len(), addr?));
                    }
                    Poll::Pending => {
                        registered = true;
                        break;
                    }
                }
            }
            while let Some(packet) = send_queue.borrow_front() {
                let result = packet
                    .borrow()
                    .map(|(data, addr)| b.poll_send_to(cx, data, addr));
                match result {
                    Some(Poll::Ready(r)) => {
                        r?;
                        send_queue.pop_front();
                    }
                    Some(Poll::Pending) => {
                        registered = true;
                        break;
                    }
                    None => break,
                }
            }
            if registered {
                return Poll::Pending;
            }
        }
    }
    fn poll_recv_from(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let CopyBidirectional {
            a, b, recv_queue, ..
        } = self;
        let mut registered = false;

        loop {
            while let Some(packet) = recv_queue.borrow_back() {
                let mut buf = ReadBuf::new(&mut packet.buf);
                match b.poll_recv_from(cx, &mut buf) {
                    Poll::Ready(addr) => {
                        packet.filled = Some((buf.filled().len(), addr?.into()));
                    }
                    Poll::Pending => {
                        registered = true;
                        break;
                    }
                }
            }
            while let Some(packet) = recv_queue.borrow_front() {
                let result = packet.borrow().map(|(data, addr)| {
                    a.poll_recv_from(
                        cx,
                        data,
                        match addr {
                            Address::SocketAddr(s) => s,
                            _ => unreachable!(),
                        },
                    )
                });
                match result {
                    Some(Poll::Ready(r)) => {
                        r?;
                        recv_queue.pop_front();
                    }
                    Some(Poll::Pending) => {
                        registered = true;
                        break;
                    }
                    None => break,
                }
            }
            if registered {
                return Poll::Pending;
            }
        }
    }
}

impl Future for CopyBidirectional {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let a_to_b = self.poll_send_to(cx)?;
        let b_to_a = self.poll_recv_from(cx)?;

        match (a_to_b, b_to_a) {
            (Poll::Pending, Poll::Pending) => Poll::Pending,
            _ => Poll::Ready(Ok(())),
        }
    }
}

/// Connect two `TcpStream`. Unlike `copy_bidirectional`, it closes the other side once one side is done.
#[instrument(err, skip(a, b))]
pub async fn connect_udp(
    ctx: &mut rd_interface::Context,
    a: UdpChannel,
    b: UdpSocket,
) -> io::Result<()> {
    let _ = ctx;
    DropAbort::new(tokio::spawn(CopyBidirectional {
        a,
        b,
        send_queue: CopyBuffer::new(1, 16),
        recv_queue: CopyBuffer::new(1, 16),
    }))
    .await??;

    Ok(())
}
