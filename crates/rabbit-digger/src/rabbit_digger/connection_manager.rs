use std::{
    collections::HashMap,
    io,
    sync::{atomic::Ordering, Arc},
    task::{Context, Poll},
    time::{Duration, SystemTime},
};

use super::event::{Event, EventType};
use atomic_shim::AtomicU64;
use dashmap::DashMap;
use futures::{FutureExt, StreamExt};
use parking_lot::Mutex;
use rd_interface::{Address, Value};
use serde::{Serialize, Serializer};
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    task::JoinHandle,
    time::interval,
};
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);

fn ts(time: &SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn serialize_atomicu64<S>(a: &AtomicU64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_u64(a.load(Ordering::Relaxed))
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
}

#[derive(Debug, Serialize)]
pub struct ConnectionInfo {
    protocol: Protocol,
    addr: Address,
    start_time: u64,
    ctx: Value,
    #[serde(serialize_with = "serialize_atomicu64")]
    upload: AtomicU64,
    #[serde(serialize_with = "serialize_atomicu64")]
    download: AtomicU64,
    #[serde(skip)]
    stop_sender: Mutex<Option<oneshot::Sender<()>>>,
}

#[derive(Debug, Serialize)]
pub struct ConnectionState {
    connections: DashMap<Uuid, ConnectionInfo>,
    #[serde(serialize_with = "serialize_atomicu64")]
    total_upload: AtomicU64,
    #[serde(serialize_with = "serialize_atomicu64")]
    total_download: AtomicU64,
}

impl ConnectionState {
    fn new() -> Self {
        ConnectionState {
            connections: DashMap::new(),
            total_upload: AtomicU64::new(0),
            total_download: AtomicU64::new(0),
        }
    }
    fn input_event(&self, event: Event) {
        let Event { uuid, events, time } = event;

        for event in events {
            match event {
                EventType::NewTcp(addr, ctx) => {
                    self.connections.insert(
                        uuid,
                        ConnectionInfo {
                            protocol: Protocol::Tcp,
                            addr: addr.clone(),
                            ctx: ctx.clone(),
                            start_time: ts(&time),
                            upload: AtomicU64::new(0),
                            download: AtomicU64::new(0),
                            stop_sender: Mutex::new(None),
                        },
                    );
                }
                EventType::NewUdp(addr, ctx) => {
                    self.connections.insert(
                        uuid,
                        ConnectionInfo {
                            protocol: Protocol::Udp,
                            addr: addr.clone(),
                            ctx: ctx.clone(),
                            start_time: ts(&time),
                            upload: AtomicU64::new(0),
                            download: AtomicU64::new(0),
                            stop_sender: Mutex::new(None),
                        },
                    );
                }
                EventType::SetStopper(sender) => {
                    if let Some(conn) = self.connections.get(&uuid) {
                        let mut stop_sender = conn.stop_sender.lock();
                        *stop_sender = Some(sender);
                    }
                }
                EventType::Read(download) => {
                    if let Some(conn) = self.connections.get(&uuid) {
                        conn.download.fetch_add(download, Ordering::Relaxed);
                        self.total_download.fetch_add(download, Ordering::Relaxed);
                    }
                }
                EventType::Write(upload) => {
                    if let Some(conn) = self.connections.get(&uuid) {
                        conn.upload.fetch_add(upload, Ordering::Relaxed);
                        self.total_upload.fetch_add(upload, Ordering::Relaxed);
                    }
                }
                EventType::RecvFrom(_, download) => {
                    if let Some(conn) = self.connections.get(&uuid) {
                        conn.download.fetch_add(download, Ordering::Relaxed);
                        self.total_download.fetch_add(download, Ordering::Relaxed);
                    }
                }
                EventType::SendTo(_, upload) => {
                    if let Some(conn) = self.connections.get(&uuid) {
                        conn.upload.fetch_add(upload, Ordering::Relaxed);
                        self.total_upload.fetch_add(upload, Ordering::Relaxed);
                    }
                }
                EventType::CloseConnection => {
                    self.connections.remove(&uuid);
                }
            };
        }
    }
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }
}

struct ManagerInner {
    state: ConnectionState,
    heartbeat_interval: broadcast::Sender<()>,
    sender: mpsc::UnboundedSender<Event>,
    heartbeat_handle: JoinHandle<()>,
}

impl ManagerInner {
    fn new() -> Arc<Self> {
        let (this, rx) = Self::new2();

        tokio::spawn(Self::recv_event(rx, this.clone()));

        this
    }
    fn new2() -> (Arc<Self>, mpsc::UnboundedReceiver<Event>) {
        let (sender, rx) = mpsc::unbounded_channel();
        let (heartbeat_interval, _) = broadcast::channel(1);
        let tx = heartbeat_interval.clone();

        let heartbeat_handle = tokio::spawn(async move {
            let mut interval = interval(HEARTBEAT_INTERVAL);
            loop {
                let _ = tx.send(());
                interval.tick().await;
            }
        });

        (
            Arc::new(Self {
                state: ConnectionState::new(),
                heartbeat_interval,
                sender,
                heartbeat_handle,
            }),
            rx,
        )
    }
    async fn recv_event(mut rx: mpsc::UnboundedReceiver<Event>, inner: Arc<ManagerInner>) {
        while let Some(event) = rx.recv().await {
            inner.state.input_event(event);
        }
        tracing::warn!("recv_event task exited");
    }
}

#[derive(Clone)]
pub struct ConnectionManager {
    inner: Arc<ManagerInner>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            inner: ManagerInner::new(),
        }
    }
    #[cfg(test)]
    pub fn new_for_test() -> (Self, mpsc::UnboundedReceiver<Event>) {
        let (inner, rx) = ManagerInner::new2();

        (Self { inner }, rx)
    }
    pub fn stop(&self) {
        self.inner.heartbeat_handle.abort()
    }
    pub fn borrow_state<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&ConnectionState) -> R,
    {
        let conn = &self.inner.state;
        f(conn)
    }
    pub fn stop_connection(&self, uuid: Uuid) -> bool {
        self.inner
            .state
            .connections
            .get(&uuid)
            .and_then(|conn| conn.stop_sender.lock().take())
            .map(|sender| sender.send(()).is_ok())
            .unwrap_or_default()
    }
    pub fn stop_connections(&self) -> usize {
        let mut stopped = 0;
        for conn in &self.inner.state.connections {
            if let Some(sender) = conn.stop_sender.lock().take() {
                if sender.send(()).is_ok() {
                    stopped += 1;
                }
            }
        }
        stopped
    }
    pub fn new_connection<T: ConnType>(
        &self,
        addr: Address,
        ctx: &rd_interface::Context,
    ) -> Connection<T> {
        Connection::<T>::new(
            addr,
            ctx,
            self.inner.heartbeat_interval.subscribe(),
            self.inner.sender.clone(),
        )
    }
}

pub trait ConnType: Default {
    fn event_type(addr: Address, ctx: Value) -> EventType;
    fn get_events(&mut self) -> Vec<EventType>;
}

#[derive(Default)]
pub struct Tcp {
    read: u64,
    write: u64,
}
impl ConnType for Tcp {
    fn event_type(addr: Address, ctx: Value) -> EventType {
        EventType::NewTcp(addr, ctx)
    }

    fn get_events(&mut self) -> Vec<EventType> {
        let mut ret = Vec::with_capacity(2);
        if self.read > 0 {
            ret.push(EventType::Read(self.read));
            self.read = 0;
        }
        if self.write > 0 {
            ret.push(EventType::Write(self.write));
            self.write = 0;
        }
        ret
    }
}
#[derive(Default)]
pub struct Udp {
    recv_from: HashMap<Address, u64>,
    send_to: HashMap<Address, u64>,
}
impl ConnType for Udp {
    fn event_type(addr: Address, ctx: Value) -> EventType {
        EventType::NewUdp(addr, ctx)
    }

    fn get_events(&mut self) -> Vec<EventType> {
        let mut ret = Vec::with_capacity(self.recv_from.len() + self.send_to.len());
        for (addr, download) in self.recv_from.drain() {
            ret.push(EventType::RecvFrom(addr, download));
        }
        for (addr, upload) in self.send_to.drain() {
            ret.push(EventType::SendTo(addr, upload));
        }
        ret
    }
}

#[derive(Debug)]
pub struct Connection<T: ConnType> {
    state: T,
    uuid: Uuid,

    heartbeat_interval: BroadcastStream<()>,
    sender: mpsc::UnboundedSender<Event>,
    stopped: oneshot::Receiver<()>,
}

impl<T> Connection<T>
where
    T: ConnType,
{
    fn new(
        addr: Address,
        ctx: &rd_interface::Context,
        heartbeat_interval: broadcast::Receiver<()>,
        sender: mpsc::UnboundedSender<Event>,
    ) -> Self {
        let (stopper, stopped) = oneshot::channel();
        let uuid = Uuid::new_v4();
        let this = Connection {
            state: T::default(),
            uuid,
            heartbeat_interval: BroadcastStream::new(heartbeat_interval),
            sender,
            stopped,
        };
        this.send(vec![
            T::event_type(addr, ctx.to_value()),
            EventType::SetStopper(stopper),
        ]);
        this
    }
    pub fn poll(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        if let Poll::Ready(_) = self.heartbeat_interval.poll_next_unpin(cx) {
            let events = self.state.get_events();
            self.send(events);
        }
        if let Poll::Ready(r) = self.stopped.poll_unpin(cx) {
            eprintln!("err {:?}", r);
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Aborted by user",
            ));
        }
        Ok(())
    }
    #[cfg(test)]
    pub async fn poll_async(&mut self) -> io::Result<()> {
        futures::future::poll_fn(|cx| {
            self.poll(cx)?;
            Poll::Ready(Ok(()))
        })
        .await
    }
    fn send(&self, events: Vec<EventType>) {
        if !events.is_empty() && self.sender.send(Event::new(self.uuid, events)).is_err() {
            tracing::warn!("Failed to send event");
        }
    }
}

impl Connection<Tcp> {
    pub fn read(&mut self, size: u64) {
        self.state.read += size;
    }
    pub fn write(&mut self, size: u64) {
        self.state.write += size;
    }
}

impl Connection<Udp> {
    pub fn recv_from(&mut self, addr: Address, size: u64) {
        self.state
            .recv_from
            .entry(addr)
            .and_modify(|e| *e += size)
            .or_insert(size);
    }
    pub fn send_to(&mut self, addr: Address, size: u64) {
        self.state
            .send_to
            .entry(addr)
            .and_modify(|e| *e += size)
            .or_insert(size);
    }
}

impl<T: ConnType> Drop for Connection<T> {
    fn drop(&mut self) {
        let events = self.state.get_events();
        self.send(events);
        self.send(vec![EventType::CloseConnection])
    }
}

#[cfg(test)]
mod tests {
    use rd_interface::IntoAddress;
    use tokio::{task::yield_now, time::sleep};

    use super::*;

    struct WantedConn {
        protocol: Protocol,
        addr: Address,
        upload: u64,
        download: u64,
    }

    fn assert_conn(conn_mgr: &ConnectionManager, wanted: WantedConn) {
        conn_mgr.borrow_state(|s| {
            let entry = s.connections.iter().next().unwrap();
            let conn = entry.value();

            assert_eq!(conn.protocol, wanted.protocol);
            assert_eq!(conn.addr, wanted.addr);
            assert_eq!(conn.upload.load(Ordering::Relaxed), wanted.upload);
            assert_eq!(conn.download.load(Ordering::Relaxed), wanted.download);
        });
    }

    #[tokio::test]
    async fn test_connection_manager_tcp() {
        let conn_mgr = ConnectionManager::new();
        let addr = "localhost:1234".into_address().unwrap();

        let mut tcp = conn_mgr.new_connection::<Tcp>(addr.clone(), &rd_interface::Context::new());
        yield_now().await;
        assert_conn(
            &conn_mgr,
            WantedConn {
                protocol: Protocol::Tcp,
                addr: addr.clone(),
                upload: 0,
                download: 0,
            },
        );
        tcp.read(1);
        tcp.write(1);
        tcp.read(1);
        tcp.poll_async().await.unwrap();
        assert_conn(
            &conn_mgr,
            WantedConn {
                protocol: Protocol::Tcp,
                addr: addr.clone(),
                upload: 0,
                download: 0,
            },
        );
        sleep(Duration::from_secs(1)).await;
        tcp.poll_async().await.unwrap();

        assert_conn(
            &conn_mgr,
            WantedConn {
                protocol: Protocol::Tcp,
                addr,
                upload: 1,
                download: 2,
            },
        );

        drop(tcp);
        yield_now().await;

        assert!(conn_mgr.inner.state.connections.is_empty());
    }

    #[tokio::test]
    async fn test_connection_manager_udp() {
        let conn_mgr = ConnectionManager::new();
        let addr = "localhost:1234".into_address().unwrap();

        let mut udp = conn_mgr.new_connection::<Udp>(addr.clone(), &rd_interface::Context::new());
        yield_now().await;
        assert_conn(
            &conn_mgr,
            WantedConn {
                protocol: Protocol::Udp,
                addr: addr.clone(),
                upload: 0,
                download: 0,
            },
        );
        udp.recv_from(addr.clone(), 1);
        udp.send_to(addr.clone(), 1);
        udp.recv_from(addr.clone(), 1);
        udp.poll_async().await.unwrap();
        assert_conn(
            &conn_mgr,
            WantedConn {
                protocol: Protocol::Udp,
                addr: addr.clone(),
                upload: 0,
                download: 0,
            },
        );
        sleep(Duration::from_secs(1)).await;
        udp.poll_async().await.unwrap();

        assert_conn(
            &conn_mgr,
            WantedConn {
                protocol: Protocol::Udp,
                addr,
                upload: 1,
                download: 2,
            },
        );

        drop(udp);
        yield_now().await;

        assert_eq!(conn_mgr.inner.state.connections.len(), 0);
    }
}
