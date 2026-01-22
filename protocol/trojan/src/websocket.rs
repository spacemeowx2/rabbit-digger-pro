use std::{
    collections::VecDeque,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use crate::stream::IOStream;
use futures::{ready, SinkExt, StreamExt};
use rd_interface::{error::map_other, AsyncRead, AsyncWrite, ReadBuf, Result};
use tokio_tungstenite::{client_async, tungstenite::Message};

pub struct WebSocketStream<S> {
    client: tokio_tungstenite::WebSocketStream<S>,
    read: VecDeque<u8>,
    wrote: usize,
}

impl<S: IOStream> WebSocketStream<S> {
    pub async fn connect(stream: S, host: &str, path: &str) -> Result<Self> {
        let url = format!("wss://{}/{}", host, path.trim_start_matches('/'));
        let (client, _resp) = client_async(url, stream).await.map_err(map_other)?;

        Ok(WebSocketStream {
            client,
            read: VecDeque::new(),
            wrote: 0,
        })
    }
}

impl<S: IOStream> AsyncRead for WebSocketStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.read.is_empty() {
            let msg = loop {
                let msg = match ready!(self.client.poll_next_unpin(cx)) {
                    Some(msg) => msg.map_err(map_other)?,
                    None => return Poll::Ready(Ok(())),
                };

                match msg {
                    Message::Binary(b) => break b,
                    Message::Close(_) => return Poll::Ready(Ok(())),
                    _ => {}
                }
            };
            self.read.extend(msg);
        }

        let (first, _) = self.read.as_slices();

        let to_read = first.len().min(buf.remaining());
        buf.initialize_unfilled_to(to_read)
            .copy_from_slice(&first[..to_read]);

        buf.advance(to_read);
        self.read.drain(..to_read);

        Poll::Ready(Ok(()))
    }
}

fn io_err(e: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e)
}

impl<S: IOStream> AsyncWrite for WebSocketStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        if self.wrote == 0 {
            ready!(self.client.poll_ready_unpin(cx).map_err(map_other)?);

            self.client
                .start_send_unpin(Message::binary(buf.to_vec()))
                .map_err(map_other)?;
            self.wrote = buf.len();
        }

        ready!(self.client.poll_flush_unpin(cx).map_err(map_other))?;
        let result = Ok(self.wrote);
        self.wrote = 0;

        Poll::Ready(result)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        self.client.poll_ready_unpin(cx).map_err(io_err)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        self.client.poll_close_unpin(cx).map_err(io_err)
    }
}
