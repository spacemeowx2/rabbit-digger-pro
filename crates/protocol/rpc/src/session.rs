use core::fmt;
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Weak,
    },
};

use futures::TryFutureExt;
use rd_interface::{Address, Arc, Context, Net, Result, TcpListener, TcpStream, UdpSocket};
use state::ClientSessionState;
use tokio::sync::oneshot;

mod state;

use crate::{
    connection::{ClientConnection, Codec, ServerConnection},
    types::{Command, Object, Request, Response, RpcValue},
};

use self::state::{ServerSessionState, Shared};

#[derive(Clone)]
pub struct ClientSession {
    conn: Arc<ClientConnection>,
    state: Arc<ClientSessionState>,
    closed: Arc<AtomicBool>,
}

impl ClientSession {
    pub async fn new(net: &Net, endpoint: &Address, codec: Codec) -> Result<Self> {
        let tcp = net.tcp_connect(&mut Context::new(), endpoint).await?;

        let t = Self {
            conn: Arc::new(ClientConnection::new(tcp, codec)),
            state: Arc::new(ClientSessionState::new()),
            closed: Arc::new(AtomicBool::new(false)),
        };

        t.send(Command::Handshake(t.state.session_id()), None)
            .await?
            .wait()
            .await?
            .0
            .into_null()?;

        Ok(t)
    }

    async fn wait_response(&self) -> Result<()> {
        let (resp, data) = self.conn.next().await?;

        self.state.send_response(resp, data);

        Ok(())
    }

    pub async fn send(&self, cmd: Command, data: Option<Vec<u8>>) -> Result<ResponseGetter> {
        let seq_id = self.state.next_seq_id();
        match self.conn.send(Request { cmd, seq_id }, data).await {
            Ok(_) => {}
            Err(e) => {
                self.closed.store(true, Ordering::Relaxed);
                return Err(e.into());
            }
        };

        let rx = self.state.wait_for_response(seq_id);

        let conn = self.clone();
        let closed = self.closed.clone();
        let state = self.state.clone();
        tokio::spawn(async move {
            if let Err(e) = conn.wait_response().await {
                closed.store(true, Ordering::Relaxed);
                state.send_response(
                    Response {
                        seq_id,
                        result: Err(e.to_string()),
                    },
                    Vec::new(),
                )
            }
        });

        Ok(ResponseGetter {
            state: Arc::downgrade(&self.state),
            seq_id,
            rx: Some(rx),
        })
    }

    pub fn close_object(&self, obj: Object) {
        let this = self.clone();
        let fut = async move {
            this.send(Command::Close(obj), None).await?.wait().await?;

            Result::<(), rd_interface::Error>::Ok(())
        }
        .inspect_err(|e| tracing::error!("Failed to close object: {:?}", e));
        tokio::spawn(fut);
    }
    #[allow(dead_code)]
    pub async fn close(&self) -> io::Result<()> {
        self.conn.close().await
    }
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

pub struct ResponseGetter {
    state: Weak<ClientSessionState>,
    seq_id: u32,
    rx: Option<oneshot::Receiver<(Response, Vec<u8>)>>,
}

impl Drop for ResponseGetter {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            state.send_response(
                Response {
                    seq_id: self.seq_id,
                    result: Err("Aborted".to_string()),
                },
                Vec::new(),
            );
        }
    }
}

impl ResponseGetter {
    pub async fn wait(mut self) -> Result<(Response, Vec<u8>)> {
        self.rx
            .take()
            .unwrap()
            .await
            .map_err(|_| rd_interface::Error::other("channel closed"))
    }
}

pub enum Obj {
    TcpStream(TcpStream),
    TcpListener(TcpListener),
    UdpSocket(UdpSocket),
}

impl Obj {
    pub fn tcp_listener(&self) -> Result<&TcpListener> {
        match self {
            Obj::TcpListener(tcp) => Ok(tcp),
            _ => Err(rd_interface::Error::other("not a tcp listener")),
        }
    }
    pub fn tcp_stream_mut(&mut self) -> Result<&mut TcpStream> {
        match self {
            Obj::TcpStream(tcp) => Ok(tcp),
            _ => Err(rd_interface::Error::other("not a tcp stream")),
        }
    }
    pub fn udp_socket_mut(&mut self) -> Result<&mut UdpSocket> {
        match self {
            Obj::UdpSocket(udp) => Ok(udp),
            _ => Err(rd_interface::Error::other("not a udp socket")),
        }
    }
}

#[derive(Clone)]
pub struct ServerSession {
    conn: Arc<ServerConnection>,
    state: Arc<ServerSessionState<Obj>>,
}

impl ServerSession {
    pub fn new(tcp: TcpStream, codec: Codec) -> Self {
        Self {
            conn: Arc::new(ServerConnection::new(tcp, codec)),
            state: Arc::new(ServerSessionState::new()),
        }
    }
    pub async fn recv(&self) -> Result<RequestGetter> {
        let (req, data) = self.conn.next().await?;

        Ok(RequestGetter {
            req,
            data,
            conn: self.conn.clone(),
            state: self.state.clone(),
            sent: false,
        })
    }
    pub async fn close(&self) -> io::Result<()> {
        self.conn.close().await
    }
}

#[must_use]
pub struct RequestGetter {
    req: Request,
    data: Vec<u8>,
    conn: Arc<ServerConnection>,
    state: Arc<ServerSessionState<Obj>>,
    sent: bool,
}

impl fmt::Debug for RequestGetter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestGetter")
            .field("req", &self.req)
            .field("data", &self.data)
            .field("sent", &self.sent)
            .finish()
    }
}

impl RequestGetter {
    pub fn cmd(&self) -> &Command {
        &self.req.cmd
    }
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    pub fn insert_object(&self, obj: Obj) -> Object {
        self.state.insert_object(obj)
    }
    pub fn remove_object(&self, obj: Object) {
        self.state.remove_object(obj)
    }
    pub fn get_object(&self, obj: Object) -> Result<Shared<Obj>> {
        self.state.get_object(obj)
    }
    pub async fn response(
        mut self,
        result: Result<RpcValue, String>,
        data: Option<Vec<u8>>,
    ) -> Result<()> {
        self.conn
            .send(
                Response {
                    seq_id: self.req.seq_id,
                    result,
                },
                data,
            )
            .await?;

        self.sent = true;

        Ok(())
    }
}

impl Drop for RequestGetter {
    fn drop(&mut self) {
        if !self.sent {
            tracing::error!("RequestGetter dropped without sending response");
        }
    }
}
