use rd_interface::{Address as RDAddr, ReadBuf};
use socks5_protocol::{sync::FromIO, Address, Error};
use std::io::{self, ErrorKind, Read, Result, Write};

pub fn map_err(e: Error) -> rd_interface::Error {
    match e {
        Error::Io(io) => rd_interface::Error::IO(io),
        e => rd_interface::Error::Other(e.into()),
    }
}

pub fn parse_udp(buf: &mut ReadBuf) -> Result<RDAddr> {
    let filled = buf.filled_mut();
    let mut cursor = std::io::Cursor::new(filled);
    let mut header = [0u8; 3];
    cursor.read_exact(&mut header)?;
    let addr = match header[0..3] {
        // TODO: support fragment sequence or at least give another error
        [0x00, 0x00, 0x00] => Address::read_from(&mut cursor).map_err(map_err)?,
        _ => {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!(
                    "server response wrong RSV {} RSV {} FRAG {}",
                    header[0], header[1], header[2]
                ),
            ))
        }
    };

    let pos = cursor.position() as usize;
    cursor.get_mut().copy_within(pos.., 0);

    buf.set_filled(buf.filled().len() - pos);

    Ok(sa2ra(addr))
}

pub fn pack_udp(addr: RDAddr, buf: &[u8], vec: &mut Vec<u8>) -> Result<()> {
    vec.clear();
    let mut cursor = std::io::Cursor::new(vec);
    cursor.write_all(&[0x00, 0x00, 0x00])?;
    ra2sa(addr).write_to(&mut cursor).map_err(map_err)?;
    cursor.write_all(buf)?;

    Ok(())
}

pub fn sa2ra(addr: socks5_protocol::Address) -> rd_interface::Address {
    match addr {
        socks5_protocol::Address::Domain(d, p) => rd_interface::Address::Domain(d, p),
        socks5_protocol::Address::SocketAddr(s) => rd_interface::Address::SocketAddr(s),
    }
}
pub fn ra2sa(addr: rd_interface::Address) -> socks5_protocol::Address {
    match addr {
        rd_interface::Address::Domain(d, p) => socks5_protocol::Address::Domain(d, p),
        rd_interface::Address::SocketAddr(s) => socks5_protocol::Address::SocketAddr(s),
    }
}
