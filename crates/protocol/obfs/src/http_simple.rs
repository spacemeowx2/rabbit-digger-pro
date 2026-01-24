use std::{
    io::{self, Cursor, Write},
    pin::Pin,
    task::{Context, Poll},
};

use crate::Obfs;
use base64::{engine::general_purpose::STANDARD, Engine};
use futures::ready;
use pin_project_lite::pin_project;
use rand::prelude::*;
use rand::rng;
use rd_interface::{
    async_trait, prelude::*, rd_config, Address, AsyncWrite, ITcpStream, IntoDyn, ReadBuf, Result,
    TcpStream, NOT_IMPLEMENTED,
};
use tokio::io::AsyncRead;

fn def_method() -> String {
    "GET".to_string()
}

fn def_uri() -> String {
    "/".to_string()
}

#[rd_config]
#[derive(Debug, Clone)]
pub struct HttpSimple {
    #[serde(default = "def_method")]
    method: String,
    #[serde(default = "def_uri")]
    uri: String,
    host: String,
}

impl Obfs for HttpSimple {
    fn tcp_connect(
        &self,
        tcp: TcpStream,
        _ctx: &mut rd_interface::Context,
        _addr: &Address,
    ) -> Result<TcpStream> {
        Ok(Connect::new(tcp, self.clone()).into_dyn())
    }

    fn tcp_accept(&self, _tcp: TcpStream, _addr: std::net::SocketAddr) -> Result<TcpStream> {
        Err(NOT_IMPLEMENTED)
    }
}

enum WriteState {
    Wait,
    Write(Vec<u8>, usize),
    Done,
}

enum ReadState {
    Read(Vec<u8>, usize),
    Write(Vec<u8>, usize),
    Done,
}

pin_project! {
    struct Connect {
        inner: TcpStream,
        write: WriteState,
        read: ReadState,
        param: HttpSimple,
    }
}

impl Connect {
    fn new(tcp: TcpStream, param: HttpSimple) -> Connect {
        Connect {
            inner: tcp,
            write: WriteState::Wait,
            read: ReadState::Read(vec![0u8; 8192], 0),
            param,
        }
    }
}

#[async_trait]
impl ITcpStream for Connect {
    async fn peer_addr(&self) -> Result<std::net::SocketAddr> {
        self.inner.peer_addr().await
    }

    async fn local_addr(&self) -> Result<std::net::SocketAddr> {
        self.inner.local_addr().await
    }

    fn poll_read(&mut self, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        loop {
            match &mut self.read {
                ReadState::Read(ref mut read_buf, pos) => {
                    let mut tmp_buf = ReadBuf::new(&mut read_buf[*pos..]);
                    ready!(Pin::new(&mut self.inner).poll_read(cx, &mut tmp_buf))?;

                    *pos += tmp_buf.filled().len();

                    if let Some(at) = find_subsequence_end(read_buf, b"\r\n\r\n") {
                        read_buf.truncate(*pos);
                        self.read = ReadState::Write(read_buf.split_off(at), 0);
                    }
                }
                ReadState::Write(ref write_buf, pos) => {
                    let remaining = &write_buf[*pos..];

                    let to_read = remaining.len().min(buf.remaining());
                    buf.initialize_unfilled_to(to_read)
                        .copy_from_slice(&remaining[..to_read]);

                    buf.advance(to_read);
                    *pos += to_read;

                    if write_buf.len() == *pos {
                        self.read = ReadState::Done;
                    }
                    return Poll::Ready(Ok(()));
                }
                ReadState::Done => {
                    return Pin::new(&mut self.inner).poll_read(cx, buf);
                }
            }
        }
    }
    fn poll_write(&mut self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, io::Error>> {
        loop {
            match &mut self.write {
                WriteState::Wait => {
                    let major = rng().next_u32() % 51;
                    let minor = rng().next_u32() % 2;

                    let key_bytes: [u8; 16] = rng().random();
                    let key = STANDARD.encode(key_bytes);

                    let mut cursor = Cursor::new(Vec::<u8>::with_capacity(1024));
                    cursor.write_fmt(format_args!(
                        "{method} {path} HTTP/1.1\r
Host: {host}\r
User-Agent: curl/7.{major}.{minor}\r
Upgrade: websocket\r
Connection: Upgrade\r
Sec-WebSocket-Key: {key}\r
Content-Length: {len}\r
\r\n",
                        method = self.param.method,
                        path = self.param.uri,
                        host = self.param.host,
                        len = buf.len(),
                    ))?;
                    cursor.write_all(buf)?;

                    let buf = cursor.into_inner();

                    self.write = WriteState::Write(buf, 0);
                }
                WriteState::Write(ref buf, pos) => {
                    let wrote = ready!(Pin::new(&mut self.inner).poll_write(cx, &buf[*pos..]))?;
                    *pos += wrote;

                    if buf.len() == *pos {
                        self.write = WriteState::Done;
                    }
                }
                WriteState::Done => {
                    return Pin::new(&mut self.inner).poll_write(cx, buf);
                }
            };
        }
    }

    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

fn find_subsequence_end(array: &[u8], pattern: &[u8]) -> Option<usize> {
    array
        .windows(pattern.len())
        .position(|window| window == pattern)
        .map(|at| at + pattern.len())
}
