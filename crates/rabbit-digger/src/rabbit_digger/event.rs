use core::mem::discriminant;
use std::time::SystemTime;

use rd_interface::{Address, Value};
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Debug)]
pub enum EventType {
    NewTcp(Address, Value),
    NewUdp(Address, Value),
    SetStopper(oneshot::Sender<()>),
    CloseConnection,
    Write(u64),
    Read(u64),
    #[allow(dead_code)]
    SendTo(Address, u64),
    #[allow(dead_code)]
    RecvFrom(Address, u64),
}

impl PartialEq for EventType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::SetStopper(_), Self::SetStopper(_)) => true,
            _ => discriminant(self) == discriminant(other),
        }
    }
}

#[derive(Debug)]
pub struct Event {
    pub uuid: Uuid,
    pub events: Vec<EventType>,
    pub time: SystemTime,
}

impl Event {
    pub fn new(uuid: Uuid, events: Vec<EventType>) -> Event {
        Event {
            uuid,
            events,
            time: SystemTime::now(),
        }
    }
}
