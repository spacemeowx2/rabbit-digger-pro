use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::io::{self, Cursor};

pub const STREAM_REQUEST_TYPE: u8 = 0x401;

#[derive(Debug)]
pub struct StreamRequest {
    pub addr: String,
    pub padding: Vec<u8>,
}

impl StreamRequest {
    pub fn new(addr: String) -> Self {
        Self {
            addr,
            padding: Vec::new(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = BytesMut::new();

        // Write request type
        buf.put_u8(STREAM_REQUEST_TYPE);

        // Write address
        let addr_bytes = self.addr.as_bytes();
        buf.put_u64(addr_bytes.len() as u64);
        buf.extend_from_slice(addr_bytes);

        // Write padding
        buf.put_u64(self.padding.len() as u64);
        buf.extend_from_slice(&self.padding);

        buf.to_vec()
    }

    pub fn decode(mut buf: Cursor<&[u8]>) -> io::Result<Self> {
        // Read and verify request type
        let req_type = buf.get_u8();
        if req_type != STREAM_REQUEST_TYPE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid request type",
            ));
        }

        // Read address
        let addr_len = buf.get_u64() as usize;
        let mut addr = vec![0u8; addr_len];
        buf.copy_to_slice(&mut addr);
        let addr = String::from_utf8(addr)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8"))?;

        // Read padding
        let padding_len = buf.get_u64() as usize;
        let mut padding = vec![0u8; padding_len];
        buf.copy_to_slice(&mut padding);

        Ok(Self { addr, padding })
    }
}

#[derive(Debug)]
pub struct StreamResponse {
    pub status: u8,
    pub message: String,
    pub padding: Vec<u8>,
}

impl StreamResponse {
    pub fn ok() -> Self {
        Self {
            status: 0,
            message: String::new(),
            padding: Vec::new(),
        }
    }

    pub fn error<S: Into<String>>(msg: S) -> Self {
        Self {
            status: 1,
            message: msg.into(),
            padding: Vec::new(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = BytesMut::new();

        // Write status
        buf.put_u8(self.status);

        // Write message
        let msg_bytes = self.message.as_bytes();
        buf.put_u64(msg_bytes.len() as u64);
        buf.extend_from_slice(msg_bytes);

        // Write padding
        buf.put_u64(self.padding.len() as u64);
        buf.extend_from_slice(&self.padding);

        buf.to_vec()
    }

    pub fn decode(mut buf: Cursor<&[u8]>) -> io::Result<Self> {
        // Read status
        let status = buf.get_u8();

        // Read message
        let msg_len = buf.get_u64() as usize;
        let mut msg = vec![0u8; msg_len];
        buf.copy_to_slice(&mut msg);
        let message = String::from_utf8(msg)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8"))?;

        // Read padding
        let padding_len = buf.get_u64() as usize;
        let mut padding = vec![0u8; padding_len];
        buf.copy_to_slice(&mut padding);

        Ok(Self {
            status,
            message,
            padding,
        })
    }
}

#[derive(Debug)]
pub struct UdpMessage {
    pub session_id: u32,
    pub packet_id: u16,
    pub fragment_id: u8,
    pub fragment_count: u8,
    pub addr: String,
    pub payload: Bytes,
}

impl UdpMessage {
    pub fn new(session_id: u32, addr: String, payload: Bytes) -> Self {
        Self {
            session_id,
            packet_id: 0,
            fragment_id: 0,
            fragment_count: 1,
            addr,
            payload,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = BytesMut::new();

        // Write fixed header fields
        buf.put_u32(self.session_id);
        buf.put_u16(self.packet_id);
        buf.put_u8(self.fragment_id);
        buf.put_u8(self.fragment_count);

        // Write address
        let addr_bytes = self.addr.as_bytes();
        buf.put_u64(addr_bytes.len() as u64);
        buf.extend_from_slice(addr_bytes);

        // Write payload
        buf.extend_from_slice(&self.payload);

        buf.to_vec()
    }

    pub fn decode(mut buf: Cursor<&[u8]>) -> io::Result<Self> {
        // Read fixed header fields
        let session_id = buf.get_u32();
        let packet_id = buf.get_u16();
        let fragment_id = buf.get_u8();
        let fragment_count = buf.get_u8();

        // Read address
        let addr_len = buf.get_u64() as usize;
        let mut addr = vec![0u8; addr_len];
        buf.copy_to_slice(&mut addr);
        let addr = String::from_utf8(addr)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8"))?;

        // Read remaining bytes as payload
        let remaining = buf.remaining();
        let mut payload = vec![0u8; remaining];
        buf.copy_to_slice(&mut payload);

        Ok(Self {
            session_id,
            packet_id,
            fragment_id,
            fragment_count,
            addr,
            payload: Bytes::from(payload),
        })
    }
}
