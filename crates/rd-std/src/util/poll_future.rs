use std::{
    future::Future,
    mem::replace,
    pin::Pin,
    task::{Context, Poll},
};

pub enum PollFuture<T> {
    Empty,
    Ready(T),
    Future(Pin<Box<dyn Future<Output = T> + Send>>),
}

impl<T> PollFuture<T> {
    pub fn new(fut: impl Future<Output = T> + Send + 'static) -> Self {
        PollFuture::Future(Box::pin(fut))
    }
    pub fn ready(val: T) -> Self {
        PollFuture::Ready(val)
    }
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<T> {
        match self {
            PollFuture::Empty => panic!("PollFuture::poll called on empty future"),
            PollFuture::Ready(_) => {
                let val = replace(self, PollFuture::Empty);
                Poll::Ready(match val {
                    PollFuture::Ready(val) => val,
                    _ => unreachable!(),
                })
            }
            PollFuture::Future(fut) => {
                let val = futures::ready!(fut.as_mut().poll(cx));
                *self = PollFuture::Empty;
                Poll::Ready(val)
            }
        }
    }
    pub fn is_ready(&self) -> bool {
        match self {
            PollFuture::Empty => true,
            PollFuture::Ready(_) => true,
            PollFuture::Future(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::*;

    #[test]
    fn test_poll_future() {
        let (tx, mut rx) = mpsc::unbounded_channel::<()>();
        let mut fut = PollFuture::new(async move {
            rx.recv().await;
            1
        });

        assert!(!fut.is_ready());
        assert_eq!(
            fut.poll(&mut Context::from_waker(futures::task::noop_waker_ref())),
            Poll::Pending
        );
        assert!(!fut.is_ready());
        tx.send(()).unwrap();
        assert_eq!(
            fut.poll(&mut Context::from_waker(futures::task::noop_waker_ref())),
            Poll::Ready(1)
        );
        assert!(fut.is_ready());
    }

    #[test]
    fn test_poll_future_ready() {
        let mut fut = PollFuture::ready(1);
        assert!(fut.is_ready());
        assert_eq!(
            fut.poll(&mut Context::from_waker(futures::task::noop_waker_ref())),
            Poll::Ready(1)
        );
        assert!(fut.is_ready());
    }
}
