use std::{
    io,
    net::SocketAddr,
    task::{self, Poll},
};

use crate::types::{Command, Object};

use crate::session::ClientSession;
use futures::ready;
use rd_interface::{
    async_trait, impl_async_read_write, Address, ITcpListener, ITcpStream, IUdpSocket, IntoDyn,
    Result,
};
use rd_std::util::async_fn::{AsyncFnIO, AsyncFnRead, AsyncFnWrite, PollAsyncFn};

pub struct TcpListenerWrapper {
    conn: ClientSession,
    obj: Object,
}

impl TcpListenerWrapper {
    pub fn new(conn: ClientSession, obj: Object) -> Self {
        Self { conn, obj }
    }
}

impl Drop for TcpListenerWrapper {
    fn drop(&mut self) {
        self.conn.close_object(self.obj)
    }
}

#[async_trait]
impl ITcpListener for TcpListenerWrapper {
    async fn accept(&self) -> Result<(rd_interface::TcpStream, SocketAddr)> {
        let (resp, _) = self
            .conn
            .send(Command::Accept(self.obj), None)
            .await?
            .wait()
            .await?;
        let (obj, addr) = resp.into_object_value()?;

        Ok((TcpWrapper::new(self.conn.clone(), obj).into_dyn(), addr))
    }

    async fn local_addr(&self) -> Result<SocketAddr> {
        let (resp, _) = self
            .conn
            .send(Command::LocalAddr(self.obj), None)
            .await?
            .wait()
            .await?;
        resp.into_value()
    }
}

pub struct UdpWrapper {
    conn: ClientSession,
    obj: Object,

    next_fut: PollAsyncFn<io::Result<(Vec<u8>, SocketAddr)>>,
    send_fut: PollAsyncFn<io::Result<()>>,
}

impl UdpWrapper {
    pub fn new(conn: ClientSession, obj: Object) -> Self {
        UdpWrapper {
            conn,
            obj,

            next_fut: PollAsyncFn::new(),
            send_fut: PollAsyncFn::new(),
        }
    }
}

impl Drop for UdpWrapper {
    fn drop(&mut self) {
        self.conn.close_object(self.obj)
    }
}

#[async_trait]
impl IUdpSocket for UdpWrapper {
    async fn local_addr(&self) -> Result<SocketAddr> {
        let (resp, _) = self
            .conn
            .send(Command::LocalAddr(self.obj), None)
            .await?
            .wait()
            .await?;
        resp.into_value()
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut rd_interface::ReadBuf,
    ) -> Poll<io::Result<SocketAddr>> {
        let UdpWrapper {
            next_fut,
            conn,
            obj,
            ..
        } = &mut *self;

        let (data, from) = ready!(next_fut.poll_next(cx, || {
            let conn = conn.clone();
            let obj = *obj;
            let fut = async move {
                let (resp, data) = conn
                    .send(Command::RecvFrom(obj), None)
                    .await?
                    .wait()
                    .await?;

                Ok((data, resp.into_value()?))
            };
            Box::pin(fut)
        }))?;

        let to_copy = data.len().min(buf.remaining());
        buf.initialize_unfilled_to(to_copy)
            .copy_from_slice(&data[..to_copy]);
        buf.advance(to_copy);

        Poll::Ready(Ok(from))
    }

    fn poll_send_to(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        target: &Address,
    ) -> Poll<io::Result<usize>> {
        let UdpWrapper {
            send_fut,
            conn,
            obj,
            ..
        } = &mut *self;

        ready!(send_fut.poll_next(cx, || {
            let conn = conn.clone();
            let obj = *obj;
            let addr = target.clone();
            let data = buf.to_vec();
            Box::pin(async move {
                let (resp, _) = conn
                    .send(Command::SendTo(obj, addr), Some(data))
                    .await?
                    .wait()
                    .await?;
                resp.into_null()?;
                Ok(())
            })
        }))?;

        Poll::Ready(Ok(buf.len()))
    }
}

#[derive(Clone)]
pub struct TcpAsyncFn {
    conn: ClientSession,
    obj: Object,
}

impl TcpWrapper {
    pub fn new(conn: ClientSession, obj: Object) -> TcpWrapper {
        TcpWrapper(AsyncFnIO::new(TcpAsyncFn { conn, obj }))
    }
}

impl Drop for TcpWrapper {
    fn drop(&mut self) {
        self.0.get_ref().conn.close_object(self.0.get_ref().obj)
    }
}

pub struct TcpWrapper(AsyncFnIO<TcpAsyncFn>);

#[async_trait]
impl AsyncFnRead for TcpAsyncFn {
    async fn read(&mut self, buf_size: usize) -> io::Result<Vec<u8>> {
        let getter = self
            .conn
            .send(Command::Read(self.obj, buf_size as u32), None)
            .await?;

        let (resp, data) = getter.wait().await?;
        resp.into_null()?;
        Ok(data)
    }
}

#[async_trait]
impl AsyncFnWrite for TcpAsyncFn {
    async fn write(&mut self, buf: Vec<u8>) -> io::Result<usize> {
        let getter = self.conn.send(Command::Write(self.obj), Some(buf)).await?;

        let (resp, _) = getter.wait().await?;
        let size = resp.into_value::<u32>()?;

        Ok(size as usize)
    }

    async fn flush(&mut self) -> io::Result<()> {
        let getter = self.conn.send(Command::Flush(self.obj), None).await?;

        getter.wait().await?;

        Ok(())
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        let getter = self.conn.send(Command::Shutdown(self.obj), None).await?;

        getter.wait().await?;

        Ok(())
    }
}

#[async_trait]
impl ITcpStream for TcpWrapper {
    async fn peer_addr(&self) -> Result<std::net::SocketAddr> {
        let this = self.0.get_ref();
        let (resp, _) = this
            .conn
            .send(Command::PeerAddr(this.obj), None)
            .await?
            .wait()
            .await?;
        resp.into_value()
    }

    async fn local_addr(&self) -> Result<std::net::SocketAddr> {
        let this = self.0.get_ref();
        let (resp, _) = this
            .conn
            .send(Command::LocalAddr(this.obj), None)
            .await?
            .wait()
            .await?;
        resp.into_value()
    }

    impl_async_read_write!(0);
}
