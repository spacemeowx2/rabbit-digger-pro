use rd_interface::{Address, Value};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Hash, PartialEq, Eq)]
pub struct Object(u32);

impl Object {
    pub fn from_u32(id: u32) -> Self {
        Self(id)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub enum RpcValue {
    Null,
    Value(Value),
    ObjectValue(Object, Value),
}

#[derive(Debug, Deserialize, Serialize)]
#[allow(dead_code)]
pub enum Error {
    ObjectNotFound,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum Command {
    // Get into the session.
    Handshake(Uuid),
    TcpConnect(Value, Address),
    TcpBind(Value, Address),
    UdpBind(Value, Address),
    LookupHost(Address),
    Accept(Object),
    Read(Object, u32),
    Write(Object),
    Flush(Object),
    Shutdown(Object),
    RecvFrom(Object),
    SendTo(Object, Address),
    LocalAddr(Object),
    PeerAddr(Object),
    Close(Object),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Request {
    pub cmd: Command,
    pub seq_id: u32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Response {
    pub seq_id: u32,
    pub result: Result<RpcValue, String>,
}

impl Response {
    pub fn into_null(self) -> rd_interface::Result<()> {
        let val = self.result.map_err(rd_interface::Error::other)?;

        match val {
            RpcValue::Null => Ok(()),
            _ => Err(rd_interface::Error::other("not null")),
        }
    }
    pub fn into_value<T>(self) -> rd_interface::Result<T>
    where
        T: DeserializeOwned,
    {
        let val = self.result.map_err(rd_interface::Error::other)?;

        match val {
            RpcValue::Value(v) => Ok(serde_json::from_value(v)?),
            _ => Err(rd_interface::Error::other("invalid response")),
        }
    }

    pub fn into_object_value<T>(self) -> rd_interface::Result<(Object, T)>
    where
        T: DeserializeOwned,
    {
        let val = self.result.map_err(rd_interface::Error::other)?;

        match val {
            RpcValue::ObjectValue(o, v) => Ok((o, serde_json::from_value(v)?)),
            _ => Err(rd_interface::Error::other("invalid response")),
        }
    }
}
