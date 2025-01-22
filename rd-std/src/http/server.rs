use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hyper::{
    client::conn as client_conn, http, server::conn as server_conn, service::service_fn, Body,
    Method, Request, Response,
};
use rd_interface::{async_trait, Address, Context, IServer, IntoAddress, Net, Result, TcpStream};
use std::net::SocketAddr;
use tracing::instrument;

use crate::ContextExt;

#[derive(Clone)]
pub struct HttpServer {
    net: Net,
    username: Option<String>,
    password: Option<String>,
}

impl HttpServer {
    #[instrument(err, skip(self, socket))]
    pub async fn serve_connection(self, socket: TcpStream, addr: SocketAddr) -> anyhow::Result<()> {
        let net = self.net.clone();

        server_conn::Http::new()
            .http1_preserve_header_case(true)
            .http1_title_case_headers(true)
            .http1_keep_alive(true)
            .serve_connection(
                socket,
                service_fn(move |req| {
                    proxy(
                        net.clone(),
                        req,
                        addr,
                        self.username.clone(),
                        self.password.clone(),
                    )
                }),
            )
            .with_upgrades()
            .await?;

        Ok(())
    }

    pub fn new(net: Net) -> Self {
        Self {
            net,
            username: None,
            password: None,
        }
    }

    pub fn with_auth(net: Net, username: String, password: String) -> Self {
        Self {
            net,
            username: Some(username),
            password: Some(password),
        }
    }
}

pub struct Http {
    server: HttpServer,
    listen_net: Net,
    bind: Address,
}

#[async_trait]
impl IServer for Http {
    async fn start(&self) -> Result<()> {
        let listener = self
            .listen_net
            .tcp_bind(&mut Context::new(), &self.bind)
            .await?;

        loop {
            let (socket, addr) = listener.accept().await?;
            let server = self.server.clone();
            tokio::spawn(async move {
                if let Err(e) = server.serve_connection(socket, addr).await {
                    tracing::error!("Error when serve_connection: {:?}", e);
                }
            });
        }
    }
}

impl Http {
    pub fn new(listen_net: Net, net: Net, bind: Address) -> Self {
        Http {
            server: HttpServer::new(net),
            listen_net,
            bind,
        }
    }

    pub fn with_auth(
        listen_net: Net,
        net: Net,
        bind: Address,
        username: String,
        password: String,
    ) -> Self {
        Http {
            server: HttpServer::with_auth(net, username, password),
            listen_net,
            bind,
        }
    }
}

fn verify_auth(
    auth_header: Option<&str>,
    username: Option<String>,
    password: Option<String>,
) -> bool {
    if username.is_none() || password.is_none() {
        return true; // 如果没有设置认证信息，则允许所有请求
    }

    if let Some(auth) = auth_header {
        if let Some(credentials) = auth.strip_prefix("Basic ") {
            if let Ok(decoded) = BASE64.decode(credentials) {
                if let Ok(auth_str) = String::from_utf8(decoded) {
                    let parts: Vec<&str> = auth_str.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        return parts[0] == username.unwrap() && parts[1] == password.unwrap();
                    }
                }
            }
        }
    }
    false
}

// Headers that must be removed when proxying
const HOP_BY_HOP_HEADERS: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

async fn proxy(
    net: Net,
    mut req: Request<Body>,
    addr: SocketAddr,
    username: Option<String>,
    password: Option<String>,
) -> anyhow::Result<Response<Body>> {
    if !verify_auth(
        req.headers()
            .get(hyper::http::header::PROXY_AUTHORIZATION)
            .map(|h| h.to_str().unwrap_or("")),
        username.clone(),
        password.clone(),
    ) {
        let resp = Response::builder()
            .status(http::StatusCode::PROXY_AUTHENTICATION_REQUIRED)
            .header(
                hyper::header::PROXY_AUTHENTICATE,
                "Basic realm=\"HTTP Proxy\"",
            );

        let resp = resp.body(Body::from("Proxy authentication required"))?;
        return Ok(resp);
    }

    let uri = req.uri();
    if let Some(mut dst) = host_addr(uri) {
        if !dst.contains(':') {
            dst += ":80"
        }
        let dst = dst.into_address()?;

        // For CONNECT requests
        if req.method() == Method::CONNECT {
            tokio::spawn(async move {
                match hyper::upgrade::on(req).await {
                    Ok(upgraded) => {
                        let mut ctx = Context::from_socketaddr(addr);
                        let stream = net.tcp_connect(&mut ctx, &dst).await?;
                        if let Err(e) = ctx.connect_tcp(stream, upgraded).await {
                            tracing::debug!("tunnel io error: {}", e);
                        };
                    }
                    Err(e) => tracing::debug!("upgrade error: {}", e),
                }
                Ok(()) as anyhow::Result<()>
            });

            let resp = Response::builder().status(http::StatusCode::OK);
            Ok(resp.body(Body::empty())?)
        } else {
            // For non-CONNECT requests
            // Ensure absolute-form URI
            if !uri.scheme().is_some() && !uri.authority().is_some() {
                if let Some(host) = req.headers().get(hyper::header::HOST) {
                    *req.uri_mut() = format!(
                        "http://{}{}",
                        host.to_str()?,
                        uri.path_and_query().map_or("", |p| p.as_str())
                    )
                    .parse()?;
                } else {
                    let resp = Response::builder().status(http::StatusCode::BAD_REQUEST);
                    return Ok(resp.body(Body::from("Bad Request: Missing Host header"))?);
                }
            }

            // Remove hop-by-hop headers
            let headers = req.headers_mut();
            for header in HOP_BY_HOP_HEADERS.iter() {
                headers.remove(*header);
            }

            // Remove headers mentioned in Connection header
            let connection_headers: Vec<String> =
                if let Some(connection) = headers.get(hyper::header::CONNECTION) {
                    if let Ok(connection_header) = connection.to_str() {
                        connection_header
                            .split(',')
                            .map(|h| h.trim().to_string())
                            .collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };

            for header in connection_headers {
                headers.remove(header.as_str());
            }

            let stream = net
                .tcp_connect(&mut Context::from_socketaddr(addr), &dst)
                .await?;

            let (mut request_sender, connection) = client_conn::Builder::new()
                .http1_preserve_header_case(true)
                .http1_title_case_headers(true)
                .handshake(stream)
                .await?;

            tokio::spawn(connection);

            let mut resp = request_sender.send_request(req).await?;

            // Remove hop-by-hop headers from response
            let headers = resp.headers_mut();
            for header in HOP_BY_HOP_HEADERS.iter() {
                headers.remove(*header);
            }

            Ok(resp)
        }
    } else {
        tracing::error!("Invalid request URI: {:?}", req.uri());
        let resp = Response::builder().status(http::StatusCode::BAD_REQUEST);

        let resp = resp.body(Body::from("Bad Request: Invalid request URI format"))?;

        Ok(resp)
    }
}

fn host_addr(uri: &http::Uri) -> Option<String> {
    uri.authority().map(|auth| auth.to_string())
}
