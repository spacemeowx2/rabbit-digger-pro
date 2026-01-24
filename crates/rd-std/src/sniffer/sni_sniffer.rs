use std::{
    fmt, io,
    mem::replace,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{task::AtomicWaker, FutureExt};
use rd_interface::{
    async_trait, Address, AsyncRead, AsyncWrite, INet, IntoDyn, Net, Result, NOT_IMPLEMENTED,
};
use tls_parser::{
    parse_tls_client_hello_extensions, parse_tls_plaintext, SNIType, TlsExtension, TlsMessage,
    TlsMessageHandshake,
};
use tokio::{
    io::AsyncWriteExt,
    task::{spawn, JoinHandle},
    time::{sleep, Sleep},
};

const BUFFER_SIZE: usize = 1024;

pub struct SNISnifferNet {
    net: Net,
    ports: Option<Vec<u16>>,
    force_sniff: bool,
}

impl SNISnifferNet {
    pub fn new(net: Net, ports: Option<Vec<u16>>, force_sniff: bool) -> Self {
        Self {
            net,
            ports,
            force_sniff,
        }
    }
}

impl INet for SNISnifferNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_tcp_bind(&self) -> Option<&dyn rd_interface::TcpBind> {
        self.net.provide_tcp_bind()
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        self.net.provide_udp_bind()
    }

    fn provide_lookup_host(&self) -> Option<&dyn rd_interface::LookupHost> {
        self.net.provide_lookup_host()
    }
}

#[async_trait]
impl rd_interface::TcpConnect for SNISnifferNet {
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> Result<rd_interface::TcpStream> {
        let mut need_sniff = addr.is_socket_addr() || self.force_sniff;

        need_sniff &= match &self.ports {
            Some(ports) => ports.contains(&addr.port()),
            None => addr.port() == 443,
        };

        if need_sniff {
            let tcp = SnifferTcp::new(addr, ConnectSendParam::new(self.net.clone(), ctx));
            return Ok(tcp.into_dyn());
        }

        self.net.tcp_connect(ctx, addr).await
    }
}

struct ConnectSendParam {
    net: Net,
    ctx: rd_interface::Context,
    buffer: Vec<u8>,
}

impl ConnectSendParam {
    fn new(net: Net, ctx: &mut rd_interface::Context) -> Self {
        Self {
            net,
            ctx: ctx.clone(),
            buffer: Vec::with_capacity(BUFFER_SIZE),
        }
    }
    // return the length of buf stored into buffer
    // the buffer limit is BUFFER_SIZE
    fn extend_buffer(&mut self, buf: &[u8]) -> usize {
        let to_copy = buf.len().min(BUFFER_SIZE - self.buffer.len());
        self.buffer.extend_from_slice(&buf[..to_copy]);
        to_copy
    }
}

async fn connect_send(
    net: Net,
    mut ctx: rd_interface::Context,
    addr: Address,
    buffer: Vec<u8>,
) -> io::Result<rd_interface::TcpStream> {
    let mut tcp = net.tcp_connect(&mut ctx, &addr).await?;

    tcp.write_all(&buffer).await?;

    Ok(tcp)
}

enum State {
    WaitingHandshake {
        addr: Address,
        param: ConnectSendParam,
        timeout: Pin<Box<Sleep>>,
    },
    Connecting {
        future: JoinHandle<io::Result<rd_interface::TcpStream>>,
    },
    Connected {
        stream: rd_interface::TcpStream,
    },
}

impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            State::WaitingHandshake { .. } => write!(f, "WaitingHandshake"),
            State::Connecting { .. } => write!(f, "Connecting"),
            State::Connected { .. } => write!(f, "Connected"),
        }
    }
}

struct SnifferTcp {
    state: State,
    receive_notify: AtomicWaker,
    connected_notify: AtomicWaker,
}

impl Drop for SnifferTcp {
    fn drop(&mut self) {
        match &mut self.state {
            State::Connecting { future } => {
                future.abort();
            }
            _ => {}
        }
    }
}

impl SnifferTcp {
    fn new(addr: &Address, param: ConnectSendParam) -> Self {
        Self {
            state: State::WaitingHandshake {
                addr: addr.clone(),
                param,
                timeout: Box::pin(sleep(Duration::from_millis(250))),
            },
            receive_notify: AtomicWaker::new(),
            connected_notify: AtomicWaker::new(),
        }
    }
}

#[async_trait]
impl rd_interface::ITcpStream for SnifferTcp {
    fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut rd_interface::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            match &mut self.state {
                State::WaitingHandshake {
                    ref addr,
                    ref mut param,
                    timeout,
                } => {
                    if let Some(sni) = get_sni(&param.buffer) {
                        let future = spawn(connect_send(
                            param.net.clone(),
                            param.ctx.clone(),
                            Address::Domain(sni, addr.port()).into_normalized(),
                            replace(&mut param.buffer, Vec::new()),
                        ));
                        self.state = State::Connecting { future };
                        continue;
                    }

                    if timeout.is_elapsed() {
                        let future = connect_send(
                            param.net.clone(),
                            param.ctx.clone(),
                            addr.clone(),
                            replace(&mut param.buffer, Vec::new()),
                        );
                        self.state = State::Connecting {
                            future: spawn(future),
                        };
                        continue;
                    }

                    let _ = timeout.poll_unpin(cx);
                    self.receive_notify.register(cx.waker());

                    return Poll::Pending;
                }
                State::Connecting { ref mut future } => {
                    let stream = futures::ready!(future.poll_unpin(cx))??;
                    self.connected_notify.wake();
                    self.state = State::Connected { stream };
                }
                State::Connected { ref mut stream } => return Pin::new(stream).poll_read(cx, buf),
            }
        }
    }

    fn poll_write(&mut self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        loop {
            match &mut self.state {
                State::WaitingHandshake { ref mut param, .. } => {
                    let copied = param.extend_buffer(buf);
                    self.receive_notify.wake();

                    return Poll::Ready(Ok(copied));
                }
                State::Connecting { .. } => {
                    self.connected_notify.register(cx.waker());
                    return Poll::Pending;
                }
                State::Connected { ref mut stream } => {
                    return Pin::new(stream).poll_write(cx, buf);
                }
            }
        }
    }

    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.state {
            State::Connected { stream } => Pin::new(stream).poll_flush(cx),
            _ => Poll::Ready(Ok(())),
        }
    }

    fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.state {
            State::Connected { stream } => Pin::new(stream).poll_shutdown(cx),
            _ => Poll::Ready(Ok(())),
        }
    }

    async fn peer_addr(&self) -> rd_interface::Result<SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }

    async fn local_addr(&self) -> rd_interface::Result<SocketAddr> {
        Err(NOT_IMPLEMENTED)
    }
}

fn is_valid_domain(domain: &str) -> bool {
    domain
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

fn get_sni(bytes: &[u8]) -> Option<String> {
    let (_, res) = parse_tls_plaintext(&bytes).ok()?;

    res.msg
        .into_iter()
        .filter_map(|m| match m {
            TlsMessage::Handshake(TlsMessageHandshake::ClientHello(ch)) => ch.ext,
            _ => None,
        })
        .filter_map(|ext| parse_tls_client_hello_extensions(ext).ok())
        .map(|i| i.1)
        .flatten()
        .filter_map(|exts| match exts {
            TlsExtension::SNI(sni) => Some(sni),
            _ => None,
        })
        .flatten()
        .filter_map(|(sni_type, data)| {
            if sni_type == SNIType::HostName {
                Some(data)
            } else {
                None
            }
        })
        .filter_map(|data| std::str::from_utf8(data).ok())
        .filter(|s| is_valid_domain(s))
        .map(ToString::to_string)
        .next()
}

#[cfg(test)]
mod tests {
    use super::{get_sni, is_valid_domain};

    const TLS_CLIENT_HELLO: &[u8] = &[
        0x16u8, 0x03, 0x01, 0x02, 0x00, 0x01, 0x00, 0x01, 0xfc, 0x03, 0x03, 0xad, 0x1a, 0xb0, 0x9a,
        0x4e, 0xad, 0xff, 0x80, 0x29, 0x40, 0xbc, 0xf5, 0xb6, 0xc2, 0x1a, 0x4d, 0xb9, 0xad, 0x74,
        0x1c, 0x12, 0x13, 0x8c, 0xf4, 0xaa, 0x1b, 0x39, 0x9b, 0xe8, 0xb6, 0x7d, 0xf7, 0x20, 0x07,
        0x21, 0xd1, 0x6b, 0xb9, 0x66, 0x65, 0xe5, 0x0e, 0x4e, 0x0a, 0xff, 0xb5, 0x77, 0x91, 0xde,
        0x97, 0x44, 0xb8, 0xd4, 0xe6, 0xf9, 0x85, 0x79, 0x23, 0x3e, 0x77, 0x12, 0xfa, 0xb0, 0x2d,
        0x24, 0x00, 0x22, 0x13, 0x01, 0x13, 0x03, 0x13, 0x02, 0xc0, 0x2b, 0xc0, 0x2f, 0xcc, 0xa9,
        0xcc, 0xa8, 0xc0, 0x2c, 0xc0, 0x30, 0xc0, 0x0a, 0xc0, 0x09, 0xc0, 0x13, 0xc0, 0x14, 0x00,
        0x9c, 0x00, 0x9d, 0x00, 0x2f, 0x00, 0x35, 0x01, 0x00, 0x01, 0x91, 0x00, 0x00, 0x00, 0x13,
        0x00, 0x11, 0x00, 0x00, 0x0e, 0x62, 0x2e, 0x62, 0x64, 0x73, 0x74, 0x61, 0x74, 0x69, 0x63,
        0x2e, 0x63, 0x6f, 0x6d, 0x00, 0x17, 0x00, 0x00, 0xff, 0x01, 0x00, 0x01, 0x00, 0x00, 0x0a,
        0x00, 0x0e, 0x00, 0x0c, 0x00, 0x1d, 0x00, 0x17, 0x00, 0x18, 0x00, 0x19, 0x01, 0x00, 0x01,
        0x01, 0x00, 0x0b, 0x00, 0x02, 0x01, 0x00, 0x00, 0x23, 0x00, 0x00, 0x00, 0x10, 0x00, 0x0e,
        0x00, 0x0c, 0x02, 0x68, 0x32, 0x08, 0x68, 0x74, 0x74, 0x70, 0x2f, 0x31, 0x2e, 0x31, 0x00,
        0x05, 0x00, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x00, 0x0a, 0x00, 0x08, 0x04,
        0x03, 0x05, 0x03, 0x06, 0x03, 0x02, 0x03, 0x00, 0x33, 0x00, 0x6b, 0x00, 0x69, 0x00, 0x1d,
        0x00, 0x20, 0x46, 0x37, 0x21, 0x79, 0xd7, 0xfb, 0xd6, 0x5e, 0x6a, 0xde, 0x72, 0x31, 0x60,
        0x73, 0x20, 0x86, 0x38, 0x0f, 0x55, 0xd5, 0xd0, 0xd3, 0x02, 0xbe, 0xd0, 0x47, 0xe6, 0x20,
        0x30, 0x0c, 0xad, 0x3c, 0x00, 0x17, 0x00, 0x41, 0x04, 0xa8, 0x64, 0xf9, 0x24, 0x29, 0x96,
        0xd0, 0x56, 0xfa, 0xf3, 0xeb, 0x92, 0x1d, 0x0a, 0x8b, 0xf6, 0xe6, 0xa7, 0x89, 0x0e, 0x63,
        0x6a, 0xc2, 0xb3, 0xb6, 0x85, 0x65, 0x56, 0x13, 0x5b, 0xa7, 0xbf, 0x02, 0x7f, 0x88, 0xc3,
        0x9f, 0x92, 0xc8, 0x4f, 0x44, 0x1f, 0x8a, 0xc5, 0x76, 0x86, 0x33, 0xd1, 0xdf, 0xf0, 0xc1,
        0x04, 0xde, 0x48, 0xff, 0x1f, 0x86, 0x2a, 0xe4, 0xd7, 0x44, 0x3d, 0x61, 0x0e, 0x00, 0x2b,
        0x00, 0x05, 0x04, 0x03, 0x04, 0x03, 0x03, 0x00, 0x0d, 0x00, 0x18, 0x00, 0x16, 0x04, 0x03,
        0x05, 0x03, 0x06, 0x03, 0x08, 0x04, 0x08, 0x05, 0x08, 0x06, 0x04, 0x01, 0x05, 0x01, 0x06,
        0x01, 0x02, 0x03, 0x02, 0x01, 0x00, 0x2d, 0x00, 0x02, 0x01, 0x01, 0x00, 0x1c, 0x00, 0x02,
        0x40, 0x01, 0x00, 0x15, 0x00, 0x88, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn test_parse_sni_none() {
        assert_eq!(get_sni(&[]), None);
    }

    #[test]
    fn test_parse_sni_ok() {
        assert_eq!(
            get_sni(TLS_CLIENT_HELLO),
            Some("b.bdstatic.com".to_string())
        );
    }

    #[test]
    fn test_mijia_cloud_invalid() {
        assert!(!is_valid_domain("Mijia Cloud"))
    }

    #[test]
    fn test_is_valid_domain() {
        assert!(is_valid_domain("www.google.com"))
    }
}
