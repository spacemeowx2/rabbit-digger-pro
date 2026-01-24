use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Sink, SinkExt, Stream};
use pin_project_lite::pin_project;
use tokio::sync::mpsc::{channel, Receiver};
use tokio_util::sync::{PollSendError, PollSender};

pin_project! {
    #[derive(Debug)]
    pub struct Channel<T> {
        buf: Option<T>,
        sender: PollSender<T>,
        receiver: Receiver<T>,
    }
}

impl<T: Send + 'static> Channel<T> {
    // return a pair of channel
    pub fn new() -> (Channel<T>, Channel<T>) {
        let (sender1, receiver1) = channel(10);
        let (sender2, receiver2) = channel(10);
        let sender1 = PollSender::new(sender1);
        let sender2 = PollSender::new(sender2);
        (
            Channel {
                buf: None,
                sender: sender1,
                receiver: receiver2,
            },
            Channel {
                buf: None,
                sender: sender2,
                receiver: receiver1,
            },
        )
    }
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }
}

impl<T> Stream for Channel<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        this.receiver.poll_recv(cx)
    }
}

impl<T: Send + 'static> Sink<T> for Channel<T> {
    type Error = PollSendError<T>;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.sender.poll_ready_unpin(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        let this = self.project();
        this.sender.start_send_unpin(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.sender.poll_flush_unpin(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.sender.poll_close_unpin(cx)
    }
}
