use std::{
    collections::HashMap,
    sync::atomic::{AtomicU32, Ordering},
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    lock::{Mutex, MutexGuard},
    FutureExt,
};
use parking_lot::Mutex as SyncMutex;
use rd_interface::Result;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::types::{Object, Response};

type WaitMap = SyncMutex<HashMap<u32, oneshot::Sender<(Response, Vec<u8>)>>>;

pub struct ClientSessionState {
    session_id: Uuid,
    seq_id: AtomicU32,
    wait_map: Arc<WaitMap>,
}

impl ClientSessionState {
    pub fn new() -> Self {
        Self {
            session_id: Uuid::new_v4(),
            seq_id: AtomicU32::new(0),
            wait_map: Arc::new(SyncMutex::new(HashMap::new())),
        }
    }
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }
    pub fn next_seq_id(&self) -> u32 {
        self.seq_id.fetch_add(1, Ordering::Relaxed)
    }
    pub fn wait_for_response(&self, seq_id: u32) -> oneshot::Receiver<(Response, Vec<u8>)> {
        let (tx, rx) = oneshot::channel();
        self.wait_map.lock().insert(seq_id, tx);
        rx
    }
    pub fn send_response(&self, resp: Response, data: Vec<u8>) {
        if let Some(tx) = self.wait_map.lock().remove(&resp.seq_id) {
            let _ = tx.send((resp, data));
        }
    }
}

pub struct Shared<Obj>(Arc<Mutex<Obj>>);

impl<Obj> Shared<Obj> {
    pub fn new(obj: Obj) -> Self {
        Self(Arc::new(Mutex::new(obj)))
    }
    pub async fn lock(&self) -> MutexGuard<'_, Obj> {
        self.0.lock().await
    }
    pub fn poll_lock(&self, cx: &mut Context<'_>) -> Poll<MutexGuard<'_, Obj>> {
        let mut fut = self.0.lock();
        fut.poll_unpin(cx)
    }
}

impl<Obj> Clone for Shared<Obj> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct ServerSessionState<Obj> {
    objects: SyncMutex<HashMap<Object, Shared<Obj>>>,
    obj_id: SyncMutex<u32>,
}

impl<Obj> ServerSessionState<Obj> {
    pub fn new() -> Self {
        Self {
            objects: SyncMutex::new(HashMap::new()),
            obj_id: SyncMutex::new(0),
        }
    }
    pub fn insert_object(&self, obj: Obj) -> Object {
        let mut obj_id = self.obj_id.lock();
        let id = *obj_id;
        *obj_id += 1;

        let key = Object::from_u32(id);
        self.objects.lock().insert(key, Shared::new(obj));
        key
    }
    pub fn remove_object(&self, obj: Object) {
        self.objects.lock().remove(&obj);
    }
    pub fn get_object(&self, obj: Object) -> Result<Shared<Obj>> {
        let obj = self
            .objects
            .lock()
            .get(&obj)
            .ok_or_else(|| rd_interface::Error::NotFound(format!("Object {:?} not found", obj)))?
            .clone();
        Ok(obj)
    }
}
