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

async fn proxy(
    net: Net,
    req: Request<Body>,
    addr: SocketAddr,
    username: Option<String>,
    password: Option<String>,
) -> anyhow::Result<Response<Body>> {
    // 验证认证信息
    if !verify_auth(
        req.headers()
            .get("Authorization")
            .map(|h| h.to_str().unwrap_or("")),
        username,
        password,
    ) {
        let mut resp = Response::new(Body::from("Unauthorized"));
        *resp.status_mut() = http::StatusCode::UNAUTHORIZED;
        resp.headers_mut().insert(
            "WWW-Authenticate",
            http::HeaderValue::from_static("Basic realm=\"Proxy Authentication Required\""),
        );
        return Ok(resp);
    }

    if let Some(mut dst) = host_addr(req.uri()) {
        if !dst.contains(':') {
            dst += ":80"
        }
        let dst = dst.into_address()?;

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

            Ok(Response::new(Body::empty()))
        } else {
            let stream = net
                .tcp_connect(&mut Context::from_socketaddr(addr), &dst)
                .await?;

            let (mut request_sender, connection) = client_conn::Builder::new()
                .http1_preserve_header_case(true)
                .http1_title_case_headers(true)
                .handshake(stream)
                .await?;

            tokio::spawn(connection);

            let resp = request_sender.send_request(req).await?;

            Ok(resp)
        }
    } else {
        tracing::error!("host is not socket addr: {:?}", req.uri());
        let mut resp = Response::new(Body::from("CONNECT must be to a socket address"));
        *resp.status_mut() = http::StatusCode::BAD_REQUEST;

        Ok(resp)
    }
}

fn host_addr(uri: &http::Uri) -> Option<String> {
    uri.authority().map(|auth| auth.to_string())
}
