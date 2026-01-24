use std::{pin::Pin, task::Poll};

use futures::{future::poll_fn, ready};
use rd_interface::{
    async_trait, constant::UDP_BUFFER_SIZE, Address, Arc, AsyncRead, AsyncWrite, Context, IServer,
    Net, ReadBuf, Result, TcpStream,
};
use serde_json::to_value;
use tokio::{select, sync::Notify};

use crate::{
    connection::Codec,
    session::{Obj, RequestGetter, ServerSession},
    types::{Command, RpcValue},
};

#[derive(Clone)]
pub struct RpcServer {
    listen: Net,
    net: Net,
    bind: Address,
    stopper: Arc<Notify>,
    codec: Codec,
}
impl RpcServer {
    pub fn new(listen: Net, net: Net, bind: Address, codec: Codec) -> RpcServer {
        RpcServer {
            listen,
            net,
            bind,
            stopper: Arc::new(Notify::new()),
            codec,
        }
    }
}

struct Guard<F>(Option<F>)
where
    F: FnOnce();

impl<F> Guard<F>
where
    F: FnOnce(),
{
    fn new(f: F) -> Self {
        Guard(Some(f))
    }
}

impl<F> Drop for Guard<F>
where
    F: FnOnce(),
{
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f()
        }
    }
}

#[async_trait]
impl IServer for RpcServer {
    async fn start(&self) -> Result<()> {
        let listener = self
            .listen
            .tcp_bind(&mut Context::new(), &self.bind)
            .await?;
        let _guard = Guard::new(|| self.stopper.notify_waiters());

        loop {
            let (conn, _) = listener.accept().await?;
            let this = self.clone();
            tokio::spawn(async move {
                let result = select! {
                    r = this.handle_conn(conn) => r,
                    _ = this.stopper.notified() => Ok(()),
                };
                if let Err(e) = result {
                    tracing::error!("handle_conn {:?}", e);
                }
            });
        }
    }
}

impl RpcServer {
    async fn handle_req(&self, req: &RequestGetter) -> Result<(RpcValue, Option<Vec<u8>>)> {
        match req.cmd() {
            Command::TcpConnect(ctx, addr) => {
                let mut ctx = Context::from_value(ctx.clone())?;
                let tcp = self.net.tcp_connect(&mut ctx, addr).await?;

                Ok((
                    RpcValue::ObjectValue(req.insert_object(Obj::TcpStream(tcp)), ctx.to_value()),
                    None,
                ))
            }
            Command::TcpBind(ctx, addr) => {
                let mut ctx = Context::from_value(ctx.clone())?;
                let listener = self.net.tcp_bind(&mut ctx, addr).await?;

                Ok((
                    RpcValue::ObjectValue(
                        req.insert_object(Obj::TcpListener(listener)),
                        ctx.to_value(),
                    ),
                    None,
                ))
            }
            Command::UdpBind(ctx, addr) => {
                let mut ctx = Context::from_value(ctx.clone())?;
                let udp = self.net.udp_bind(&mut ctx, addr).await?;

                Ok((
                    RpcValue::ObjectValue(req.insert_object(Obj::UdpSocket(udp)), ctx.to_value()),
                    None,
                ))
            }
            Command::Close(obj) => {
                req.remove_object(*obj);

                Ok((RpcValue::Null, None))
            }
            Command::LookupHost(addr) => {
                let addrs = self.net.lookup_host(addr).await?;

                Ok((RpcValue::Value(to_value(addrs)?), None))
            }
            Command::RecvFrom(obj) => {
                let obj = req.get_object(*obj)?;
                let mut buf = [0; UDP_BUFFER_SIZE];

                poll_fn(move |cx| {
                    let mut udp = ready!(obj.poll_lock(cx));
                    let mut udp = Pin::new(udp.udp_socket_mut()?);

                    let mut read_buf = ReadBuf::new(&mut buf);
                    let from = ready!(udp.poll_recv_from(cx, &mut read_buf))?;

                    Poll::Ready(Ok((
                        RpcValue::Value(to_value(from)?),
                        Some(read_buf.filled().to_vec()),
                    )))
                })
                .await
            }
            Command::SendTo(obj, addr) => {
                let obj = req.get_object(*obj)?;
                let data = req.data();

                poll_fn(move |cx| {
                    let mut udp = ready!(obj.poll_lock(cx));
                    let udp = udp.udp_socket_mut()?;

                    ready!(udp.poll_send_to(cx, data, addr))?;

                    Poll::Ready(Ok((RpcValue::Null, None)))
                })
                .await
            }
            Command::Read(obj, buf_size) => {
                let obj = req.get_object(*obj)?;
                let mut buf = vec![0u8; *buf_size as usize];

                poll_fn(move |cx| {
                    let mut tcp = ready!(obj.poll_lock(cx));
                    let tcp = Pin::new(tcp.tcp_stream_mut()?);
                    let mut read_buf = ReadBuf::new(&mut buf);
                    ready!(tcp.poll_read(cx, &mut read_buf))?;
                    let filled = read_buf.filled().len();

                    Poll::Ready(Ok((RpcValue::Null, Some(buf[..filled].to_vec()))))
                })
                .await
            }
            Command::Write(obj) => {
                let obj = req.get_object(*obj)?;

                poll_fn(|cx| {
                    let mut tcp = ready!(obj.poll_lock(cx));
                    let tcp = Pin::new(tcp.tcp_stream_mut()?);
                    let write = ready!(tcp.poll_write(cx, req.data()))?;

                    Poll::Ready(Ok((RpcValue::Value(to_value(write)?), None)))
                })
                .await
            }
            Command::Flush(obj) => {
                let obj = req.get_object(*obj)?;

                poll_fn(|cx| {
                    let mut tcp = ready!(obj.poll_lock(cx));
                    let tcp = Pin::new(tcp.tcp_stream_mut()?);
                    ready!(tcp.poll_flush(cx))?;

                    Poll::Ready(Ok((RpcValue::Null, None)))
                })
                .await
            }
            Command::Shutdown(obj) => {
                let obj = req.get_object(*obj)?;

                poll_fn(|cx| {
                    let mut tcp = ready!(obj.poll_lock(cx));
                    let tcp = Pin::new(tcp.tcp_stream_mut()?);
                    ready!(tcp.poll_shutdown(cx))?;

                    Poll::Ready(Ok((RpcValue::Null, None)))
                })
                .await
            }
            Command::Accept(obj) => {
                let obj = req.get_object(*obj)?;
                let obj = obj.lock().await;
                let (tcp, addr) = obj.tcp_listener()?.accept().await?;

                Ok((
                    RpcValue::ObjectValue(req.insert_object(Obj::TcpStream(tcp)), to_value(addr)?),
                    None,
                ))
            }
            Command::LocalAddr(obj) => {
                let obj = req.get_object(*obj)?;
                let obj = obj.lock().await;
                let addr = match &*obj {
                    Obj::TcpStream(tcp) => tcp.local_addr().await,
                    Obj::TcpListener(listener) => listener.local_addr().await,
                    Obj::UdpSocket(udp) => udp.local_addr().await,
                }?;

                Ok((RpcValue::Value(to_value(addr)?), None))
            }
            Command::PeerAddr(obj) => {
                let obj = req.get_object(*obj)?;
                let obj = obj.lock().await;
                let addr = match &*obj {
                    Obj::TcpStream(tcp) => tcp.peer_addr().await,
                    _ => return Err(rd_interface::Error::other("invalid object")),
                }?;

                Ok((RpcValue::Value(to_value(addr)?), None))
            }
            _ => Err(rd_interface::Error::other("Invalid command")),
        }
    }
    async fn handle_conn(&self, tcp: TcpStream) -> Result<()> {
        let sess = ServerSession::new(tcp, self.codec);
        let handshake_req = sess.recv().await?;
        // TODO: handle session_id
        let _session_id = match handshake_req.cmd() {
            Command::Handshake(session_id) => session_id,
            _ => return Err(rd_interface::Error::other("Invalid handshake")),
        };
        handshake_req.response(Ok(RpcValue::Null), None).await?;

        let notify = Arc::new(Notify::new());
        let on_close = close_callback(sess.clone(), notify.clone());
        let _guard = Guard::new(on_close.clone());

        let e = loop {
            let req = match sess.recv().await {
                Ok(req) => req,
                Err(e) => break e,
            };
            let this = self.clone();
            let notify = notify.clone();
            let on_close = on_close.clone();
            tokio::spawn(async move {
                let result = select! {
                    r = this.handle_req(&req) => r,
                    _ = notify.notified() => return Err(rd_interface::Error::other("Connection closed")),
                };
                let (result, data) = match result {
                    Ok((result, data)) => (Ok(result), data),
                    Err(e) => (Err(e.to_string()), None),
                };

                let result = match data {
                    Some(data) => req.response(result, Some(data)).await,
                    None => req.response(result, None).await,
                };

                if let Err(e) = result {
                    tracing::error!("Server send error: {:?}", e);
                    on_close();
                }

                Ok(())
            });
        };

        tracing::error!("RPC server error: {:?}", e);

        Ok(())
    }
}

fn close_callback(sess: ServerSession, notify: Arc<Notify>) -> impl Fn() + Clone {
    return move || {
        let sess = sess.clone();
        tokio::spawn(async move { sess.close().await });
        notify.notify_waiters()
    };
}
