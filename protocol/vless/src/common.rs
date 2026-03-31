use std::{
    fs, io,
    path::Path,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::{SinkExt, StreamExt};
use rd_interface::{error::map_other, Address, AsyncRead, AsyncWrite, ReadBuf, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls::{
    crypto::ring::default_provider,
    pki_types::{CertificateDer, PrivateKeyDer},
    ServerConfig,
};
use tokio_util::codec::{Decoder, Encoder};
use uuid::Uuid;

pub const VLESS_VERSION: u8 = 0;
pub const COMMAND_TCP: u8 = 0x01;
pub const COMMAND_MUX: u8 = 0x03;
pub const FLOW_VISION: &str = "xtls-rprx-vision";
pub const MUX_DOMAIN: &str = "v1.mux.cool";
pub const MUX_PORT: u16 = 666;
const ADDRESS_TYPE_IPV4: u8 = 1;
const ADDRESS_TYPE_DOMAIN: u8 = 2;
const ADDRESS_TYPE_IPV6: u8 = 3;
const MAX_VISION_CONTENT: usize = u16::MAX as usize;

pub fn ensure_rustls_provider_installed() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = default_provider().install_default();
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedFlow(Option<String>);

impl NormalizedFlow {
    pub fn parse(flow: Option<String>) -> Result<Self> {
        match flow.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => Ok(Self(None)),
            Some(FLOW_VISION) => Ok(Self(Some(FLOW_VISION.to_string()))),
            Some(other) => Err(rd_interface::Error::other(format!(
                "unsupported vless flow: {other}"
            ))),
        }
    }

    pub fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }

    pub fn is_vision(&self) -> bool {
        self.as_deref() == Some(FLOW_VISION)
    }
}

#[derive(Clone, Debug)]
pub struct UserId([u8; 16]);

impl UserId {
    pub fn parse(id: &str) -> Result<Self> {
        Ok(Self(*Uuid::parse_str(id).map_err(map_other)?.as_bytes()))
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

#[derive(Debug)]
pub struct RequestHeader {
    pub command: u8,
    pub addr: Address,
    pub flow: NormalizedFlow,
}

pub async fn write_request_header<S>(
    stream: &mut S,
    user_id: &UserId,
    flow: &NormalizedFlow,
    command: u8,
    addr: &Address,
) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let buf = build_request_header(user_id, flow, command, addr)?;
    stream.write_all(&buf).await?;
    stream.flush().await
}

pub fn build_request_header(
    user_id: &UserId,
    flow: &NormalizedFlow,
    command: u8,
    addr: &Address,
) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(64);
    buf.push(VLESS_VERSION);
    buf.extend_from_slice(user_id.as_bytes());
    write_addons(&mut buf, flow.as_deref())?;
    buf.push(command);
    if command != COMMAND_MUX {
        write_address(&mut buf, addr)?;
    }
    Ok(buf)
}

pub async fn read_request_header<S>(stream: &mut S, expected: &UserId) -> io::Result<RequestHeader>
where
    S: AsyncRead + Unpin,
{
    let mut version = [0u8; 1];
    stream.read_exact(&mut version).await?;
    if version[0] != VLESS_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported vless version {}", version[0]),
        ));
    }

    let mut user = [0u8; 16];
    stream.read_exact(&mut user).await?;
    if user != *expected.as_bytes() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid vless user id",
        ));
    }

    let flow = read_addons(stream).await?;
    let mut command = [0u8; 1];
    stream.read_exact(&mut command).await?;
    let addr = if command[0] == COMMAND_MUX {
        Address::Domain(MUX_DOMAIN.to_string(), MUX_PORT)
    } else {
        read_address(stream).await?
    };
    Ok(RequestHeader {
        command: command[0],
        addr,
        flow,
    })
}

pub async fn write_response_header<S>(stream: &mut S) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream.write_all(&[VLESS_VERSION, 0]).await?;
    stream.flush().await
}

pub struct ResponseHeaderStream<S> {
    inner: S,
    header: [u8; 2],
    header_read: usize,
    skipped: Vec<u8>,
    skipped_read: usize,
    ready: bool,
}

pub struct PrefixWriteStream<S> {
    inner: S,
    pending: BytesMut,
    pending_payload_len: usize,
    prefix_sent: bool,
}

impl<S> PrefixWriteStream<S> {
    pub fn new(inner: S, prefix: Vec<u8>) -> Self {
        Self {
            inner,
            pending: BytesMut::from(prefix.as_slice()),
            pending_payload_len: 0,
            prefix_sent: false,
        }
    }

    fn poll_drain_pending(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>
    where
        S: AsyncWrite + Unpin,
    {
        while !self.pending.is_empty() {
            let wrote = match Pin::new(&mut self.inner).poll_write(cx, &self.pending) {
                Poll::Ready(Ok(n)) => n,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            };
            self.pending.advance(wrote);
            if !self.pending.is_empty() {
                return Poll::Pending;
            }
        }
        self.prefix_sent = true;
        Poll::Ready(Ok(()))
    }
}

impl<S> AsyncRead for PrefixWriteStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if !self.pending.is_empty() {
            match self.poll_drain_pending(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S> AsyncWrite for PrefixWriteStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if !self.prefix_sent {
            if self.pending_payload_len == 0 && !buf.is_empty() {
                self.pending.extend_from_slice(buf);
                self.pending_payload_len = buf.len();
            }
            match self.poll_drain_pending(cx) {
                Poll::Ready(Ok(())) => {
                    let payload_len = self.pending_payload_len;
                    self.pending_payload_len = 0;
                    return Poll::Ready(Ok(payload_len));
                }
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.pending.is_empty() {
            match self.poll_drain_pending(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.pending.is_empty() {
            match self.poll_drain_pending(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl<S> ResponseHeaderStream<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            header: [0; 2],
            header_read: 0,
            skipped: Vec::new(),
            skipped_read: 0,
            ready: false,
        }
    }

    fn poll_ready_header(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>
    where
        S: AsyncRead + Unpin,
    {
        while !self.ready {
            if self.header_read < 2 {
                let mut rb = ReadBuf::new(&mut self.header[self.header_read..]);
                match Pin::new(&mut self.inner).poll_read(cx, &mut rb) {
                    Poll::Ready(Ok(())) => {
                        if rb.filled().is_empty() {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "early eof",
                            )));
                        }
                        self.header_read += rb.filled().len();
                    }
                    Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                    Poll::Pending => return Poll::Pending,
                }
                continue;
            }

            if self.header[0] != VLESS_VERSION {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected vless response version {}", self.header[0]),
                )));
            }
            if self.skipped.is_empty() {
                self.skipped.resize(self.header[1] as usize, 0);
                if self.skipped.is_empty() {
                    self.ready = true;
                    break;
                }
            }

            if self.skipped_read < self.skipped.len() {
                let mut rb = ReadBuf::new(&mut self.skipped[self.skipped_read..]);
                match Pin::new(&mut self.inner).poll_read(cx, &mut rb) {
                    Poll::Ready(Ok(())) => {
                        if rb.filled().is_empty() {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "early eof",
                            )));
                        }
                        self.skipped_read += rb.filled().len();
                    }
                    Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                    Poll::Pending => return Poll::Pending,
                }
                continue;
            }

            self.ready = true;
        }

        Poll::Ready(Ok(()))
    }
}

impl<S> AsyncRead for ResponseHeaderStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.poll_ready_header(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut self.inner).poll_read(cx, buf),
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> AsyncWrite for ResponseHeaderStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub struct VisionStream<S> {
    inner: S,
    user_id: [u8; 16],
    wrote_uuid: bool,
    write_pending: BytesMut,
    write_payload_len: usize,
    read_buf: BytesMut,
    read_pending: BytesMut,
    expect_uuid: bool,
    remaining_header: usize,
    current_command: u8,
    remaining_content: usize,
    remaining_padding: usize,
    direct_mode: bool,
    shared_read_raw: Option<Arc<AtomicBool>>,
}

impl<S> VisionStream<S> {
    pub fn new_with_shared(
        inner: S,
        user_id: &UserId,
        shared_read_raw: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self::new(inner, user_id, shared_read_raw)
    }

    fn new(inner: S, user_id: &UserId, shared_read_raw: Option<Arc<AtomicBool>>) -> Self {
        Self {
            inner,
            user_id: *user_id.as_bytes(),
            wrote_uuid: false,
            write_pending: BytesMut::new(),
            write_payload_len: 0,
            read_buf: BytesMut::new(),
            read_pending: BytesMut::new(),
            expect_uuid: true,
            remaining_header: 0,
            current_command: 0,
            remaining_content: 0,
            remaining_padding: 0,
            direct_mode: false,
            shared_read_raw,
        }
    }
    fn enqueue_write(&mut self, buf: &[u8]) -> usize {
        let payload_len = buf.len().min(MAX_VISION_CONTENT);
        if !self.wrote_uuid {
            self.write_pending.extend_from_slice(&self.user_id);
            self.wrote_uuid = true;
        }
        self.write_pending.put_u8(0);
        self.write_pending.put_u16(payload_len as u16);
        self.write_pending.put_u16(0);
        self.write_pending.extend_from_slice(&buf[..payload_len]);
        self.write_payload_len = payload_len;
        payload_len
    }

    fn try_decode_frames(&mut self) -> io::Result<()> {
        loop {
            if self.direct_mode {
                if !self.read_buf.is_empty() {
                    self.read_pending
                        .extend_from_slice(&self.read_buf.split().freeze());
                }
                return Ok(());
            }

            if self.expect_uuid {
                let prefix_len = self.read_buf.len().min(self.user_id.len());
                if prefix_len > 0 && self.read_buf[..prefix_len] != self.user_id[..prefix_len] {
                    self.direct_mode = true;
                    if !self.read_buf.is_empty() {
                        self.read_pending
                            .extend_from_slice(&self.read_buf.split().freeze());
                    }
                    return Ok(());
                }
                if self.read_buf.len() < 21 {
                    return Ok(());
                }
                self.read_buf.advance(16);
                self.expect_uuid = false;
                self.remaining_header = 5;
            }

            if self.remaining_header > 0 {
                if self.read_buf.len() < self.remaining_header {
                    return Ok(());
                }
                while self.remaining_header > 0 {
                    let b = self.read_buf.get_u8();
                    match self.remaining_header {
                        5 => self.current_command = b,
                        4 => self.remaining_content = (b as usize) << 8,
                        3 => self.remaining_content |= b as usize,
                        2 => self.remaining_padding = (b as usize) << 8,
                        1 => self.remaining_padding |= b as usize,
                        _ => unreachable!(),
                    }
                    self.remaining_header -= 1;
                }
            }

            let need = self.remaining_content + self.remaining_padding;
            if self.read_buf.len() < need {
                return Ok(());
            }

            if self.remaining_content > 0 {
                let content = self.read_buf.split_to(self.remaining_content);
                self.read_pending.extend_from_slice(&content);
                self.remaining_content = 0;
            }
            if self.remaining_padding > 0 {
                self.read_buf.advance(self.remaining_padding);
                self.remaining_padding = 0;
            }

            if self.current_command == 0 {
                self.remaining_header = 5;
                continue;
            }

            self.direct_mode = true;
            if self.current_command == 2 {
                if let Some(shared) = &self.shared_read_raw {
                    shared.store(true, Ordering::Relaxed);
                }
            }
            if !self.read_buf.is_empty() {
                self.read_pending
                    .extend_from_slice(&self.read_buf.split().freeze());
            }
            return Ok(());
        }
    }
}

impl<S> AsyncRead for VisionStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut rd_interface::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        if !this.read_pending.is_empty() {
            let to_copy = this.read_pending.len().min(buf.remaining());
            buf.put_slice(&this.read_pending.split_to(to_copy));
            return Poll::Ready(Ok(()));
        }

        if this.direct_mode {
            return Pin::new(&mut this.inner).poll_read(cx, buf);
        }

        loop {
            match this.try_decode_frames() {
                Ok(()) => {
                    if !this.read_pending.is_empty() {
                        let to_copy = this.read_pending.len().min(buf.remaining());
                        buf.put_slice(&this.read_pending.split_to(to_copy));
                        return Poll::Ready(Ok(()));
                    }
                }
                Err(err) => return Poll::Ready(Err(err)),
            }

            let mut tmp = [0u8; 4096];
            let mut rb = rd_interface::ReadBuf::new(&mut tmp);
            match Pin::new(&mut this.inner).poll_read(cx, &mut rb) {
                Poll::Ready(Ok(())) => {
                    if rb.filled().is_empty() {
                        return Poll::Ready(Ok(()));
                    }
                    this.read_buf.extend_from_slice(rb.filled());
                }
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<S> AsyncWrite for VisionStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();

        while !this.write_pending.is_empty() {
            let wrote = match Pin::new(&mut this.inner).poll_write(cx, &this.write_pending) {
                Poll::Ready(Ok(n)) => n,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            };
            this.write_pending.advance(wrote);
            if !this.write_pending.is_empty() {
                return Poll::Pending;
            }
            let payload_len = this.write_payload_len;
            this.write_payload_len = 0;
            return Poll::Ready(Ok(payload_len));
        }

        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let payload_len = this.enqueue_write(buf);
        while !this.write_pending.is_empty() {
            let wrote = match Pin::new(&mut this.inner).poll_write(cx, &this.write_pending) {
                Poll::Ready(Ok(n)) => n,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            };
            this.write_pending.advance(wrote);
            if !this.write_pending.is_empty() {
                return Poll::Pending;
            }
        }
        this.write_payload_len = 0;
        Poll::Ready(Ok(payload_len))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.write_pending.is_empty() {
            while !this.write_pending.is_empty() {
                let wrote = match Pin::new(&mut this.inner).poll_write(cx, &this.write_pending) {
                    Poll::Ready(Ok(n)) => n,
                    Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                    Poll::Pending => return Poll::Pending,
                };
                this.write_pending.advance(wrote);
            }
        }
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.write_pending.is_empty() {
            while !this.write_pending.is_empty() {
                let wrote = match Pin::new(&mut this.inner).poll_write(cx, &this.write_pending) {
                    Poll::Ready(Ok(n)) => n,
                    Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                    Poll::Pending => return Poll::Pending,
                };
                this.write_pending.advance(wrote);
            }
        }
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

pub struct XudpCodec {
    last_addr: Option<Address>,
}

impl XudpCodec {
    pub fn client() -> Self {
        Self { last_addr: None }
    }

    pub fn server() -> Self {
        Self { last_addr: None }
    }
}

impl Encoder<(Bytes, Address)> for XudpCodec {
    type Error = io::Error;

    fn encode(&mut self, item: (Bytes, Address), dst: &mut BytesMut) -> io::Result<()> {
        if item.0.len() > u16::MAX as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "xudp payload too large",
            ));
        }

        let mut meta = BytesMut::new();
        let use_compact = self.last_addr.as_ref() == Some(&item.1);
        meta.put_u16(0); // session id
        meta.put_u8(if use_compact { 2 } else { 1 });
        meta.put_u8(1);
        if !use_compact {
            meta.put_u8(2);
            write_address_buf(&mut meta, &item.1)?;
            self.last_addr = Some(item.1.clone());
        }

        dst.reserve(2 + meta.len() + 2 + item.0.len());
        dst.put_u16(meta.len() as u16);
        dst.extend_from_slice(&meta);
        dst.put_u16(item.0.len() as u16);
        dst.extend_from_slice(&item.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tokio::io::duplex;

    #[tokio::test]
    async fn test_vision_stream_transitions_to_direct_mode_and_updates_shared_flag() {
        let user_id = UserId::parse("27848739-7e61-4ea0-ba56-d8edf2587d12").unwrap();
        let shared = Arc::new(AtomicBool::new(false));
        let (mut peer, io) = duplex(128);
        let mut stream = VisionStream::new_with_shared(io, &user_id, Some(shared.clone()));

        let mut frame = Vec::new();
        frame.extend_from_slice(user_id.as_bytes());
        frame.extend_from_slice(&[2, 0, 0, 0, 0]);
        frame.extend_from_slice(b"x");
        peer.write_all(&frame).await.unwrap();

        let mut buf = [0u8; 1];
        stream.read_exact(&mut buf).await.unwrap();

        assert_eq!(&buf, b"x");
        assert!(shared.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn test_prefix_write_stream_sends_header_once_before_payload() {
        let (mut peer, io) = duplex(128);
        let mut stream = PrefixWriteStream::new(io, b"hdr".to_vec());

        stream.write_all(b"abc").await.unwrap();
        stream.flush().await.unwrap();

        let mut first = [0u8; 6];
        peer.read_exact(&mut first).await.unwrap();
        assert_eq!(&first, b"hdrabc");

        stream.write_all(b"def").await.unwrap();
        stream.flush().await.unwrap();

        let mut second = [0u8; 3];
        peer.read_exact(&mut second).await.unwrap();
        assert_eq!(&second, b"def");
    }

    #[tokio::test]
    async fn test_mux_request_header_omits_address_and_round_trips() {
        let user_id = UserId::parse("27848739-7e61-4ea0-ba56-d8edf2587d12").unwrap();
        let flow = NormalizedFlow::parse(Some(FLOW_VISION.to_string())).unwrap();
        let mux_addr = Address::Domain(MUX_DOMAIN.to_string(), MUX_PORT);
        let header = build_request_header(&user_id, &flow, COMMAND_MUX, &mux_addr).unwrap();

        assert_eq!(header[0], VLESS_VERSION);
        assert_eq!(header.last().copied(), Some(COMMAND_MUX));

        let (mut writer, mut reader) = duplex(128);
        writer.write_all(&header).await.unwrap();

        let parsed = read_request_header(&mut reader, &user_id).await.unwrap();
        assert_eq!(parsed.command, COMMAND_MUX);
        assert_eq!(parsed.addr, mux_addr);
        assert_eq!(parsed.flow, flow);
    }

    #[test]
    fn test_xudp_codec_encodes_compact_follow_up_packet() {
        let addr = Address::SocketAddr("127.0.0.1:5353".parse().unwrap());
        let mut codec = XudpCodec::client();

        let mut first = BytesMut::new();
        codec
            .encode((Bytes::from_static(b"one"), addr.clone()), &mut first)
            .unwrap();
        assert_eq!(u16::from_be_bytes([first[0], first[1]]), 12);
        assert_eq!(first[4], 1);

        let mut second = BytesMut::new();
        codec
            .encode((Bytes::from_static(b"two"), addr), &mut second)
            .unwrap();
        assert_eq!(&second[..6], &[0, 4, 0, 0, 2, 1]);
    }

    #[test]
    fn test_xudp_codec_decodes_compact_follow_up_packet() {
        let addr = Address::SocketAddr("127.0.0.1:5353".parse().unwrap());
        let mut encoder = XudpCodec::client();
        let mut buf = BytesMut::new();
        encoder
            .encode((Bytes::from_static(b"one"), addr.clone()), &mut buf)
            .unwrap();
        encoder
            .encode((Bytes::from_static(b"two"), addr.clone()), &mut buf)
            .unwrap();

        let mut decoder = XudpCodec::server();
        let (first_payload, first_addr) = decoder.decode(&mut buf).unwrap().unwrap();
        assert_eq!(&first_payload[..], b"one");
        assert_eq!(first_addr, addr);

        let (second_payload, second_addr) = decoder.decode(&mut buf).unwrap().unwrap();
        assert_eq!(&second_payload[..], b"two");
        assert_eq!(second_addr, addr);
    }
}

impl Decoder for XudpCodec {
    type Item = (BytesMut, Address);
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Self::Item>> {
        if src.len() < 2 {
            return Ok(None);
        }
        let meta_len = u16::from_be_bytes([src[0], src[1]]) as usize;
        if meta_len < 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid xudp metadata length",
            ));
        }
        if src.len() < 2 + meta_len + 2 {
            return Ok(None);
        }

        let packet_type = src[4];
        if packet_type == 4 {
            src.advance(2 + meta_len);
            let payload_len = u16::from_be_bytes([src[0], src[1]]) as usize;
            if src.len() < 2 + payload_len {
                return Ok(None);
            }
            src.advance(2 + payload_len);
            return Ok(None);
        }
        if packet_type != 1 && packet_type != 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported xudp packet type {packet_type}"),
            ));
        }

        let mut meta = src.split_to(2 + meta_len);
        meta.advance(2);
        let _session_id = meta.get_u16();
        let packet_type = meta.get_u8();
        let _option = meta.get_u8();
        let addr = if meta.is_empty() {
            self.last_addr.clone().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "xudp compact packet missing prior address",
                )
            })?
        } else {
            let network = meta.get_u8();
            if network != 2 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported xudp network type",
                ));
            }
            let addr = read_address_buf(&mut meta)?;
            if packet_type == 1 || packet_type == 2 {
                self.last_addr = Some(addr.clone());
            }
            addr
        };
        if src.len() < 2 {
            return Ok(None);
        }
        let payload_len = u16::from_be_bytes([src[0], src[1]]) as usize;
        if src.len() < 2 + payload_len {
            return Ok(None);
        }
        src.advance(2);
        let payload = src.split_to(payload_len);
        Ok(Some((payload, addr)))
    }
}

pub async fn relay_stream_udp<S>(stream: S, net: rd_interface::Net) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut source) =
        tokio_util::codec::Framed::new(stream, XudpCodec::server()).split();
    let mut udp = net
        .udp_bind(
            &mut rd_interface::Context::new(),
            &Address::SocketAddr("0.0.0.0:0".parse().unwrap()),
        )
        .await
        .map_err(rd_interface::Error::to_io_err)?;

    let mut buf = vec![0u8; 65535];
    loop {
        tokio::select! {
            item = source.next() => {
                match item {
                    Some(Ok((data, target))) => {
                        udp.send_to(&data, &target).await.map_err(rd_interface::Error::to_io_err)?;
                    }
                    Some(Err(err)) => return Err(err),
                    None => return Ok(()),
                }
            }
            recv = async {
                let mut rb = ReadBuf::new(&mut buf);
                let from = udp.recv_from(&mut rb).await.map_err(rd_interface::Error::to_io_err)?;
                Ok::<_, io::Error>((rb.filled().to_vec(), from))
            } => {
                let (data, from) = recv?;
                sink.send((Bytes::from(data), from.into())).await?;
            }
        }
    }
}

pub fn load_cert_and_key(
    cert_path: &str,
    key_path: &str,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let cert_chain = fs::read(Path::new(cert_path)).map_err(map_other)?;
    let certs = rustls_pemfile::certs(&mut &*cert_chain)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(map_other)?;
    let key = fs::read(Path::new(key_path)).map_err(map_other)?;
    let key = rustls_pemfile::private_key(&mut &*key)
        .map_err(map_other)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no private keys found"))
        .map_err(map_other)?;
    Ok((certs, key))
}

pub fn build_server_config(cert_path: &str, key_path: &str) -> Result<ServerConfig> {
    let (certs, key) = load_cert_and_key(cert_path, key_path)?;
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(map_other)
}

fn write_addons(buf: &mut Vec<u8>, flow: Option<&str>) -> io::Result<()> {
    match flow {
        None => buf.push(0),
        Some(flow) => {
            if flow != FLOW_VISION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsupported flow",
                ));
            }
            if flow.len() > 127 {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "flow too long"));
            }
            buf.push((flow.len() + 2) as u8);
            buf.push(0x0a);
            buf.push(flow.len() as u8);
            buf.extend_from_slice(flow.as_bytes());
        }
    }
    Ok(())
}

async fn read_addons<S>(stream: &mut S) -> io::Result<NormalizedFlow>
where
    S: AsyncRead + Unpin,
{
    let mut len = [0u8; 1];
    stream.read_exact(&mut len).await?;
    if len[0] == 0 {
        return Ok(NormalizedFlow(None));
    }
    let mut buf = vec![0u8; len[0] as usize];
    stream.read_exact(&mut buf).await?;
    if buf.len() < 2 || buf[0] != 0x0a {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported vless addons",
        ));
    }
    let str_len = buf[1] as usize;
    if buf.len() != str_len + 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid vless flow addons",
        ));
    }
    NormalizedFlow::parse(Some(
        String::from_utf8(buf[2..].to_vec())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
    ))
    .map_err(Into::into)
}

fn write_address(buf: &mut Vec<u8>, addr: &Address) -> io::Result<()> {
    match addr {
        Address::SocketAddr(socket) => match socket.ip() {
            std::net::IpAddr::V4(ip) => {
                buf.extend_from_slice(&socket.port().to_be_bytes());
                buf.push(ADDRESS_TYPE_IPV4);
                buf.extend_from_slice(&ip.octets());
            }
            std::net::IpAddr::V6(ip) => {
                buf.extend_from_slice(&socket.port().to_be_bytes());
                buf.push(ADDRESS_TYPE_IPV6);
                buf.extend_from_slice(&ip.octets());
            }
        },
        Address::Domain(domain, port) => {
            if domain.len() > 255 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "domain too long",
                ));
            }
            buf.extend_from_slice(&port.to_be_bytes());
            buf.push(ADDRESS_TYPE_DOMAIN);
            buf.push(domain.len() as u8);
            buf.extend_from_slice(domain.as_bytes());
        }
    }
    Ok(())
}

async fn read_address<S>(stream: &mut S) -> io::Result<Address>
where
    S: AsyncRead + Unpin,
{
    let mut port = [0u8; 2];
    stream.read_exact(&mut port).await?;
    let mut atyp = [0u8; 1];
    stream.read_exact(&mut atyp).await?;
    let port = u16::from_be_bytes(port);
    match atyp[0] {
        ADDRESS_TYPE_IPV4 => {
            let mut ip = [0u8; 4];
            stream.read_exact(&mut ip).await?;
            Ok(std::net::SocketAddr::from((ip, port)).into())
        }
        ADDRESS_TYPE_IPV6 => {
            let mut ip = [0u8; 16];
            stream.read_exact(&mut ip).await?;
            Ok(std::net::SocketAddr::from((ip, port)).into())
        }
        ADDRESS_TYPE_DOMAIN => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut domain = vec![0u8; len[0] as usize];
            stream.read_exact(&mut domain).await?;
            Ok(Address::Domain(
                String::from_utf8(domain)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
                port,
            ))
        }
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported address type {other}"),
        )),
    }
}

fn write_address_buf(buf: &mut BytesMut, addr: &Address) -> io::Result<()> {
    let mut tmp = Vec::new();
    write_address(&mut tmp, addr)?;
    buf.extend_from_slice(&tmp);
    Ok(())
}

fn read_address_buf(buf: &mut BytesMut) -> io::Result<Address> {
    if buf.len() < 3 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "short xudp address",
        ));
    }
    let port = buf.get_u16();
    let atyp = buf.get_u8();
    match atyp {
        ADDRESS_TYPE_IPV4 => {
            if buf.len() < 4 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short ipv4"));
            }
            let mut ip = [0u8; 4];
            buf.copy_to_slice(&mut ip);
            Ok(std::net::SocketAddr::from((ip, port)).into())
        }
        ADDRESS_TYPE_IPV6 => {
            if buf.len() < 16 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short ipv6"));
            }
            let mut ip = [0u8; 16];
            buf.copy_to_slice(&mut ip);
            Ok(std::net::SocketAddr::from((ip, port)).into())
        }
        ADDRESS_TYPE_DOMAIN => {
            if buf.is_empty() {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short domain"));
            }
            let len = buf.get_u8() as usize;
            if buf.len() < len {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short domain"));
            }
            let domain = String::from_utf8(buf.split_to(len).to_vec())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            Ok(Address::Domain(domain, port))
        }
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported xudp address type {other}"),
        )),
    }
}
