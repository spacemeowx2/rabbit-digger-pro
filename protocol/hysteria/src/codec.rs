use rd_interface::Address;

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

pub(crate) fn write_tcp_request(out: &mut Vec<u8>, target: &Address) {
    write_varint(out, HY2_TCP_REQUEST);
    match target {
        Address::SocketAddr(sa) => match sa.ip() {
            std::net::IpAddr::V4(v4) => {
                write_varint(out, 0);
                out.extend_from_slice(&v4.octets());
            }
            std::net::IpAddr::V6(v6) => {
                write_varint(out, 2);
                out.extend_from_slice(&v6.octets());
            }
        },
        Address::Domain(domain, _port) => {
            write_varint(out, 1);
            write_varint(out, domain.len() as u64);
            out.extend_from_slice(domain.as_bytes());
        }
    }
    write_varint(out, target.port() as u64);
}
