use rd_interface::{async_trait, impl_async_write, AsyncRead, ITcpStream, TcpStream};
use std::{
    collections::VecDeque,
    io,
    net::SocketAddr,
    pin::Pin,
    task::{self, Poll},
};
use tokio::io::{AsyncReadExt, ReadBuf};

pub struct PeekableTcpStream {
    tcp: TcpStream,
    buf: VecDeque<u8>,
}

#[async_trait]
impl ITcpStream for PeekableTcpStream {
    async fn peer_addr(&self) -> crate::Result<SocketAddr> {
        self.tcp.peer_addr().await
    }

    async fn local_addr(&self) -> crate::Result<SocketAddr> {
        self.tcp.local_addr().await
    }

    impl_async_write!(tcp);

    fn poll_read(&mut self, cx: &mut task::Context<'_>, buf: &mut ReadBuf) -> Poll<io::Result<()>> {
        let (first, ..) = &self.buf.as_slices();
        if !first.is_empty() {
            let read = first.len().min(buf.remaining());
            let unfilled = buf.initialize_unfilled_to(read);
            unfilled[0..read].copy_from_slice(&first[0..read]);
            buf.advance(read);

            // remove 0..read
            self.buf.drain(0..read);

            Poll::Ready(Ok(()))
        } else {
            Pin::new(&mut self.tcp).poll_read(cx, buf)
        }
    }
}

impl PeekableTcpStream {
    pub fn new(tcp: TcpStream) -> Self {
        PeekableTcpStream {
            tcp,
            buf: VecDeque::new(),
        }
    }
    // Fill self.buf to size using self.tcp.read_exact
    async fn fill_buf(&mut self, size: usize) -> crate::Result<()> {
        if size > self.buf.len() {
            let to_read = size - self.buf.len();
            let mut buf = vec![0u8; to_read];
            self.tcp.read_exact(&mut buf).await?;
            self.buf.append(&mut buf.into());
        }
        Ok(())
    }
    pub async fn peek_exact(&mut self, buf: &mut [u8]) -> crate::Result<()> {
        self.fill_buf(buf.len()).await?;
        let self_buf = self.buf.make_contiguous();
        buf.copy_from_slice(&self_buf[0..buf.len()]);

        Ok(())
    }
    pub fn into_inner(self) -> (TcpStream, VecDeque<u8>) {
        (self.tcp, self.buf)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rd_interface::{Context, IntoAddress, IntoDyn, Net};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        time::sleep,
    };

    use super::*;
    use crate::tests::TestNet;

    async fn spawn_listener(net: &Net) {
        let net = net.clone();
        tokio::spawn(async move {
            let listener = net
                .tcp_bind(
                    &mut Context::new(),
                    &"127.0.0.1:1234".into_address().unwrap(),
                )
                .await
                .unwrap();
            let (mut tcp, _) = listener.accept().await.unwrap();

            tcp.write_all(b"12345678").await.unwrap();
            let mut buf = Vec::new();
            tcp.read_to_end(&mut buf).await.unwrap();
            buf
        });

        sleep(Duration::from_millis(10)).await;
    }

    #[tokio::test]
    async fn test_peekable_tcp_stream() {
        let net = TestNet::new().into_dyn();
        spawn_listener(&net).await;

        let mut tcp = net
            .tcp_connect(
                &mut Context::new(),
                &"127.0.0.1:1234".into_address().unwrap(),
            )
            .await
            .map(PeekableTcpStream::new)
            .unwrap();

        let mut buf = [0u8; 4];
        tcp.peek_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"1234");

        let mut tcp = tcp.into_dyn();

        let mut buf = [0u8; 8];
        tcp.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"12345678");
        assert!(tcp.local_addr().await.is_ok());
        assert!(tcp.peer_addr().await.is_ok());
    }

    #[tokio::test]
    async fn test_peekable_tcp_stream_into_inner() {
        let net = TestNet::new().into_dyn();
        spawn_listener(&net).await;

        let mut tcp = net
            .tcp_connect(
                &mut Context::new(),
                &"127.0.0.1:1234".into_address().unwrap(),
            )
            .await
            .map(PeekableTcpStream::new)
            .unwrap();

        let mut buf = [0u8; 4];
        tcp.peek_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"1234");

        let (mut tcp, rest) = tcp.into_inner();
        assert_eq!(&rest, b"1234");
        tcp.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"5678");
    }
}
