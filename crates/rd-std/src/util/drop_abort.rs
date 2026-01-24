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
