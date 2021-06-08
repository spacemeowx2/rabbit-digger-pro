use core::fmt;
use std::{
    io::{self, Cursor, Write},
    mem::replace,
    pin::Pin,
    task::{Context, Poll},
};

use crate::Obfs;
use futures::ready;
use rand::prelude::*;
use rd_interface::{
    async_trait,
    schemars::{self, JsonSchema},
    Address, AsyncWrite, Config, ITcpStream, IntoDyn, ReadBuf, Result, TcpStream, NOT_IMPLEMENTED,
};
use serde_derive::{Deserialize, Serialize};
use tokio::io::AsyncRead;

#[derive(Debug, Serialize, Deserialize, Config, JsonSchema)]
pub struct HttpSimple {
    obfs_param: String,
}

impl Obfs for HttpSimple {
    fn tcp_connect(
        &self,
        tcp: TcpStream,
        _ctx: &mut rd_interface::Context,
        _addr: Address,
    ) -> Result<TcpStream> {
        Ok(Connect::new(tcp, &self.obfs_param).into_dyn())
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

struct Connect {
    inner: TcpStream,
    write: WriteState,
    read: ReadState,
    obfs_param: String,
}

impl Connect {
    fn new(tcp: TcpStream, param: &str) -> Connect {
        Connect {
            inner: tcp,
            write: WriteState::Wait,
            read: ReadState::Read(vec![0u8; 8192], 0),
            obfs_param: param.to_string(),
        }
    }
}

impl AsyncRead for Connect {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            let read = replace(&mut self.read, ReadState::Done);
            self.read = match read {
                ReadState::Read(mut read_buf, pos) => {
                    let mut tmp_buf = ReadBuf::new(&mut read_buf[pos..]);
                    ready!(Pin::new(&mut self.inner).poll_read(cx, &mut tmp_buf))?;
                    let new_pos = pos + tmp_buf.filled().len();

                    if let Some(at) = find_subsequence(&read_buf, b"\r\n\r\n") {
                        ReadState::Write(read_buf.split_off(at), 0)
                    } else {
                        ReadState::Read(read_buf, new_pos)
                    }
                }
                ReadState::Write(write_buf, pos) => {
                    let remaining = &write_buf[pos..];
                    let unfilled = buf.initialize_unfilled();
                    let to_read = remaining.len().min(unfilled.len());
                    unfilled.copy_from_slice(&write_buf[pos..pos + to_read]);
                    let new_pos = pos + to_read;

                    if write_buf.len() == new_pos {
                        ReadState::Done
                    } else {
                        ReadState::Write(write_buf, new_pos)
                    }
                }
                ReadState::Done => return Pin::new(&mut self.inner).poll_read(cx, buf),
            }
        }
    }
}

struct UrlEncode<'a>(&'a [u8]);

impl<'a> fmt::Display for UrlEncode<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for i in self.0 {
            write!(f, "%{:02x}", i)?;
        }
        Ok(())
    }
}

impl AsyncWrite for Connect {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        loop {
            let write = replace(&mut self.write, WriteState::Done);
            self.write = match write {
                WriteState::Wait => {
                    let head_len = thread_rng().gen_range(0..64usize).max(buf.len());
                    let head = &buf[..head_len];
                    let body = &buf[head_len..];

                    let mut cursor = Cursor::new(Vec::<u8>::with_capacity(1024));
                    cursor.write_fmt(format_args!(
                        "GET /{path} HTTP/1.1\r\nHost: {host}\r\n\r\n",
                        path = UrlEncode(head),
                        host = self.obfs_param
                    ))?;
                    cursor.write_all(body)?;

                    let buf = cursor.into_inner();

                    WriteState::Write(buf, 0)
                }
                WriteState::Write(buf, pos) => {
                    let wrote = ready!(Pin::new(&mut self.inner).poll_write(cx, &buf[pos..]))?;
                    let new_pos = pos + wrote;

                    if buf.len() == new_pos {
                        WriteState::Done
                    } else {
                        WriteState::Write(buf, new_pos)
                    }
                }
                WriteState::Done => return Pin::new(&mut self.inner).poll_write(cx, buf),
            };
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
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
}

fn find_subsequence(array: &[u8], pattern: &[u8]) -> Option<usize> {
    array
        .windows(pattern.len())
        .position(|window| window == pattern)
}
