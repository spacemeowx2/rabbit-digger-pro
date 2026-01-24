use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rd_interface::{async_trait, Address, INet, Net, Result, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct HttpClient {
    server: Address,
    net: Net,
    username: Option<String>,
    password: Option<String>,
}

#[async_trait]
impl rd_interface::TcpConnect for HttpClient {
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &rd_interface::Address,
    ) -> Result<TcpStream> {
        let socket = self.net.tcp_connect(ctx, &self.server).await?;
        let mut socket = socket;

        let mut req = format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\n", addr, addr);

        // 添加认证头
        if let (Some(username), Some(password)) = (&self.username, &self.password) {
            let auth = format!("{}:{}", username, password);
            let encoded = BASE64.encode(auth);
            req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", encoded));
        }

        req.push_str("\r\n");

        socket.write_all(req.as_bytes()).await?;

        // Read response headers
        let mut buf = Vec::with_capacity(1024);
        let mut tmp = [0u8; 1024];
        loop {
            let n = socket.read(&mut tmp).await?;
            if n == 0 {
                return Err(rd_interface::Error::Other("proxy closed connection".into()));
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            if buf.len() > 64 * 1024 {
                return Err(rd_interface::Error::Other(
                    "proxy response header too large".into(),
                ));
            }
        }

        let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let header_bytes = &buf[..header_end];
        let header_str =
            std::str::from_utf8(header_bytes).map_err(|e| rd_interface::Error::Other(e.into()))?;

        let status_line = header_str.lines().next().unwrap_or("");
        let mut parts = status_line.split_whitespace();
        let _http = parts.next().unwrap_or("");
        let code = parts.next().unwrap_or("0").parse::<u16>().unwrap_or(0);
        if code != 200 {
            return Err(rd_interface::Error::Other(
                format!("CONNECT failed: {status_line}").into(),
            ));
        }

        // NOTE: 如果响应 header 后面还有多读到的字节，这里暂时丢弃；CONNECT 正常情况下不会有 body。
        Ok(socket)
    }
}

impl INet for HttpClient {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }
}

impl HttpClient {
    pub fn new(net: Net, server: Address) -> Self {
        Self {
            server,
            net,
            username: None,
            password: None,
        }
    }

    pub fn with_auth(net: Net, server: Address, username: String, password: String) -> Self {
        Self {
            server,
            net,
            username: Some(username),
            password: Some(password),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::{assert_net_provider, ProviderCapability, TestNet};
    use rd_interface::{IntoAddress, IntoDyn};

    use super::*;

    #[test]
    fn test_provider() {
        let net = TestNet::new().into_dyn();

        let http = HttpClient::new(net, "127.0.0.1:12345".into_address().unwrap()).into_dyn();

        assert_net_provider(
            &http,
            ProviderCapability {
                tcp_connect: true,
                ..Default::default()
            },
        );
    }
}
