//! Native transport: a dedicated thread running a small Tokio runtime, so the
//! GPUI main loop never blocks on the socket.

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use super::{BackendEvent, Connection};

pub fn connect(url: String) -> (Connection, UnboundedReceiver<BackendEvent>) {
    let (event_tx, event_rx) = unbounded();
    let (outbox, outbox_rx) = unbounded::<String>();

    std::thread::Builder::new()
        .name("rdp-gui-rpc".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = event_tx.unbounded_send(BackendEvent::Closed(format!(
                        "tokio runtime failed: {error}"
                    )));
                    return;
                }
            };
            runtime.block_on(pump(url, event_tx, outbox_rx));
        })
        .expect("failed to spawn the rpc thread");

    (Connection { outbox }, event_rx)
}

async fn pump(
    url: String,
    event_tx: futures::channel::mpsc::UnboundedSender<BackendEvent>,
    mut outbox_rx: futures::channel::mpsc::UnboundedReceiver<String>,
) {
    let (socket, _response) = match tokio_tungstenite::connect_async(&url).await {
        Ok(pair) => pair,
        Err(error) => {
            let _ = event_tx.unbounded_send(BackendEvent::Closed(error.to_string()));
            return;
        }
    };

    let _ = event_tx.unbounded_send(BackendEvent::Connected);
    let (mut sink, mut stream) = socket.split();

    let upstream = async {
        while let Some(text) = outbox_rx.next().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    };

    let downstream = async {
        while let Some(message) = stream.next().await {
            match message {
                Ok(Message::Text(text)) => {
                    if event_tx
                        .unbounded_send(BackendEvent::Text(text.to_string()))
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Close(frame)) => {
                    let _ = event_tx.unbounded_send(BackendEvent::Closed(format!(
                        "closed by server: {frame:?}"
                    )));
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = event_tx.unbounded_send(BackendEvent::Closed(error.to_string()));
                    break;
                }
            }
        }
    };

    tokio::select! {
        _ = upstream => {}
        _ = downstream => {}
    }
}
