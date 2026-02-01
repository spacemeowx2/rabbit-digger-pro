use crate::udp::{decrypt_payload, encrypt_payload};
use bytes::BytesMut;
use futures::ready;
use rd_interface::{
    async_trait, impl_async_read_write, prelude::*, Address as RDAddress, AsyncRead, AsyncWrite,
    ITcpStream, IUdpSocket, ReadBuf, TcpStream, UdpSocket, NOT_IMPLEMENTED,
};
use shadowsocks::{
    context::SharedContext,
    crypto::CipherKind,
    relay::{socks5::Address as SSAddress, tcprelay::crypto_io},
    ProxyClientStream, ServerConfig,
};
use socks5_protocol::Address as S5Addr;
use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    str::FromStr,
    task::{self, Poll},
};

pub struct WrapAddress(pub RDAddress);

impl From<RDAddress> for WrapAddress {
    fn from(a: RDAddress) -> Self {
        Self(a)
    }
}

impl From<WrapAddress> for SSAddress {
    fn from(w: WrapAddress) -> Self {
        match w.0 {
            RDAddress::Domain(domain, port) => SSAddress::DomainNameAddress(domain, port),
            RDAddress::SocketAddr(s) => SSAddress::SocketAddress(s),
        }
    }
}

impl From<WrapAddress> for S5Addr {
    fn from(w: WrapAddress) -> Self {
        match w.0 {
            RDAddress::Domain(domain, port) => S5Addr::Domain(domain, port),
            RDAddress::SocketAddr(s) => S5Addr::SocketAddr(s),
        }
    }
}

#[rd_config]
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Debug, Copy, Clone)]
pub enum Cipher {
    #[serde(rename = "none")]
    NONE,
    #[serde(rename = "table")]
    SS_TABLE,
    #[serde(rename = "rc4-md5")]
    SS_RC4_MD5,
    #[serde(rename = "aes-128-ctr")]
    AES_128_CTR,
    #[serde(rename = "aes-192-ctr")]
    AES_192_CTR,
    #[serde(rename = "aes-256-ctr")]
    AES_256_CTR,
    #[serde(rename = "aes-128-cfb1")]
    AES_128_CFB1,
    #[serde(rename = "aes-128-cfb8")]
    AES_128_CFB8,
    #[serde(rename = "aes-128-cfb")]
    AES_128_CFB128,
    #[serde(rename = "aes-192-cfb1")]
    AES_192_CFB1,
    #[serde(rename = "aes-192-cfb8")]
    AES_192_CFB8,
    #[serde(rename = "aes-192-cfb")]
    AES_192_CFB128,
    #[serde(rename = "aes-256-cfb1")]
    AES_256_CFB1,
    #[serde(rename = "aes-256-cfb8")]
    AES_256_CFB8,
    #[serde(rename = "aes-256-cfb")]
    AES_256_CFB128,
    #[serde(rename = "aes-128-ofb")]
    AES_128_OFB,
    #[serde(rename = "aes-192-ofb")]
    AES_192_OFB,
    #[serde(rename = "aes-256-ofb")]
    AES_256_OFB,
    #[serde(rename = "camellia-128-ctr")]
    CAMELLIA_128_CTR,
    #[serde(rename = "camellia-192-ctr")]
    CAMELLIA_192_CTR,
    #[serde(rename = "camellia-256-ctr")]
    CAMELLIA_256_CTR,
    #[serde(rename = "camellia-128-cfb1")]
    CAMELLIA_128_CFB1,
    #[serde(rename = "camellia-128-cfb8")]
    CAMELLIA_128_CFB8,
    #[serde(rename = "camellia-128-cfb")]
    CAMELLIA_128_CFB128,
    #[serde(rename = "camellia-192-cfb1")]
    CAMELLIA_192_CFB1,
    #[serde(rename = "camellia-192-cfb8")]
    CAMELLIA_192_CFB8,
    #[serde(rename = "camellia-192-cfb")]
    CAMELLIA_192_CFB128,
    #[serde(rename = "camellia-256-cfb1")]
    CAMELLIA_256_CFB1,
    #[serde(rename = "camellia-256-cfb8")]
    CAMELLIA_256_CFB8,
    #[serde(rename = "camellia-256-cfb")]
    CAMELLIA_256_CFB128,
    #[serde(rename = "camellia-128-ofb")]
    CAMELLIA_128_OFB,
    #[serde(rename = "camellia-192-ofb")]
    CAMELLIA_192_OFB,
    #[serde(rename = "camellia-256-ofb")]
    CAMELLIA_256_OFB,
    #[serde(rename = "rc4")]
    RC4,
    #[serde(rename = "chacha20-ietf")]
    CHACHA20,
    #[serde(rename = "aes-128-gcm")]
    AES_128_GCM,
    #[serde(rename = "aes-256-gcm")]
    AES_256_GCM,
    #[serde(rename = "chacha20-ietf-poly1305")]
    CHACHA20_POLY1305,
    #[serde(rename = "aes-128-ccm")]
    AES_128_CCM,
    #[serde(rename = "aes-256-ccm")]
    AES_256_CCM,
    #[serde(rename = "aes-128-gcm-siv")]
    AES_128_GCM_SIV,
    #[serde(rename = "aes-256-gcm-siv")]
    AES_256_GCM_SIV,
    #[serde(rename = "xchacha20-ietf-poly1305")]
    XCHACHA20_POLY1305,
    #[serde(rename = "sm4-gcm")]
    SM4_GCM,
    #[serde(rename = "sm4-ccm")]
    SM4_CCM,
}

impl From<Cipher> for CipherKind {
    fn from(c: Cipher) -> Self {
        let s: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        CipherKind::from_str(s.as_str().unwrap()).unwrap()
    }
}

pub struct WrapSSTcp(pub ProxyClientStream<TcpStream>);

#[async_trait]
impl ITcpStream for WrapSSTcp {
    async fn peer_addr(&self) -> rd_interface::Result<SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }

    async fn local_addr(&self) -> rd_interface::Result<SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }

    impl_async_read_write!(0);
}

pub struct WrapSSUdp {
    socket: UdpSocket,
    method: CipherKind,
    key: Box<[u8]>,
    server_addr: SocketAddr,
    send_buf: BytesMut,
}

impl WrapSSUdp {
    pub fn new(socket: UdpSocket, svr_cfg: &ServerConfig, server_addr: SocketAddr) -> Self {
        let key = svr_cfg.key().to_vec().into_boxed_slice();
        let method = svr_cfg.method();

        WrapSSUdp {
            socket,
            method,
            key,
            server_addr,
            send_buf: BytesMut::new(),
        }
    }
}

#[async_trait]
impl IUdpSocket for WrapSSUdp {
    async fn local_addr(&self) -> rd_interface::Result<SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<SocketAddr>> {
        // Waiting for response from server SERVER -> CLIENT
        ready!(self.socket.poll_recv_from(cx, buf))?;
        let (n, addr) = decrypt_payload(self.method, &self.key, buf.filled_mut())?;
        buf.set_filled(n);

        Poll::Ready(Ok(match addr {
            S5Addr::Domain(_, _) => unreachable!("Udp recv_from domain name"),
            S5Addr::SocketAddr(s) => s,
        }))
    }

    fn poll_send_to(
        &mut self,
        cx: &mut task::Context<'_>,
        buf: &[u8],
        target: &RDAddress,
    ) -> Poll<io::Result<usize>> {
        if self.send_buf.is_empty() {
            let addr: S5Addr = WrapAddress::from(target.clone()).into();
            encrypt_payload(self.method, &self.key, &addr, buf, &mut self.send_buf)?;
        }

        ready!(self
            .socket
            .poll_send_to(cx, &self.send_buf, &self.server_addr.into()))?;
        self.send_buf.clear();

        Poll::Ready(Ok(buf.len()))
    }
}

pub struct CryptoStream<S>(crypto_io::CryptoStream<S>, SharedContext);

impl<S> CryptoStream<S> {
    pub fn from_server_stream(
        context: SharedContext,
        stream: S,
        method: CipherKind,
        key: &[u8],
    ) -> Self {
        CryptoStream(
            crypto_io::CryptoStream::<S>::from_stream(
                &context,
                stream,
                crypto_io::StreamType::Server,
                method,
                key,
            ),
            context,
        )
    }
}

impl<S> AsyncRead for CryptoStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let CryptoStream(s, c) = Pin::get_mut(self);
        crypto_io::CryptoRead::poll_read_decrypted(Pin::new(s), cx, c, buf).map_err(Into::into)
    }
}

impl<S> AsyncWrite for CryptoStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        crypto_io::CryptoWrite::poll_write_encrypted(Pin::new(&mut self.0), cx, buf)
            .map_err(Into::into)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        self.0.poll_flush(cx).map_err(Into::into)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        self.0.poll_shutdown(cx).map_err(Into::into)
    }
}
