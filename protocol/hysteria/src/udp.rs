use std::{
    collections::HashMap,
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use bytes::Bytes;
use futures_util::Stream;
use parking_lot::Mutex;
use quinn::Connection;
use rd_interface::{
    async_trait, Address, Error, IUdpSocket, IntoDyn, Net, ReadBuf, Result, UdpSocket,
};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

use crate::codec::{varint_len, write_varint};

pub(crate) struct Hy2Udp {
    conn: Connection,
    net: Net,
    state: Arc<Mutex<Hy2UdpState>>,
    incoming: broadcast::Sender<IncomingPacket>,
    _recv_task: tokio::task::JoinHandle<()>,
}

#[derive(Clone, Debug)]
struct IncomingPacket {
    peer: SocketAddr,
    payload: Bytes,
}

struct Hy2UdpState {
    next_session_id: u32,
    sessions: HashMap<Address, SessionEntry>,
    id_to_peer: HashMap<u32, SocketAddr>,
    reassembly: HashMap<(u32, u16), Reassembly>,
    last_prune: Instant,
}

impl Default for Hy2UdpState {
    fn default() -> Self {
        Self {
            next_session_id: 0,
            sessions: HashMap::new(),
            id_to_peer: HashMap::new(),
            reassembly: HashMap::new(),
            last_prune: Instant::now(),
        }
    }
}

struct Reassembly {
    frag_count: u8,
    frags: Vec<Option<Bytes>>,
    received: u8,
    created_at: Instant,
}

enum SessionEntry {
    Ready(Session),
    Resolving(ResolvingSession),
}

struct ResolvingSession {
    address_str: String,
    join: tokio::task::JoinHandle<Result<SocketAddr>>,
}

#[derive(Clone)]
struct Session {
    session_id: u32,
    address_str: String,
    next_packet_id: u16,
}

impl Hy2Udp {
    pub(crate) fn new(conn: Connection, net: Net) -> Self {
        let (incoming, _) = broadcast::channel(1024);
        let state = Arc::new(Mutex::new(Hy2UdpState {
            next_session_id: rand::random(),
            last_prune: Instant::now(),
            ..Default::default()
        }));

        let recv_task = {
            let conn = conn.clone();
            let incoming = incoming.clone();
            let state = state.clone();
            tokio::spawn(async move {
                recv_loop(conn, state, incoming).await;
            })
        };

        Self {
            conn,
            net,
            state,
            incoming,
            _recv_task: recv_task,
        }
    }

    pub(crate) fn bind_socket(&self) -> UdpSocket {
        Hy2UdpSocket {
            conn: self.conn.clone(),
            net: self.net.clone(),
            state: self.state.clone(),
            max_datagram_size: self.conn.max_datagram_size(),
            incoming: Mutex::new(BroadcastStream::new(self.incoming.subscribe())),
        }
        .into_dyn()
    }
}

async fn recv_loop(
    conn: Connection,
    state: Arc<Mutex<Hy2UdpState>>,
    incoming: broadcast::Sender<IncomingPacket>,
) {
    loop {
        let datagram = match conn.read_datagram().await {
            Ok(d) => d,
            Err(_) => return,
        };
        if datagram.len() < 4 + 2 + 1 + 1 {
            continue;
        }
        let session_id = u32::from_be_bytes(datagram[0..4].try_into().unwrap());
        let packet_id = u16::from_be_bytes(datagram[4..6].try_into().unwrap());
        let frag_id = datagram[6];
        let frag_count = datagram[7];
        let mut rest = &datagram[8..];

        let (addr_len, vi_len) = match crate::codec::decode_varint(rest) {
            Some(v) => v,
            None => continue,
        };
        rest = &rest[vi_len..];
        let addr_len = addr_len as usize;
        if rest.len() < addr_len {
            continue;
        }
        rest = &rest[addr_len..];
        let payload = Bytes::copy_from_slice(rest);

        let (peer, maybe_complete) = {
            let mut st = state.lock();
            prune_reassembly(&mut st);
            let peer = match st.id_to_peer.get(&session_id).copied() {
                Some(p) => p,
                None => continue,
            };

            if frag_count <= 1 {
                (peer, Some(payload))
            } else {
                let key = (session_id, packet_id);
                let now = Instant::now();
                let entry = st.reassembly.entry(key).or_insert_with(|| Reassembly {
                    frag_count,
                    frags: vec![None; frag_count as usize],
                    received: 0,
                    created_at: now,
                });

                if entry.frag_count != frag_count {
                    st.reassembly.remove(&key);
                    continue;
                }
                if now.duration_since(entry.created_at) > REASSEMBLY_TTL {
                    st.reassembly.remove(&key);
                    continue;
                }
                if frag_id >= frag_count {
                    continue;
                }
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
                    (peer, Some(Bytes::from(out)))
                } else {
                    (peer, None)
                }
            }
        };

        if let Some(payload) = maybe_complete {
            let _ = incoming.send(IncomingPacket { peer, payload });
        }
    }
}

const REASSEMBLY_TTL: Duration = Duration::from_secs(10);
const REASSEMBLY_PRUNE_INTERVAL: Duration = Duration::from_secs(1);
const REASSEMBLY_MAX_ENTRIES: usize = 2048;

fn prune_reassembly(st: &mut Hy2UdpState) {
    let now = Instant::now();
    if now.duration_since(st.last_prune) < REASSEMBLY_PRUNE_INTERVAL
        && st.reassembly.len() < REASSEMBLY_MAX_ENTRIES
    {
        return;
    }
    st.reassembly
        .retain(|_, e| now.duration_since(e.created_at) <= REASSEMBLY_TTL);
    if st.reassembly.len() > REASSEMBLY_MAX_ENTRIES {
        st.reassembly.clear();
    }
    st.last_prune = now;
}

struct Hy2UdpSocket {
    conn: Connection,
    net: Net,
    state: Arc<Mutex<Hy2UdpState>>,
    max_datagram_size: Option<usize>,
    incoming: Mutex<BroadcastStream<IncomingPacket>>,
}

#[async_trait]
impl IUdpSocket for Hy2UdpSocket {
    async fn local_addr(&self) -> Result<SocketAddr> {
        Err(rd_interface::NOT_IMPLEMENTED)
    }

    fn poll_recv_from(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<SocketAddr>> {
        loop {
            let mut incoming = self.incoming.lock();
            match Pin::new(&mut *incoming).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "udp closed",
                    )))
                }
                Poll::Ready(Some(Ok(pkt))) => {
                    let to_copy = pkt.payload.len().min(buf.remaining());
                    buf.put_slice(&pkt.payload[..to_copy]);
                    return Poll::Ready(Ok(pkt.peer));
                }
                Poll::Ready(Some(Err(
                    tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_),
                ))) => {
                    continue;
                }
            }
        }
    }

    fn poll_send_to(
        &mut self,
        cx: &mut Context<'_>,
        buf: &[u8],
        target: &Address,
    ) -> Poll<io::Result<usize>> {
        let max = match self.max_datagram_size {
            Some(m) => m,
            None => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "datagram not supported",
                )))
            }
        };

        let session = match self.poll_get_or_create_session(cx, target) {
            Poll::Ready(Ok(s)) => s,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Pending => return Poll::Pending,
        };

        let addr_bytes = session.address_str.as_bytes();
        let addr_len_vi = varint_len(addr_bytes.len() as u64);
        let header_len = 4 + 2 + 1 + 1 + addr_len_vi + addr_bytes.len();
        if header_len >= max {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "datagram size too small",
            )));
        }
        let per_frag = max - header_len;
        let frag_count = ((buf.len() + per_frag - 1) / per_frag).max(1);
        if frag_count > 255 {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "udp payload too large",
            )));
        }

        let (session_id, packet_id) = {
            let mut st = self.state.lock();
            let entry = st
                .sessions
                .get_mut(target)
                .and_then(|e| match e {
                    SessionEntry::Ready(s) => Some(s),
                    _ => None,
                })
                .expect("session ready");
            let pid = entry.next_packet_id;
            entry.next_packet_id = entry.next_packet_id.wrapping_add(1);
            (entry.session_id, pid)
        };

        for frag_id in 0..frag_count {
            let start = frag_id * per_frag;
            let end = ((frag_id + 1) * per_frag).min(buf.len());
            let payload = &buf[start..end];

            let mut out = Vec::with_capacity(header_len + payload.len());
            out.extend_from_slice(&session_id.to_be_bytes());
            out.extend_from_slice(&packet_id.to_be_bytes());
            out.push(frag_id as u8);
            out.push(frag_count as u8);
            write_varint(&mut out, addr_bytes.len() as u64);
            out.extend_from_slice(addr_bytes);
            out.extend_from_slice(payload);

            if let Err(e) = self.conn.send_datagram(Bytes::from(out)) {
                return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)));
            }
        }

        Poll::Ready(Ok(buf.len()))
    }
}

impl Hy2UdpSocket {
    fn poll_get_or_create_session(
        &mut self,
        cx: &mut Context<'_>,
        target: &Address,
    ) -> Poll<io::Result<Session>> {
        let mut st = self.state.lock();

        if let Some(mut entry) = st.sessions.remove(target) {
            match &mut entry {
                SessionEntry::Ready(s) => {
                    let s = s.clone();
                    st.sessions.insert(target.clone(), entry);
                    return Poll::Ready(Ok(s));
                }
                SessionEntry::Resolving(r) => match Pin::new(&mut r.join).poll(cx) {
                    Poll::Pending => {
                        st.sessions.insert(target.clone(), entry);
                        return Poll::Pending;
                    }
                    Poll::Ready(Ok(Ok(peer))) => {
                        let session_id = st.next_session_id;
                        st.next_session_id = st.next_session_id.wrapping_add(1);
                        let session = Session {
                            session_id,
                            address_str: r.address_str.clone(),
                            next_packet_id: 0,
                        };
                        st.id_to_peer.insert(session_id, peer);
                        st.sessions
                            .insert(target.clone(), SessionEntry::Ready(session.clone()));
                        return Poll::Ready(Ok(session));
                    }
                    Poll::Ready(Ok(Err(e))) => {
                        return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
                    }
                    Poll::Ready(Err(e)) => {
                        return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
                    }
                },
            }
        }

        let (address_str, resolve) = match target {
            Address::SocketAddr(sa) => (sa.to_string(), None),
            Address::Domain(domain, port) => {
                (format!("{domain}:{port}"), Some((domain.clone(), *port)))
            }
        };

        if let Some((domain, port)) = resolve {
            let net = self.net.clone();
            let join = tokio::spawn(async move {
                net.lookup_host(&Address::Domain(domain, port))
                    .await?
                    .into_iter()
                    .next()
                    .ok_or_else(|| Error::Other("failed to resolve udp target".into()))
            });
            st.sessions.insert(
                target.clone(),
                SessionEntry::Resolving(ResolvingSession { address_str, join }),
            );
            return Poll::Pending;
        }

        let peer = match target {
            Address::SocketAddr(sa) => *sa,
            Address::Domain(_, _) => unreachable!(),
        };
        let session_id = st.next_session_id;
        st.next_session_id = st.next_session_id.wrapping_add(1);
        st.id_to_peer.insert(session_id, peer);
        let session = Session {
            session_id,
            address_str,
            next_packet_id: 0,
        };
        st.sessions
            .insert(target.clone(), SessionEntry::Ready(session.clone()));

        Poll::Ready(Ok(session))
    }
}
