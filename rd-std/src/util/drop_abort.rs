use std::{
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll},
};

use futures::Future;
use tokio::task::{JoinError, JoinHandle};

pub struct DropAbort<T>(JoinHandle<T>);

impl<T> Drop for DropAbort<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<T> Deref for DropAbort<T> {
    type Target = JoinHandle<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for DropAbort<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> DropAbort<T> {
    pub fn new(handle: JoinHandle<T>) -> Self {
        DropAbort(handle)
    }
}

impl<T> Future for DropAbort<T> {
    type Output = Result<T, JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_drop_abort() {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = flag.clone();

        let handle = tokio::spawn(async move {
            loop {
                if flag_clone.load(Ordering::Relaxed) {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
            "completed"
        });

        {
            let drop_abort = DropAbort::new(handle);
            sleep(Duration::from_millis(50)).await;
            drop(drop_abort);
        }

        sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_drop_abort_deref() {
        let handle = tokio::spawn(async { "test" });
        let drop_abort = DropAbort::new(handle);
        let _ = drop_abort.0;
    }

    #[tokio::test]
    async fn test_drop_abort_deref_mut() {
        let handle = tokio::spawn(async { "test" });
        let mut drop_abort = DropAbort::new(handle);
        let _ = drop_abort.0;
    }
}
