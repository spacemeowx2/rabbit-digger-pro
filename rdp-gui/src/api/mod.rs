//! JSON-RPC transport. The UI does not care which target it runs on: both
//! implementations hand back the same pair, a socket handle for outgoing text
//! and a channel of raw incoming text.

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};

#[cfg(not(target_family = "wasm"))]
mod native;
#[cfg(not(target_family = "wasm"))]
use native as imp;

#[cfg(target_family = "wasm")]
mod web;
#[cfg(target_family = "wasm")]
use web as imp;

/// What the socket reports to the UI.
pub enum BackendEvent {
    Connected,
    Closed(String),
    Text(String),
}

/// Handle for writing to the socket.
pub struct Connection {
    outbox: UnboundedSender<String>,
}

impl Connection {
    pub fn send(&self, text: String) {
        let _ = self.outbox.unbounded_send(text);
    }
}

/// Opens a socket and returns the write handle plus the incoming event stream.
pub fn connect(
    url: String,
    token: Option<String>,
) -> (Connection, UnboundedReceiver<BackendEvent>) {
    imp::connect(url, token)
}

/// Default daemon endpoint.
pub const DEFAULT_ENDPOINT: &str = "127.0.0.1:9091";

pub fn ws_url(endpoint: &str) -> String {
    format!("ws://{endpoint}/api/rpc")
}
