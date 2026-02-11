use std::{collections::HashMap, fs, io, net::SocketAddr, path::Path, sync::Arc};

use bytes::Bytes;
use h3::server::RequestStream;
use http::{Request, Response, StatusCode};
use parking_lot::Mutex;
use quinn::crypto::rustls::QuicServerConfig;
use quinn::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rd_interface::{
    async_trait, error::map_other, prelude::*, Address, Context, Error, IServer, IntoAddress,
    IntoDyn, Net, Result, TcpStream,
};
use rd_std::ContextExt;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

use crate::{codec::decode_varint, stream::Hy2Stream, transport};

#[rd_config]
#[derive(Debug, Clone)]
pub struct HysteriaServerConfig {
    pub(crate) bind: Address,

    pub(crate) tls_cert: String,
    pub(crate) tls_key: String,

    #[serde(skip_serializing_if = "rd_interface::config::detailed_field")]
    pub(crate) auth: String,

    #[serde(default)]
    pub(crate) udp: bool,

    #[serde(default)]
    pub(crate) salamander: Option<String>,

    #[serde(default)]
    pub(crate) net: rd_interface::config::NetRef,
}

pub struct HysteriaServer {
    cfg: HysteriaServerConfig,
    net: Net,
}

impl HysteriaServer {
    pub fn new(cfg: HysteriaServerConfig) -> Result<Self> {
        Ok(Self {
            net: cfg.net.value_cloned(),
            cfg,
        })
    }
}

#[async_trait]
impl IServer for HysteriaServer {
    async fn start(&self) -> Result<()> {
        let endpoint = create_endpoint(&self.cfg)?;
        serve_endpoint(endpoint, self.cfg.clone(), self.net.clone()).await
    }
}

pub(crate) fn create_endpoint(cfg: &HysteriaServerConfig) -> Result<quinn::Endpoint> {
    let bind = match &cfg.bind {
        Address::SocketAddr(sa) => *sa,
        Address::Domain(_, _) => {
            return Err(Error::Other(
                "hysteria server: bind must be a socket addr".into(),
            ))
        }
    };

    let (certs, key) = load_cert_and_key(&cfg.tls_cert, &cfg.tls_key)?;
    let mut server_crypto = quinn::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(map_other)?;
    server_crypto.alpn_protocols = vec![b"h3".to_vec(), b"h3-29".to_vec()];

    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(
        QuicServerConfig::try_from(server_crypto).map_err(map_other)?,
    ));
    let transport_config = Arc::get_mut(&mut server_config.transport).unwrap();
    transport_config.datagram_receive_buffer_size(Some(1024 * 1024));
    transport_config.datagram_send_buffer_size(1024 * 1024);

    let salamander_key = cfg
        .salamander
        .as_ref()
        .map(|s| Arc::new(s.as_bytes().to_vec()));
    let sock = transport::make_quinn_socket(bind, salamander_key).map_err(map_other)?;
    let runtime: Arc<dyn quinn::Runtime> = Arc::new(quinn::TokioRuntime);
    let endpoint = quinn::Endpoint::new_with_abstract_socket(
        quinn::EndpointConfig::default(),
        Some(server_config),
        sock,
        runtime,
    )
    .map_err(map_other)?;
    Ok(endpoint)
}

pub(crate) async fn serve_endpoint(
    endpoint: quinn::Endpoint,
    cfg: HysteriaServerConfig,
    net: Net,
) -> Result<()> {
    while let Some(conn) = endpoint.accept().await {
        let net = net.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(conn, cfg, net).await {
                tracing::error!("hysteria server connection error: {:?}", e);
            }
        });
    }
    Ok(())
}

async fn handle_connection(
    conn: quinn::Incoming,
    cfg: HysteriaServerConfig,
    net: Net,
) -> Result<()> {
    let connection = conn.await.map_err(map_other)?;
    let remote = connection.remote_address();
    let quinn_conn = h3_quinn::Connection::new(connection.clone());
    let mut h3_conn = h3::server::builder()
        .build(quinn_conn)
        .await
        .map_err(map_other)?;
    hy2_auth_server(&mut h3_conn, &cfg).await?;

    let udp_handle = if cfg.udp {
        Some(tokio::spawn(hy2_udp_loop(connection.clone(), net.clone())))
    } else {
        None
    };

    loop {
        let stream = connection.accept_bi().await;
        let (send, recv) = match stream {
            Ok(s) => s,
            Err(quinn::ConnectionError::ApplicationClosed { .. }) => break,
            Err(e) => return Err(map_other(e)),
        };
        let net = net.clone();
        tokio::spawn(async move {
            if let Err(e) = hy2_handle_tcp_stream(send, recv, remote, net).await {
                tracing::debug!("hysteria tcp stream error: {:?}", e);
            }
        });
    }

    if let Some(h) = udp_handle {
        h.abort();
    }

    // Keep h3_conn alive. Dropping it would close the underlying QUIC connection.
    let _ = h3_conn;
    Ok(())
}

async fn hy2_auth_server(
    h3_conn: &mut h3::server::Connection<h3_quinn::Connection, Bytes>,
    cfg: &HysteriaServerConfig,
) -> Result<()> {
    loop {
        match h3_conn.accept().await.map_err(map_other)? {
            None => return Err(Error::Other("hysteria: no auth request".into())),
            Some(resolver) => {
                let (req, stream) = resolver.resolve_request().await.map_err(map_other)?;
                if is_auth_request(&req, cfg) {
                    respond_auth_ok(stream, cfg.udp).await?;
                    return Ok(());
                }
                respond_forbidden(stream).await?;
            }
        }
    }
}

fn is_auth_request(req: &Request<()>, cfg: &HysteriaServerConfig) -> bool {
    if req.uri().path() != "/auth" {
        return false;
    }
    if req.uri().authority().map(|a| a.as_str()) != Some("hysteria") {
        return false;
    }
    if let Some(auth) = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
    {
        auth == cfg.auth
    } else {
        false
    }
}

async fn respond_auth_ok(
    mut stream: RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    udp: bool,
) -> Result<()> {
    let response = Response::builder()
        .status(StatusCode::from_u16(233).unwrap())
        .header("hysteria-udp", if udp { "true" } else { "false" })
        .body(())
        .map_err(map_other)?;
    stream.send_response(response).await.map_err(map_other)?;
    stream.finish().await.map_err(map_other)?;
    Ok(())
}

async fn respond_forbidden(
    mut stream: RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
) -> Result<()> {
    let response = Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(())
        .map_err(map_other)?;
    stream.send_response(response).await.map_err(map_other)?;
    stream.finish().await.map_err(map_other)?;
    Ok(())
}

async fn hy2_handle_tcp_stream(
    send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    remote: SocketAddr,
    net: Net,
) -> Result<()> {
    let msg_type = read_quic_varint(&mut recv).await?;
    if msg_type != 0x401 {
        return Err(Error::Other("hysteria: unknown stream type".into()));
    }

    let addr_type = read_quic_varint(&mut recv).await?;
    let target = match addr_type {
        0 => {
            let mut ip = [0u8; 4];
            recv.read_exact(&mut ip).await.map_err(map_other)?;
            let port = read_quic_varint(&mut recv).await? as u16;
            Address::SocketAddr(SocketAddr::from((ip, port)))
        }
        2 => {
            let mut ip = [0u8; 16];
            recv.read_exact(&mut ip).await.map_err(map_other)?;
            let port = read_quic_varint(&mut recv).await? as u16;
            Address::SocketAddr(SocketAddr::from((ip, port)))
        }
        1 => {
            let len = read_quic_varint(&mut recv).await? as usize;
            let mut buf = vec![0u8; len];
            recv.read_exact(&mut buf).await.map_err(map_other)?;
            let domain = String::from_utf8(buf).map_err(map_other)?;
            let port = read_quic_varint(&mut recv).await? as u16;
            Address::Domain(domain, port)
        }
        _ => return Err(Error::Other("hysteria: invalid addr type".into())),
    };

    let mut ctx = Context::from_socketaddr(remote);
    let outbound = net.tcp_connect(&mut ctx, &target).await?;
    let inbound: TcpStream = Hy2Stream { send, recv }.into_dyn();
    ctx.connect_tcp(inbound, outbound).await?;
    Ok(())
}

async fn read_quic_varint<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> Result<u64> {
    let mut first = [0u8; 1];
    r.read_exact(&mut first).await.map_err(map_other)?;
    let tag = first[0] >> 6;
    let len = match tag {
        0b00 => 1,
        0b01 => 2,
        0b10 => 4,
        0b11 => 8,
        _ => 1,
    };
    let mut buf = [0u8; 8];
    buf[0] = first[0];
    if len > 1 {
        r.read_exact(&mut buf[1..len]).await.map_err(map_other)?;
    }
    let (v, _) = decode_varint(&buf[..len]).ok_or_else(|| Error::Other("bad varint".into()))?;
    Ok(v)
}

async fn hy2_udp_loop(conn: quinn::Connection, net: Net) {
    let state = Arc::new(Mutex::new(UdpState::default()));
    loop {
        let datagram = match conn.read_datagram().await {
            Ok(d) => d,
            Err(_) => return,
        };
        if datagram.is_empty() || datagram[0] != 0x3 {
            continue;
        }
        if datagram.len() < 1 + 4 + 2 + 1 + 1 {
            continue;
        }
        let session_id = u32::from_be_bytes(datagram[1..5].try_into().unwrap());
        let packet_id = u16::from_be_bytes(datagram[5..7].try_into().unwrap());
        let frag_id = datagram[7];
        let frag_count = datagram[8];
        let mut rest = &datagram[9..];

        let (addr_len, vi_len) = match decode_varint(rest) {
            Some(v) => v,
            None => continue,
        };
        rest = &rest[vi_len..];
        let addr_len = addr_len as usize;
        if rest.len() < addr_len {
            continue;
        }
        let addr_str = match std::str::from_utf8(&rest[..addr_len]) {
            Ok(s) => s.to_string(),
            Err(_) => continue,
        };
        rest = &rest[addr_len..];
        let payload = Bytes::copy_from_slice(rest);

        let maybe_complete = {
            let mut st = state.lock();
            st.id_to_addr
                .entry(session_id)
                .or_insert_with(|| addr_str.clone());
            if frag_count <= 1 {
                Some(payload)
            } else {
                let key = (session_id, packet_id);
                let entry = st.reassembly.entry(key).or_insert_with(|| Reassembly {
                    frag_count,
                    frags: vec![None; frag_count as usize],
                    received: 0,
                });
                if entry.frag_count != frag_count || frag_id >= frag_count {
                    st.reassembly.remove(&key);
                    None
                } else {
                    let slot = &mut entry.frags[frag_id as usize];
                    if slot.is_none() {
                        *slot = Some(payload);
                        entry.received = entry.received.saturating_add(1);
                    }
                    if entry.received == frag_count {
                        let mut out = Vec::new();
                        for frag in entry.frags.iter() {
                            if let Some(b) = frag {
                                out.extend_from_slice(b);
                            }
                        }
                        st.reassembly.remove(&key);
                        Some(Bytes::from(out))
                    } else {
                        None
                    }
                }
            }
        };

        if let Some(payload) = maybe_complete {
            let conn2 = conn.clone();
            let net2 = net.clone();
            let state2 = state.clone();
            tokio::spawn(async move {
                if let Err(e) = udp_send_to_target(conn2, net2, state2, session_id, payload).await {
                    tracing::debug!("hysteria udp send error: {:?}", e);
                }
            });
        }
    }
}

#[derive(Default)]
struct UdpState {
    sessions: HashMap<u32, Arc<UdpSession>>,
    id_to_addr: HashMap<u32, String>,
    reassembly: HashMap<(u32, u16), Reassembly>,
}

struct Reassembly {
    frag_count: u8,
    frags: Vec<Option<Bytes>>,
    received: u8,
}

struct UdpSession {
    tx: mpsc::Sender<Bytes>,
}

async fn udp_send_to_target(
    conn: quinn::Connection,
    net: Net,
    state: Arc<Mutex<UdpState>>,
    session_id: u32,
    payload: Bytes,
) -> Result<()> {
    let session = {
        let st = state.lock();
        st.sessions.get(&session_id).cloned()
    };
    let session = match session {
        Some(s) => s,
        None => {
            let addr_str = {
                let st = state.lock();
                st.id_to_addr
                    .get(&session_id)
                    .cloned()
                    .ok_or_else(|| Error::Other("missing udp addr".into()))?
            };
            let target: Address = addr_str.as_str().into_address()?;
            let socket = net
                .udp_bind(&mut Context::new(), &"0.0.0.0:0".parse().unwrap())
                .await?;
            let (tx, rx) = mpsc::channel::<Bytes>(1024);
            let created = Arc::new(UdpSession { tx });

            let mut st = state.lock();
            match st.sessions.get(&session_id) {
                Some(existing) => existing.clone(),
                None => {
                    st.sessions.insert(session_id, created.clone());
                    tokio::spawn(udp_session_loop(
                        conn.clone(),
                        session_id,
                        socket,
                        target,
                        rx,
                    ));
                    created
                }
            }
        }
    };

    session
        .tx
        .send(payload)
        .await
        .map_err(|_| Error::Other("udp session closed".into()))?;
    Ok(())
}

async fn udp_session_loop(
    conn: quinn::Connection,
    session_id: u32,
    mut socket: rd_interface::UdpSocket,
    target: Address,
    mut rx: mpsc::Receiver<Bytes>,
) {
    let max = conn.max_datagram_size().unwrap_or(1200);
    let header_len = 1 + 4 + 2 + 1 + 1;
    let per_frag = max.saturating_sub(header_len).max(1);
    let mut next_packet_id: u16 = 0;

    let mut buf = vec![0u8; 64 * 1024];
    loop {
        tokio::select! {
            biased;
            Some(data) = rx.recv() => {
                let _ = socket.send_to(data.as_ref(), &target).await;
            }
            r = async {
                let mut rb = rd_interface::ReadBuf::new(&mut buf);
                let _ = socket.recv_from(&mut rb).await?;
                Ok::<usize, rd_interface::Error>(rb.filled().len())
            } => {
                let len = match r {
                    Ok(n) => n,
                    Err(_) => return,
                };
                let data = &buf[..len];
                let packet_id = next_packet_id;
                next_packet_id = next_packet_id.wrapping_add(1);

                let frag_count = ((data.len() + per_frag - 1) / per_frag).max(1);
                if frag_count > 255 {
                    continue;
                }
                for frag_id in 0..frag_count {
                    let start = frag_id * per_frag;
                    let end = ((frag_id + 1) * per_frag).min(data.len());
                    let payload = &data[start..end];

                    let mut out = Vec::with_capacity(header_len + payload.len());
                    out.push(0x2);
                    out.extend_from_slice(&session_id.to_be_bytes());
                    out.extend_from_slice(&packet_id.to_be_bytes());
                    out.push(frag_id as u8);
                    out.push(frag_count as u8);
                    out.extend_from_slice(payload);
                    if conn.send_datagram(Bytes::from(out)).is_err() {
                        return;
                    }
                }
            }
        }
    }
}

fn load_cert_and_key(
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
