use std::{fs, net::SocketAddr, path::Path, sync::Arc};

use bytes::Bytes;
use h3::client::SendRequest;
use http::{Request, StatusCode};
use quinn::crypto::rustls::QuicClientConfig;
use rand::RngCore;
use rd_interface::{
    async_trait, error::map_other, prelude::*, registry::NetRef, Address, Error, INet, IntoDyn,
    Net, Result, TcpStream, UdpSocket,
};
use tokio::sync::OnceCell;

use crate::crypto_provider::ensure_rustls_provider_installed;
use crate::{
    codec::{read_quic_varint, write_tcp_request_with_padding},
    stream::Hy2Stream,
    transport,
    udp::Hy2Udp,
};

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

    /// Client's maximum receive rate in bytes per second. 0 means unknown.
    #[serde(default)]
    pub(crate) cc_rx: u64,

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
    _h3_conn: h3::client::Connection<h3_quinn::Connection, Bytes>,
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
        ensure_rustls_provider_installed();
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

        let default_bind = if server_addr.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let bind: SocketAddr = cfg
            .bind
            .as_deref()
            .unwrap_or(default_bind)
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
            .map_err(|e| Error::Other(format!("quic connect failed: {e:?}").into()))?;

        let (h3_conn, h3_send, udp_supported) = hy2_auth(cfg, &conn)
            .await
            .map_err(|e| attach_close_reason("hy2 auth failed", &conn, e))?;
        let udp =
            (cfg.udp && udp_supported).then(|| Arc::new(Hy2Udp::new(conn.clone(), net.clone())));

        Ok(Self {
            _endpoint: endpoint,
            conn,
            _h3_conn: h3_conn,
            _h3_send: h3_send,
            udp,
        })
    }
}

fn attach_close_reason(context: &str, conn: &quinn::Connection, err: Error) -> Error {
    match conn.close_reason() {
        Some(close_reason) => {
            Error::Other(format!("{context}: {err:?}; close_reason={close_reason:?}").into())
        }
        None => err,
    }
}

async fn hy2_auth(
    cfg: &HysteriaNetConfig,
    conn: &quinn::Connection,
) -> Result<(
    h3::client::Connection<h3_quinn::Connection, Bytes>,
    SendRequest<h3_quinn::OpenStreams, Bytes>,
    bool,
)> {
    let quinn_conn = h3_quinn::Connection::new(conn.clone());
    let (h3_conn, mut send_request) = h3::client::new(quinn_conn)
        .await
        .map_err(|e| Error::Other(format!("h3 client init failed: {e:?}").into()))?;

    let mut builder = Request::builder()
        .method("POST")
        .uri("https://hysteria/auth");

    builder = builder.header("hysteria-auth", cfg.auth.as_str());
    builder = builder.header("hysteria-cc-rx", cfg.cc_rx.to_string());
    if cfg.padding {
        builder = builder.header("hysteria-padding", random_padding_string());
    }

    let req = builder.body(()).map_err(map_other)?;

    let mut stream = send_request
        .send_request(req)
        .await
        .map_err(|e| Error::Other(format!("auth request send failed: {e:?}").into()))?;
    stream
        .finish()
        .await
        .map_err(|e| Error::Other(format!("auth request finish failed: {e:?}").into()))?;
    let resp = stream
        .recv_response()
        .await
        .map_err(|e| Error::Other(format!("auth response receive failed: {e:?}").into()))?;

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

    Ok((h3_conn, send_request, udp_supported))
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
        let (mut send, recv) = client
            .conn
            .open_bi()
            .await
            .map_err(map_other)
            .map_err(|e| {
                attach_close_reason("failed to open bidirectional stream", &client.conn, e)
            })?;

        let mut req = Vec::with_capacity(64);
        let padding = if self.cfg.padding {
            random_padding_bytes(32)
        } else {
            Vec::new()
        };
        write_tcp_request_with_padding(&mut req, addr, &padding);
        send.write_all(&req).await.map_err(map_other)?;

        let mut recv = recv;
        read_tcp_response(&mut recv).await?;
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
            cc_rx: 0,
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

fn random_padding_bytes(len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut out);
    out
}

fn random_padding_string() -> String {
    let mut out = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut out);
    out.iter().map(|b| (b'a' + (b % 26)) as char).collect()
}

async fn read_tcp_response(recv: &mut quinn::RecvStream) -> Result<()> {
    let mut status = [0u8; 1];
    recv.read_exact(&mut status).await.map_err(map_other)?;
    let msg_len = read_quic_varint(recv).await? as usize;
    if msg_len > 0 {
        let mut msg = vec![0u8; msg_len];
        recv.read_exact(&mut msg).await.map_err(map_other)?;
        let _ = String::from_utf8_lossy(&msg);
        let padding_len = read_quic_varint(recv).await? as usize;
        if padding_len > 0 {
            let mut discard = vec![0u8; padding_len.min(64 * 1024)];
            let mut left = padding_len;
            while left > 0 {
                let n = discard.len().min(left);
                recv.read_exact(&mut discard[..n])
                    .await
                    .map_err(map_other)?;
                left -= n;
            }
        }
        if status[0] != 0x00 {
            return Err(Error::Other(
                format!("hysteria tcp error: {}", String::from_utf8_lossy(&msg)).into(),
            ));
        }
        return Ok(());
    }

    let padding_len = read_quic_varint(recv).await? as usize;
    if padding_len > 0 {
        let mut discard = vec![0u8; padding_len.min(64 * 1024)];
        let mut left = padding_len;
        while left > 0 {
            let n = discard.len().min(left);
            recv.read_exact(&mut discard[..n])
                .await
                .map_err(map_other)?;
            left -= n;
        }
    }

    if status[0] != 0x00 {
        return Err(Error::Other("hysteria tcp error".into()));
    }
    Ok(())
}
