//! Browser transport: `web_sys::WebSocket` plus a local task that drains the
//! outbox. Only compiled for `wasm32`.

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use futures::StreamExt;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, WebSocket};

use super::{BackendEvent, Connection};

pub fn connect(
    mut url: String,
    token: Option<String>,
) -> (Connection, UnboundedReceiver<BackendEvent>) {
    if let Some(token) = token {
        url.push_str(if url.contains('?') {
            "&token="
        } else {
            "?token="
        });
        url.push_str(&token);
    }
    let (event_tx, event_rx) = unbounded();
    let (outbox, mut outbox_rx) = unbounded::<String>();

    let socket = match WebSocket::new(&url) {
        Ok(socket) => socket,
        Err(error) => {
            let _ = event_tx
                .unbounded_send(BackendEvent::Closed(format!("WebSocket failed: {error:?}")));
            return (Connection { outbox }, event_rx);
        }
    };
    socket.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let on_open = {
        let event_tx = event_tx.clone();
        Closure::<dyn FnMut()>::new(move || {
            let _ = event_tx.unbounded_send(BackendEvent::Connected);
        })
    };
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let on_message = {
        let event_tx = event_tx.clone();
        Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            if let Some(text) = event.data().as_string() {
                let _ = event_tx.unbounded_send(BackendEvent::Text(text));
            }
        })
    };
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    let on_close = {
        let event_tx = event_tx.clone();
        Closure::<dyn FnMut()>::new(move || {
            let _ = event_tx.unbounded_send(BackendEvent::Closed("socket closed".to_string()));
        })
    };
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();

    let on_error = {
        let event_tx = event_tx.clone();
        Closure::<dyn FnMut()>::new(move || {
            let _ = event_tx.unbounded_send(BackendEvent::Closed("socket error".to_string()));
        })
    };
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();

    {
        let socket = socket.clone();
        wasm_bindgen_futures::spawn_local(async move {
            while let Some(text) = outbox_rx.next().await {
                if socket.send_with_str(&text).is_err() {
                    break;
                }
            }
        });
    }

    (Connection { outbox }, event_rx)
}
