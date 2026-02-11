use rd_interface::{error::map_other, Address, Error, Result};
use tokio::io::{AsyncRead, AsyncReadExt};

pub(crate) const HY2_TCP_REQUEST: u64 = 0x401;

pub(crate) fn varint_len(x: u64) -> usize {
    if x < (1 << 6) {
        1
    } else if x < (1 << 14) {
        2
    } else if x < (1 << 30) {
        4
    } else {
        8
    }
}

pub(crate) fn write_varint(out: &mut Vec<u8>, mut x: u64) {
    // QUIC variable-length integer encoding
    // 00: 1 byte, 01: 2 bytes, 10: 4 bytes, 11: 8 bytes
    if x < (1 << 6) {
        out.push(x as u8);
    } else if x < (1 << 14) {
        x |= 0b01 << 14;
        out.extend_from_slice(&(x as u16).to_be_bytes());
    } else if x < (1 << 30) {
        x |= 0b10 << 30;
        out.extend_from_slice(&(x as u32).to_be_bytes());
    } else {
        // Up to 2^62-1
        x |= 0b11 << 62;
        out.extend_from_slice(&(x as u64).to_be_bytes());
    }
}

pub(crate) fn decode_varint(input: &[u8]) -> Option<(u64, usize)> {
    let first = *input.first()?;
    let tag = first >> 6;
    match tag {
        0b00 => Some(((first & 0b0011_1111) as u64, 1)),
        0b01 => {
            if input.len() < 2 {
                return None;
            }
            let b0 = input[0] & 0b0011_1111;
            let v = u16::from_be_bytes([b0, input[1]]) as u64;
            Some((v, 2))
        }
        0b10 => {
            if input.len() < 4 {
                return None;
            }
            let b0 = input[0] & 0b0011_1111;
            let v = u32::from_be_bytes([b0, input[1], input[2], input[3]]) as u64;
            Some((v, 4))
        }
        0b11 => {
            if input.len() < 8 {
                return None;
            }
            let b0 = input[0] & 0b0011_1111;
            let v = u64::from_be_bytes([
                b0, input[1], input[2], input[3], input[4], input[5], input[6], input[7],
            ]);
            Some((v, 8))
        }
        _ => None,
    }
}

pub(crate) fn write_tcp_request_with_padding(out: &mut Vec<u8>, target: &Address, padding: &[u8]) {
    write_varint(out, HY2_TCP_REQUEST);
    let addr = address_to_host_port(target);
    let addr_bytes = addr.as_bytes();
    write_varint(out, addr_bytes.len() as u64);
    out.extend_from_slice(addr_bytes);
    write_varint(out, padding.len() as u64);
    out.extend_from_slice(padding);
}

pub(crate) fn write_tcp_response_ok(out: &mut Vec<u8>, padding: &[u8]) {
    write_tcp_response(out, 0x00, "", padding);
}

pub(crate) fn write_tcp_response_error(out: &mut Vec<u8>, msg: &str, padding: &[u8]) {
    write_tcp_response(out, 0x01, msg, padding);
}

fn write_tcp_response(out: &mut Vec<u8>, status: u8, msg: &str, padding: &[u8]) {
    out.push(status);
    write_varint(out, msg.as_bytes().len() as u64);
    out.extend_from_slice(msg.as_bytes());
    write_varint(out, padding.len() as u64);
    out.extend_from_slice(padding);
}

pub(crate) fn address_to_host_port(target: &Address) -> String {
    match target {
        Address::SocketAddr(sa) => sa.to_string(),
        Address::Domain(domain, port) => format!("{domain}:{port}"),
    }
}

pub(crate) async fn read_quic_varint<R: AsyncRead + Unpin>(r: &mut R) -> Result<u64> {
    let mut first = [0u8; 1];
    r.read_exact(&mut first).await.map_err(map_other)?;
    let tag = first[0] >> 6;
    let len = match tag {
        0b00 => 1,
        0b01 => 2,
        0b10 => 4,
        0b11 => 8,
        _ => 1,
    };
    let mut buf = [0u8; 8];
    buf[0] = first[0];
    if len > 1 {
        r.read_exact(&mut buf[1..len]).await.map_err(map_other)?;
    }
    let (v, _) =
        decode_varint(&buf[..len]).ok_or_else(|| Error::Other("hysteria: bad varint".into()))?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use rd_interface::IntoAddress;

    use super::*;

    #[test]
    fn test_varint_roundtrip_boundaries() {
        let values = [
            0u64,
            1,
            63,
            64,
            16383,
            16384,
            (1 << 30) - 1,
            1 << 30,
            (1 << 62) - 1,
        ];
        for v in values {
            let mut buf = Vec::new();
            write_varint(&mut buf, v);
            let (got, used) = decode_varint(&buf).unwrap();
            assert_eq!(got, v);
            assert_eq!(used, buf.len());
        }
    }

    #[test]
    fn test_tcp_request_encoding() {
        let target = "example.com:80".into_address().unwrap();
        let padding = b"pad";
        let mut out = Vec::new();
        write_tcp_request_with_padding(&mut out, &target, padding);

        let (id, n1) = decode_varint(&out).unwrap();
        assert_eq!(id, HY2_TCP_REQUEST);
        let (addr_len, n2) = decode_varint(&out[n1..]).unwrap();
        let addr_len = addr_len as usize;
        let start = n1 + n2;
        let addr = std::str::from_utf8(&out[start..start + addr_len]).unwrap();
        assert_eq!(addr, "example.com:80");
        let (pad_len, n3) = decode_varint(&out[start + addr_len..]).unwrap();
        assert_eq!(pad_len as usize, padding.len());
        let pad_start = start + addr_len + n3;
        assert_eq!(&out[pad_start..pad_start + padding.len()], padding);
        assert_eq!(pad_start + padding.len(), out.len());
    }
}
