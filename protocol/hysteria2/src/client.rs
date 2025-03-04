use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures::{ready, AsyncRead, AsyncWrite, FutureExt};
use h3::{
    client::{Connection, ResponseStream, SendRequest},
    error::ErrorLevel,
    ext::Protocol,
    proto::frame::FramePayload,
};
use h3_quinn::{Connection as QuinnConnection, OpenStreams};
use quinn::{ClientConfig, Endpoint, TransportConfig};
use rd_interface::{
    async_trait, registry::Builder, Address, Error, INet, IntoDyn, Net, ReadBuf, Result,
    TcpStream, UdpSocket,
};
use tokio::sync::Mutex;

use crate::{
    config::Hysteria2Config,
    crypto::Salamander,
    protocol::{StreamRequest, StreamResponse, UdpMessage},
};

const ALPN_H3: &[u8] = b"h3";

type ClientConnection = Connection<QuinnConnection>;
type ClientSendRequest = SendRequest<QuinnConnection>;
type StreamWrapper = h3::client::SendStreamAndResponse<QuinnConnection>;
type BidiStreamType = h3::client::RequestStream<h3_quinn::OpenStreams>;

pub struct Hysteria2Net {
    conn: Arc<Mutex<Option<ClientConnection>>>,
    send_request: Arc<Mutex<Option<ClientSendRequest>>>,
    config: Arc<Hysteria2Config>,
    obfs: Option<Salamander>,
    net: Net,
}

impl Hysteria2Net {
    pub fn new(config: Hysteria2Config) -> Self {
        let obfs = config.obfs.as_ref().map(|pwd| Salamander::new(pwd));

        Self {
            conn: Arc::new(Mutex::new(None)),
            send_request: Arc::new(Mutex::new(None)),
            config: Arc::new(config.clone()),
            obfs,
            net: config.net.value_cloned(),
        }
    }

    async fn ensure_connected(&self) -> Result<()> {
        if self.send_request.lock().await.is_some() {
            return Ok(());
        }

        // Configure QUIC client
        let mut client_config = ClientConfig::with_native_roots();
        let mut tls_config = client_config.crypto.tls_client_config();
        tls_config.alpn_protocols = vec![ALPN_H3.to_vec()];

        if self.config.skip_cert_verify {
            // TODO: implement skip cert verify
        }

        let mut transport_config = TransportConfig::default();
        transport_config
            .max_idle_timeout(Some(std::time::Duration::from_secs(30).try_into().unwrap()));
        client_config.transport_config(Arc::new(transport_config));

        // Create endpoint
        let mut endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap())?;
        endpoint.set_default_client_config(client_config);

        // Connect to server
        let connection = endpoint
            .connect(
                self.config.server.to_socket_addr()?,
                self.config
                    .sni
                    .as_deref()
                    .unwrap_or_else(|| self.config.server.host().as_str()),
            )?
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Perform HTTP/3 handshake
        let conn = h3_quinn::Connection::new(connection);
        let (driver, send_request) = h3::client::new(conn)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Spawn connection driver
        tokio::spawn(async move {
            if let Err(e) = driver.serve().await {
                tracing::error!("Connection driver error: {:?}", e);
            }
        });

        // Send authentication request
        let mut req = client::Request::builder()
            .method("POST")
            .uri("/auth")
            .header("hysteria-auth", &self.config.auth)
            .header("hysteria-cc-rx", self.config.rx_window.to_string())
            .header(":scheme", "https")
            .header(":authority", "hysteria")
            .body(())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        req.extensions_mut().insert(Protocol::new());

        let mut res = send_request
            .send_request(req)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let status = res.status();
        if status != 233 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("Authentication failed: {}", status),
            )
            .into());
        }

        *self.send_request.lock().await = Some(send_request);

        Ok(())
    }

    async fn create_stream(&self, addr: &Address) -> Result<StreamWrapper> {
        self.ensure_connected().await?;

        let mut req = client::Request::builder()
            .method("POST")
            .uri("/")
            .header(":scheme", "https")
            .header(":authority", "hysteria")
            .body(())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        req.extensions_mut().insert(Protocol::new());

        let (mut send, recv) = self
            .send_request
            .lock()
            .await
            .as_mut()
            .unwrap()
            .send_request(req)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            .into_parts();

        let stream_req = StreamRequest::new(format!("{}:{}", addr.host(), addr.port()));
        send.write_all(&stream_req.encode()).await?;

        let mut buf = vec![0; 1024];
        let n = recv.read(&mut buf).await?;
        let resp = StreamResponse::decode(io::Cursor::new(&buf[..n]))?;

        if resp.status != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Stream request failed: {}", resp.message),
            )
            .into());
        }

        Ok(StreamWrapper::new(send, recv))
    }
}

#[async_trait]
impl rd_interface::TcpConnect for Hysteria2Net {
    async fn tcp_connect(
        &self,
        _ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> Result<TcpStream> {
        let stream = self.create_stream(addr).await?;
        Ok(Hysteria2Stream::new(stream).into_dyn())
    }
}

#[async_trait]
impl rd_interface::UdpBind for Hysteria2Net {
    async fn udp_bind(
        &self,
        _ctx: &mut rd_interface::Context,
        _addr: &Address,
    ) -> Result<UdpSocket> {
        if self.config.disable_udp {
            return Err(Error::NotEnabled);
        }

        self.ensure_connected().await?;

        Ok(Hysteria2Udp::new(self.send_request.clone()).into_dyn())
    }
}

impl INet for Hysteria2Net {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        if !self.config.disable_udp {
            Some(self)
        } else {
            None
        }
    }
}

impl Builder<Net> for Hysteria2Net {
    const NAME: &'static str = "hysteria2";
    type Config = Hysteria2Config;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        Ok(Self::new(config))
    }
}

impl rd_interface::IntoDyn<TcpStream> for Hysteria2Stream {
    fn into_dyn(self) -> TcpStream {
        TcpStream(Box::new(self))
    }
}

impl rd_interface::IntoDyn<UdpSocket> for Hysteria2Udp {
    fn into_dyn(self) -> UdpSocket {
        UdpSocket(Box::new(self))
    }
}

pub struct Hysteria2Stream {
    stream: StreamWrapper,
}

impl Hysteria2Stream {
    fn new(stream: StreamWrapper) -> Self {
        Self { stream }
    }
}

#[async_trait]
impl rd_interface::ITcpStream for Hysteria2Stream {
    async fn peer_addr(&self) -> Result<SocketAddr> {
        Err(Error::NotImplemented)
    }

    async fn local_addr(&self) -> Result<SocketAddr> {
        Err(Error::NotImplemented)
    }

    fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let n = ready!(Pin::new(&mut self.stream).poll_data(cx))?
            .map(|b| {
                let len = b.len().min(buf.remaining());
                buf.initialize_unfilled_to(len).copy_from_slice(&b[..len]);
                len
            })
            .unwrap_or(0);
        buf.advance(n);
        Poll::Ready(Ok(()))
    }

    fn poll_write(
        &mut self,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        ready!(Pin::new(&mut self.stream).poll_ready(cx))?;
        Pin::new(&mut self.stream)
            .start_send(Bytes::copy_from_slice(buf))?;
        ready!(Pin::new(&mut self.stream).poll_flush(cx))?;
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_close(cx)
    }
}

pub struct Hysteria2Udp {
    send_request: Arc<Mutex<Option<ClientSendRequest>>>,
    next_session_id: u32,
    stream: Option<StreamWrapper>,
}

impl Hysteria2Udp {
    fn new(send_request: Arc<Mutex<Option<ClientSendRequest>>>) -> Self {
        Self {
            send_request,
            next_session_id: 0,
            stream: None,
        }
    }

    fn next_id(&mut self) -> u32 {
        let id = self.next_session_id;
        self.next_session_id = self.next_session_id.wrapping_add(1);
        id
    }

    async fn ensure_stream(&mut self) -> io::Result<&mut StreamWrapper> {
        if self.stream.is_none() {
            let mut req = client::Request::builder()
                .method("POST")
                .uri("/udp")
                .header(":scheme", "https")
                .header(":authority", "hysteria")
                .body(())
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            req.extensions_mut().insert(Protocol::new());

            let send_request = self.send_request.lock().await;
            let (send, recv) = send_request
                .as_ref()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "not connected"))?
                .send_request(req)
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
                .into_parts();

            self.stream = Some(StreamWrapper::new(send, recv));
        }

        Ok(self.stream.as_mut().unwrap())
    }
}

#[async_trait]
impl rd_interface::IUdpSocket for Hysteria2Udp {
    async fn local_addr(&self) -> Result<SocketAddr> {
        Err(Error::NotImplemented)
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<SocketAddr>> {
        let stream = ready!(self.ensure_stream().poll_unpin(cx))?;

        let mut recv_buf = vec![0; 65535];
        let n = ready!(Pin::new(stream).poll_read(cx, &mut recv_buf))?;

        if n == 0 {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed",
            )));
        }

        let msg = UdpMessage::decode(io::Cursor::new(&recv_buf[..n]))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let addr = msg
            .addr
            .parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let to_copy = msg.payload.len().min(buf.remaining());
        buf.initialize_unfilled_to(to_copy)
            .copy_from_slice(&msg.payload[..to_copy]);
        buf.advance(to_copy);

        Poll::Ready(Ok(addr))
    }

    fn poll_send_to(
        &mut self,
        cx: &mut Context<'_>,
        buf: &[u8],
        target: &Address,
    ) -> Poll<io::Result<usize>> {
        let stream = ready!(self.ensure_stream().poll_unpin(cx))?;

        let msg = UdpMessage::new(
            self.next_id(),
            format!("{}:{}", target.host(), target.port()),
            Bytes::copy_from_slice(buf),
        );

        let encoded = msg.encode();
        ready!(Pin::new(stream).poll_write(cx, &encoded))?;
        ready!(Pin::new(stream).poll_flush(cx))?;

        Poll::Ready(Ok(buf.len()))
    }
}
