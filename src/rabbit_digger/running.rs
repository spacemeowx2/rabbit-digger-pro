use std::{
    fmt::Debug,
    io,
    mem::replace,
    net::SocketAddr,
    pin::Pin,
    task::{self, Poll},
};

use futures::{ready, FutureExt, TryFutureExt};
use parking_lot::RwLock as SyncRwLock;
use rd_interface::{
    async_trait,
    context::common_field::{DestDomain, DestSocketAddr},
    Address, AddressDomain, Arc, AsyncRead, AsyncWrite, Context, INet, IUdpSocket, IntoDyn, Net,
    ReadBuf, Result, Server, TcpListener, TcpStream, UdpSocket, Value,
};
use tokio::{
    sync::{oneshot, RwLock, Semaphore},
    task::JoinHandle,
};
use tracing::{instrument, Instrument};

use super::{
    connection::{Connection, ConnectionConfig},
    event::EventType,
};

pub struct RunningNet {
    name: String,
    net: SyncRwLock<Net>,
}

impl RunningNet {
    pub fn new(name: String, net: Net) -> Arc<RunningNet> {
        Arc::new(RunningNet {
            name,
            net: SyncRwLock::new(net),
        })
    }
    pub fn update_net(&self, net: Net) {
        *self.net.write() = net;
    }
    pub fn as_net(self: &Arc<Self>) -> Net {
        Net::from(self.clone() as Arc<dyn INet>)
    }
    fn net(&self) -> Net {
        self.net.read().clone()
    }
}

impl Debug for RunningNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningNet")
            .field("name", &self.name)
            .finish()
    }
}

#[async_trait]
impl INet for RunningNet {
    #[instrument(err)]
    async fn tcp_connect(&self, ctx: &mut Context, addr: &Address) -> Result<TcpStream> {
        ctx.append_net(&self.name);
        self.net().tcp_connect(ctx, addr).await
    }

    #[instrument(err)]
    async fn tcp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<TcpListener> {
        ctx.append_net(&self.name);
        self.net().tcp_bind(ctx, addr).await
    }

    #[instrument(err)]
    async fn udp_bind(&self, ctx: &mut Context, addr: &Address) -> Result<UdpSocket> {
        ctx.append_net(&self.name);
        self.net().udp_bind(ctx, addr).await
    }

    #[instrument(err)]
    async fn lookup_host(&self, addr: &Address) -> Result<Vec<SocketAddr>> {
        self.net().lookup_host(addr).await
    }

    fn get_inner(&self) -> Option<Net> {
        Some(self.net())
    }
}

pub struct RunningServerNet {
    server_name: String,
    net: Net,
    config: ConnectionConfig,
}

impl RunningServerNet {
    pub fn new(server_name: String, net: Net, config: ConnectionConfig) -> RunningServerNet {
        RunningServerNet {
            server_name,
            net,
            config,
        }
    }
}

impl Debug for RunningServerNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningServerNet")
            .field("server_name", &self.server_name)
            .finish()
    }
}

#[async_trait]
impl INet for RunningServerNet {
    #[instrument(err, skip(ctx))]
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> rd_interface::Result<TcpStream> {
        ctx.append_net(self.server_name.clone());
        // prepare context
        match addr {
            Address::Domain(domain, port) => ctx.insert_common(DestDomain(AddressDomain {
                domain: domain.to_string(),
                port: *port,
            }))?,
            Address::SocketAddr(addr) => ctx.insert_common(DestSocketAddr(*addr))?,
        };

        let tcp = self.net.tcp_connect(ctx, &addr).await?;

        tracing::info!(target: "rabbit_digger", ?ctx, "Connected");
        let tcp = WrapTcpStream::new(tcp, self.config.clone(), addr.clone(), ctx);
        Ok(tcp.into_dyn())
    }

    // TODO: wrap TcpListener
    #[instrument(err)]
    async fn tcp_bind(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> rd_interface::Result<TcpListener> {
        ctx.append_net(self.server_name.clone());

        self.net.tcp_bind(ctx, addr).await
    }

    #[instrument(err)]
    async fn udp_bind(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> rd_interface::Result<UdpSocket> {
        ctx.append_net(self.server_name.clone());

        let udp = WrapUdpSocket::new(
            self.net.udp_bind(ctx, addr).await?,
            self.config.clone(),
            addr.clone(),
            ctx,
        );
        Ok(udp.into_dyn())
    }

    #[instrument(err)]
    async fn lookup_host(&self, addr: &Address) -> Result<Vec<SocketAddr>> {
        self.net.lookup_host(addr).await
    }

    fn get_inner(&self) -> Option<Net> {
        Some(self.net.clone())
    }
}

pub struct WrapUdpSocket {
    inner: UdpSocket,
    conn: Connection,
    stopped: parking_lot::Mutex<oneshot::Receiver<()>>,
}

impl WrapUdpSocket {
    pub fn new(
        inner: UdpSocket,
        config: ConnectionConfig,
        addr: Address,
        ctx: &Context,
    ) -> WrapUdpSocket {
        let (sender, stopped) = oneshot::channel();
        WrapUdpSocket {
            inner,
            conn: Connection::new(config, EventType::NewUdp(addr, ctx.to_value(), sender)),
            stopped: parking_lot::Mutex::new(stopped),
        }
    }
}

#[async_trait]
impl IUdpSocket for WrapUdpSocket {
    async fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.local_addr().await
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<SocketAddr>> {
        let WrapUdpSocket {
            inner,
            conn,
            stopped,
        } = &mut *self;
        if let Poll::Ready(_) = stopped.lock().poll_unpin(cx) {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Aborted by user",
            )
            .into()));
        }
        let addr = ready!(inner.poll_recv_from(cx, buf)?);
        conn.send(EventType::UdpInbound(
            addr.into(),
            buf.filled().len() as u64,
        ));
        Poll::Ready(Ok(addr))
    }

    fn poll_send_to(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        target: &Address,
    ) -> Poll<io::Result<usize>> {
        let r = ready!(self.inner.poll_send_to(cx, buf, target))?;

        self.conn
            .send(EventType::UdpOutbound(target.clone(), buf.len() as u64));
        Poll::Ready(Ok(r))
    }
}

pub struct WrapTcpStream {
    inner: TcpStream,
    conn: Connection,
    stopped: oneshot::Receiver<()>,
}

impl WrapTcpStream {
    pub fn new(
        inner: TcpStream,
        config: ConnectionConfig,
        addr: Address,
        ctx: &Context,
    ) -> WrapTcpStream {
        let (sender, stopped) = oneshot::channel();
        WrapTcpStream {
            inner,
            conn: Connection::new(config, EventType::NewTcp(addr, ctx.to_value(), sender)),
            stopped,
        }
    }
}

#[async_trait]
impl rd_interface::ITcpStream for WrapTcpStream {
    async fn peer_addr(&self) -> Result<SocketAddr> {
        self.inner.peer_addr().await
    }

    async fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.local_addr().await
    }

    fn poll_read(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Poll::Ready(_) = self.stopped.poll_unpin(cx) {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Aborted by user",
            )));
        }
        let before = buf.filled().len();
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let s = buf.filled().len() - before;
                self.conn.send(EventType::Inbound(s as u64));
                Ok(()).into()
            }
            r => r,
        }
    }

    fn poll_write(&mut self, cx: &mut task::Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(s)) => {
                self.conn.send(EventType::Outbound(s as u64));
                Ok(s).into()
            }
            r => r,
        }
    }

    fn poll_flush(&mut self, cx: &mut task::Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(&mut self, cx: &mut task::Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

enum State {
    WaitConfig,
    Running {
        opt: Value,
        handle: JoinHandle<anyhow::Result<()>>,
        semaphore: Arc<Semaphore>,
    },
    Finished {
        result: anyhow::Result<()>,
    },
}

#[derive(Clone)]
pub struct RunningServer {
    #[allow(dead_code)]
    name: String,
    server_type: String,
    state: Arc<RwLock<State>>,
}

impl RunningServer {
    pub fn new(name: String, server_type: String) -> Self {
        RunningServer {
            name,
            server_type,
            state: Arc::new(RwLock::new(State::WaitConfig)),
        }
    }
    pub fn server_type(&self) -> &str {
        &self.server_type
    }
    pub async fn start(&self, server: Server, opt: &Value) -> anyhow::Result<()> {
        match &*self.state.read().await {
            // skip if config is not changed
            State::Running {
                opt: running_opt, ..
            } => {
                if opt == running_opt {
                    return Ok(());
                }
            }
            _ => {}
        };

        self.stop().await?;

        let name = self.name.clone();
        let semaphore = Arc::new(Semaphore::new(0));
        let s2 = semaphore.clone();
        let task = async move {
            let r = server.start().await.map_err(Into::into);
            s2.close();
            r
        };
        let handle = tokio::spawn(
            task.inspect_err(move |e| tracing::error!("Server {} error: {:?}", name, e))
                .instrument(tracing::info_span!("server_start")),
        );

        *self.state.write().await = State::Running {
            opt: opt.clone(),
            handle,
            semaphore,
        };

        Ok(())
    }
    pub async fn stop(&self) -> anyhow::Result<()> {
        match &*self.state.read().await {
            State::Running {
                handle, semaphore, ..
            } => {
                handle.abort();
                semaphore.close();
            }
            _ => return Ok(()),
        };
        self.join().await;

        Ok(())
    }
    pub async fn join(&self) {
        let state = self.state.read().await;

        match &*state {
            State::Running { semaphore, .. } => {
                let _ = semaphore.acquire().await;
            }
            _ => return,
        };
        drop(state);

        let mut state = self.state.write().await;

        let result = match &mut *state {
            State::Running { handle, .. } => handle.await.map_err(Into::into).and_then(|i| i),
            _ => return,
        };

        *state = State::Finished { result };
    }
    pub async fn take_result(&self) -> Option<anyhow::Result<()>> {
        let mut state = self.state.write().await;

        match &*state {
            State::Finished { .. } => {
                let old = replace(&mut *state, State::WaitConfig);
                return match old {
                    State::Finished { result, .. } => Some(result),
                    _ => unreachable!(),
                };
            }
            _ => return None,
        };
    }
}
