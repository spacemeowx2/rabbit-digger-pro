use std::{fs, net::SocketAddr, path::Path, sync::Arc};

use bytes::Bytes;
use futures_util::future::poll_fn;
use h3::client::SendRequest;
use http::{Request, StatusCode};
use quinn::crypto::rustls::QuicClientConfig;
use rd_interface::{
    async_trait, error::map_other, prelude::*, registry::NetRef, Address, Error, INet, IntoDyn,
    Net, Result, TcpStream, UdpSocket,
};
use tokio::sync::OnceCell;

use crate::{codec::write_tcp_request, stream::Hy2Stream, transport, udp::Hy2Udp};

#[rd_config]
#[derive(Debug, Clone)]
pub struct HysteriaNetConfig {
    pub(crate) server: Address,
    #[serde(skip_serializing_if = "rd_interface::config::detailed_field")]
    pub(crate) auth: String,

    #[serde(default)]
    pub(crate) server_name: Option<String>,

    /// Trust additional CA cert(s) for the server (PEM).
    #[serde(default)]
    pub(crate) ca_pem: Option<String>,

    /// Local bind address for QUIC (e.g. `[::]:0`).
    #[serde(default)]
    pub(crate) bind: Option<String>,

    /// Enable Salamander obfuscation (shared key).
    #[serde(default)]
    pub(crate) salamander: Option<String>,

    #[serde(default)]
    pub(crate) udp: bool,

    #[serde(default)]
    pub(crate) padding: bool,

    #[serde(default)]
    pub(crate) net: NetRef,
}

pub struct HysteriaNet {
    cfg: HysteriaNetConfig,
    net: Net,
    client: OnceCell<Arc<Hy2Client>>,
}

struct Hy2Client {
    _endpoint: quinn::Endpoint,
    conn: quinn::Connection,
    _h3_driver: tokio::task::JoinHandle<()>,
    _h3_send: SendRequest<h3_quinn::OpenStreams, Bytes>,
    udp: Option<Arc<Hy2Udp>>,
}

impl HysteriaNet {
    pub fn new(cfg: HysteriaNetConfig) -> Result<Self> {
        Ok(Self {
            net: cfg.net.value_cloned(),
            cfg,
            client: OnceCell::new(),
        })
    }

    async fn get_client(&self) -> Result<Arc<Hy2Client>> {
        self.client
            .get_or_try_init(|| async {
                let client = Hy2Client::connect(&self.cfg, &self.net).await?;
                Ok(Arc::new(client))
            })
            .await
            .map(Clone::clone)
    }
}

impl Hy2Client {
    async fn connect(cfg: &HysteriaNetConfig, net: &Net) -> Result<Self> {
        let server_addr = resolve_server_addr(net, &cfg.server).await?;
        let server_name = resolve_server_name(cfg)?;

        let roots = load_roots(cfg.ca_pem.as_deref())?;
        let mut client_crypto = quinn::rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_crypto.alpn_protocols = vec![b"h3".to_vec(), b"h3-29".to_vec()];

        let mut client_config = quinn::ClientConfig::new(Arc::new(
            QuicClientConfig::try_from(client_crypto).map_err(map_other)?,
        ));

        let mut transport_config = quinn::TransportConfig::default();
        transport_config.datagram_receive_buffer_size(Some(1024 * 1024));
        transport_config.datagram_send_buffer_size(1024 * 1024);
        client_config.transport_config(Arc::new(transport_config));

        let bind: SocketAddr = cfg
            .bind
            .as_deref()
            .unwrap_or("[::]:0")
            .parse()
            .map_err(map_other)?;

        let salamander_key = cfg
            .salamander
            .as_ref()
            .map(|s| Arc::new(s.as_bytes().to_vec()));

        let quinn_sock = transport::make_quinn_socket(bind, salamander_key).map_err(map_other)?;
        let runtime: Arc<dyn quinn::Runtime> = Arc::new(quinn::TokioRuntime);
        let mut endpoint = quinn::Endpoint::new_with_abstract_socket(
            quinn::EndpointConfig::default(),
            None,
            quinn_sock,
            runtime,
        )
        .map_err(map_other)?;
        endpoint.set_default_client_config(client_config);

        let conn = endpoint
            .connect(server_addr, &server_name)
            .map_err(map_other)?
            .await
            .map_err(map_other)?;

        let (h3_driver, h3_send, udp_supported) = hy2_auth(cfg, &conn).await?;
        let udp =
            (cfg.udp && udp_supported).then(|| Arc::new(Hy2Udp::new(conn.clone(), net.clone())));

        Ok(Self {
            _endpoint: endpoint,
            conn,
            _h3_driver: h3_driver,
            _h3_send: h3_send,
            udp,
        })
    }
}

async fn hy2_auth(
    cfg: &HysteriaNetConfig,
    conn: &quinn::Connection,
) -> Result<(
    tokio::task::JoinHandle<()>,
    SendRequest<h3_quinn::OpenStreams, Bytes>,
    bool,
)> {
    let quinn_conn = h3_quinn::Connection::new(conn.clone());
    let (mut h3_conn, mut send_request) = h3::client::new(quinn_conn).await.map_err(map_other)?;

    // Keep the H3 connection driven in the background; otherwise the request/response
    // machinery may stall.
    let driver = tokio::spawn(async move {
        let _ = poll_fn(|cx| h3_conn.poll_close(cx)).await;
    });

    let mut builder = Request::builder()
        .method("POST")
        .uri("https://hysteria/auth");

    builder = builder.header("authorization", cfg.auth.as_str());
    builder = builder.header("hysteria-udp", if cfg.udp { "true" } else { "false" });
    builder = builder.header(
        "hysteria-padding",
        if cfg.padding { "true" } else { "false" },
    );

    let req = builder.body(()).map_err(map_other)?;

    let mut stream = send_request.send_request(req).await.map_err(map_other)?;
    stream.finish().await.map_err(map_other)?;
    let resp = stream.recv_response().await.map_err(map_other)?;

    if resp.status() != StatusCode::from_u16(233).unwrap() {
        return Err(Error::Other(
            format!("HY2 auth failed: status={}", resp.status()).into(),
        ));
    }

    let udp_supported = resp
        .headers()
        .get("hysteria-udp")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    Ok((driver, send_request, udp_supported))
}

fn resolve_server_name(cfg: &HysteriaNetConfig) -> Result<String> {
    if let Some(sni) = &cfg.server_name {
        return Ok(sni.clone());
    }
    match &cfg.server {
        Address::Domain(host, _) => Ok(host.clone()),
        Address::SocketAddr(_) => Err(Error::Other(
            "hysteria: server_name is required when server is an IP".into(),
        )),
    }
}

async fn resolve_server_addr(net: &Net, server: &Address) -> Result<SocketAddr> {
    match server {
        Address::SocketAddr(sa) => Ok(*sa),
        Address::Domain(_, _) => net
            .lookup_host(server)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Other("hysteria: failed to resolve server address".into())),
    }
}

fn load_roots(ca_pem: Option<&str>) -> Result<quinn::rustls::RootCertStore> {
    let mut roots = quinn::rustls::RootCertStore::empty();

    if let Some(pem_path) = ca_pem {
        let data = fs::read(Path::new(pem_path)).map_err(map_other)?;
        let mut cursor = std::io::Cursor::new(data);
        let certs = rustls_pemfile::certs(&mut cursor).collect::<std::io::Result<Vec<_>>>()?;
        for cert in certs {
            roots.add(cert).map_err(map_other)?;
        }
        return Ok(roots);
    }

    let native = rustls_native_certs::load_native_certs();
    for cert in native.certs {
        roots.add(cert).map_err(map_other)?;
    }
    Ok(roots)
}

#[async_trait]
impl rd_interface::TcpConnect for HysteriaNet {
    async fn tcp_connect(
        &self,
        ctx: &mut rd_interface::Context,
        addr: &Address,
    ) -> Result<TcpStream> {
        let _ = ctx;
        let client = self.get_client().await?;
        let (mut send, recv) = client.conn.open_bi().await.map_err(map_other)?;

        let mut req = Vec::with_capacity(64);
        write_tcp_request(&mut req, addr);
        send.write_all(&req).await.map_err(map_other)?;

        Ok(Hy2Stream { send, recv }.into_dyn())
    }
}

#[async_trait]
impl rd_interface::UdpBind for HysteriaNet {
    async fn udp_bind(
        &self,
        ctx: &mut rd_interface::Context,
        _addr: &Address,
    ) -> Result<UdpSocket> {
        let _ = ctx;
        if !self.cfg.udp {
            return Err(Error::NotEnabled);
        }
        let client = self.get_client().await?;
        let udp = client.udp.as_ref().ok_or(Error::NotEnabled)?;
        Ok(udp.bind_socket())
    }
}

impl INet for HysteriaNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use rd_interface::IntoAddress;
    use rd_std::tests::{assert_net_provider, ProviderCapability, TestNet};

    use super::*;

    #[test]
    fn test_provider() {
        let net = TestNet::new().into_dyn();

        let hy2 = HysteriaNet::new(HysteriaNetConfig {
            server: "127.0.0.1:18443".into_address().unwrap(),
            auth: "test-password".to_string(),
            server_name: Some("localhost".to_string()),
            ca_pem: None,
            bind: None,
            salamander: None,
            udp: false,
            padding: false,
            net: NetRef::new_with_value("test".into(), net),
        })
        .unwrap()
        .into_dyn();

        assert_net_provider(
            &hy2,
            ProviderCapability {
                tcp_connect: true,
                udp_bind: true,
                ..Default::default()
            },
        );
    }
}
