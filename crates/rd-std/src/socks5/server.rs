use super::common::{pack_udp, parse_udp, sa2ra};
use crate::ContextExt;
use anyhow::Context as AnyhowContext;
use futures::ready;
use rd_interface::{
    async_trait, constant::UDP_BUFFER_SIZE, Address as RdAddr, Address as RDAddr, AsyncRead,
    Context, IServer, IUdpChannel, IntoDyn, Net, ReadBuf, Result, TcpStream, UdpSocket,
};
use socks5_protocol::{
    Address, AuthMethod, AuthRequest, AuthResponse, Command, CommandReply, CommandRequest,
    CommandResponse, Version,
};
use std::{
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    pin::Pin,
    sync::Arc,
    task::{self, Poll},
};
use tokio::io::{AsyncWriteExt, BufWriter};
use tracing::instrument;

struct Socks5ServerConfig {
    net: Net,
    listen_net: Net,
}

#[derive(Clone)]
pub struct Socks5Server {
    cfg: Arc<Socks5ServerConfig>,
}

impl Socks5Server {
    async fn handle_command_request(
        &self,
        mut socket: &mut BufWriter<TcpStream>,
    ) -> anyhow::Result<CommandRequest> {
        let version = Version::read(&mut socket).await?;
        let auth_req = AuthRequest::read(&mut socket).await?;

        let method = auth_req.select_from(&[AuthMethod::Noauth]);
        let auth_resp = AuthResponse::new(method);
        // TODO: do auth here

        version.write(&mut socket).await?;
        auth_resp.write(&mut socket).await?;
        socket.flush().await?;

        let cmd_req = CommandRequest::read(&mut socket).await?;

        Ok(cmd_req)
    }
    async fn response_command_error(
        &self,
        mut socket: &mut BufWriter<TcpStream>,
        e: impl std::convert::TryInto<io::Error>,
    ) -> anyhow::Result<()> {
        CommandResponse::error(e).write(&mut socket).await?;
        socket.flush().await?;
        Ok(())
    }
    #[instrument(err, skip(self, socket))]
    pub async fn serve_connection(self, socket: TcpStream, addr: SocketAddr) -> anyhow::Result<()> {
        let mut socket = BufWriter::with_capacity(512, socket);

        let default_addr: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));
        let Socks5ServerConfig { net, listen_net } = &*self.cfg;
        let local_ip = socket.get_ref().local_addr().await?.ip();

        let cmd_req = self
            .handle_command_request(&mut socket)
            .await
            .context("handle command request")?;

        match cmd_req.command {
            Command::Connect => {
                let dst = sa2ra(cmd_req.address);
                let ctx = &mut Context::from_socketaddr(addr);
                let out = match net.tcp_connect(ctx, &dst).await {
                    Ok(socket) => socket,
                    Err(e) => return self.response_command_error(&mut socket, e).await,
                };

                let addr = out.local_addr().await.unwrap_or(default_addr).into();
                CommandResponse::success(addr).write(&mut socket).await?;
                socket.flush().await.context("command response")?;

                let socket = socket.into_inner();

                ctx.connect_tcp(out, socket).await.context("connect tcp")?;
            }
            Command::UdpAssociate => {
                let dst = match cmd_req.address {
                    Address::SocketAddr(addr) => rd_interface::Address::any_addr_port(&addr),
                    _ => {
                        CommandResponse::reply_error(CommandReply::AddressTypeNotSupported)
                            .write(&mut socket)
                            .await?;

                        socket.flush().await?;
                        return Ok(());
                    }
                };
                let ctx = &mut Context::from_socketaddr(addr);
                let out = match net.udp_bind(ctx, &dst).await {
                    Ok(socket) => socket,
                    Err(e) => return self.response_command_error(&mut socket, e).await,
                };
                let udp = listen_net
                    .udp_bind(
                        &mut Context::from_socketaddr(addr),
                        &RdAddr::any_addr_port(&addr),
                    )
                    .await?;

                // success
                let udp_port = match udp.local_addr().await {
                    Ok(a) => a.port(),
                    Err(e) => return self.response_command_error(&mut socket, e).await,
                };
                let addr: SocketAddr = (local_ip, udp_port).into();
                let addr: Address = addr.into();

                CommandResponse::success(addr).write(&mut socket).await?;
                socket.flush().await.context("command response")?;

                let udp_channel = Socks5UdpSocket {
                    udp,
                    tcp: socket.into_inner(),
                    endpoint: None,
                    send_buf: Vec::with_capacity(UDP_BUFFER_SIZE),
                };
                ctx.connect_udp(udp_channel.into_dyn(), out)
                    .await
                    .context("connect udp")?;
            }
            _ => {
                return Ok(());
            }
        };

        Ok(())
    }
    pub fn new(listen_net: Net, net: Net) -> Self {
        Self {
            cfg: Arc::new(Socks5ServerConfig { net, listen_net }),
        }
    }
}

pub struct Socks5UdpSocket {
    udp: UdpSocket,
    // keep connection
    tcp: TcpStream,
    endpoint: Option<SocketAddr>,
    send_buf: Vec<u8>,
}

impl Socks5UdpSocket {
    fn poll_tcp_close(&mut self, cx: &mut task::Context<'_>) -> Poll<bool> {
        let mut buf = [0; 1];
        let mut buf = ReadBuf::new(&mut buf);
        Poll::Ready(ready!(Pin::new(&mut self.tcp).poll_read(cx, &mut buf)).is_err())
    }
}

impl IUdpChannel for Socks5UdpSocket {
    fn poll_send_to(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<RDAddr>> {
        let from_addr = ready!(self.udp.poll_recv_from(cx, buf))?;
        if self.endpoint.is_none() {
            self.endpoint = Some(from_addr);
        }

        let addr = parse_udp(buf)?;

        Poll::Ready(Ok(addr))
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        target: &SocketAddr,
    ) -> Poll<io::Result<usize>> {
        if let Poll::Ready(true) = self.poll_tcp_close(cx) {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "connection closed",
            )));
        }
        let Socks5UdpSocket {
            udp,
            endpoint,
            send_buf,
            ..
        } = &mut *self;

        if send_buf.is_empty() {
            pack_udp((*target).into(), buf, send_buf)?;
        }

        match endpoint {
            Some(endpoint) => {
                ready!(udp.poll_send_to(cx, send_buf, &(*endpoint).into()))?;
                send_buf.clear();
            }
            None => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "udp endpoint not set",
                )))
            }
        };

        Poll::Ready(Ok(buf.len()))
    }
}

pub struct Socks5 {
    server: Socks5Server,
    listen_net: Net,
    bind: RdAddr,
}

#[async_trait]
impl IServer for Socks5 {
    async fn start(&self) -> Result<()> {
        let listener = self
            .listen_net
            .tcp_bind(&mut Context::new(), &self.bind)
            .await?;

        loop {
            let (socket, addr) = listener.accept().await?;
            let server = self.server.clone();
            let _ = tokio::spawn(async move {
                if let Err(e) = server.serve_connection(socket, addr).await {
                    tracing::error!("Error when serve_connection: {:?}", e)
                }
            });
        }
    }
}

impl Socks5 {
    pub fn new(listen_net: Net, net: Net, bind: RdAddr) -> Self {
        Socks5 {
            server: Socks5Server::new(listen_net.clone(), net),
            listen_net,
            bind,
        }
    }
}
