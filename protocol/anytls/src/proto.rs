use rd_interface::Address;
use std::{collections::HashMap, net::IpAddr};

pub(crate) const CMD_WASTE: u8 = 0;
pub(crate) const CMD_SYN: u8 = 1;
pub(crate) const CMD_PSH: u8 = 2;
pub(crate) const CMD_FIN: u8 = 3;
pub(crate) const CMD_SETTINGS: u8 = 4;
pub(crate) const CMD_ALERT: u8 = 5;
pub(crate) const CMD_UPDATE_PADDING_SCHEME: u8 = 6;
pub(crate) const CMD_SYNACK: u8 = 7;
pub(crate) const CMD_HEART_REQUEST: u8 = 8;
pub(crate) const CMD_HEART_RESPONSE: u8 = 9;
pub(crate) const CMD_SERVER_SETTINGS: u8 = 10;

pub(crate) const HEADER_LEN: usize = 7;
const CHECK_MARK: i32 = -1;
const DEFAULT_PADDING_SCHEME: &[u8] = b"stop=8\n0=30-30\n1=100-400\n2=400-500,c,500-1000,c,500-1000,c,500-1000,c,500-1000\n3=9-9,500-1000\n4=500-1000\n5=500-1000\n6=500-1000\n7=500-1000";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    pub(crate) cmd: u8,
    pub(crate) sid: u32,
    pub(crate) data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct PaddingScheme {
    raw: Vec<u8>,
    stop: u32,
    scheme: HashMap<u32, Vec<PaddingPart>>,
}

#[derive(Debug, Clone)]
enum PaddingPart {
    Check,
    Range { min: usize, max: usize },
}

impl PaddingScheme {
    pub(crate) fn default() -> Self {
        Self::parse(DEFAULT_PADDING_SCHEME).expect("default AnyTLS padding scheme is valid")
    }

    pub(crate) fn parse(raw: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(raw).ok()?;
        let mut stop = None;
        let mut scheme = HashMap::new();

        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key == "stop" {
                stop = value.parse::<u32>().ok();
                continue;
            }
            let Ok(pkt) = key.parse::<u32>() else {
                continue;
            };
            let mut parts = Vec::new();
            for raw_part in value.split(',') {
                if raw_part == "c" {
                    parts.push(PaddingPart::Check);
                    continue;
                }
                let Some((min, max)) = raw_part.split_once('-') else {
                    continue;
                };
                let (Ok(mut min), Ok(mut max)) = (min.parse::<usize>(), max.parse::<usize>())
                else {
                    continue;
                };
                if min == 0 || max == 0 {
                    continue;
                }
                if min > max {
                    std::mem::swap(&mut min, &mut max);
                }
                parts.push(PaddingPart::Range { min, max });
            }
            scheme.insert(pkt, parts);
        }

        Some(Self {
            raw: raw.to_vec(),
            stop: stop?,
            scheme,
        })
    }

    pub(crate) fn md5_hex(&self) -> String {
        format!("{:x}", md5::compute(&self.raw))
    }

    pub(crate) fn generate_record_payload_sizes(&self, pkt: u32) -> Vec<i32> {
        if pkt >= self.stop {
            return Vec::new();
        }
        self.scheme
            .get(&pkt)
            .map(|parts| {
                parts
                    .iter()
                    .map(|part| match part {
                        PaddingPart::Check => CHECK_MARK,
                        PaddingPart::Range { min, max } => {
                            if min == max {
                                *min as i32
                            } else {
                                rand::random_range(*min..*max) as i32
                            }
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

pub(crate) fn encode_frame(frame: &Frame) -> Vec<u8> {
    assert!(frame.data.len() <= u16::MAX as usize);
    let mut out = Vec::with_capacity(HEADER_LEN + frame.data.len());
    out.push(frame.cmd);
    out.extend_from_slice(&frame.sid.to_be_bytes());
    out.extend_from_slice(&(frame.data.len() as u16).to_be_bytes());
    out.extend_from_slice(&frame.data);
    out
}

pub(crate) fn decode_header(header: [u8; HEADER_LEN]) -> (u8, u32, u16) {
    (
        header[0],
        u32::from_be_bytes([header[1], header[2], header[3], header[4]]),
        u16::from_be_bytes([header[5], header[6]]),
    )
}

pub(crate) fn encode_socks_addr(addr: &Address) -> rd_interface::Result<Vec<u8>> {
    let mut out = Vec::new();
    match addr {
        Address::SocketAddr(socket) => {
            match socket.ip() {
                IpAddr::V4(ip) => {
                    out.push(1);
                    out.extend_from_slice(&ip.octets());
                }
                IpAddr::V6(ip) => {
                    out.push(4);
                    out.extend_from_slice(&ip.octets());
                }
            }
            out.extend_from_slice(&socket.port().to_be_bytes());
        }
        Address::Domain(domain, port) => {
            if domain.len() > u8::MAX as usize {
                return Err(rd_interface::Error::other("domain name too long"));
            }
            out.push(3);
            out.push(domain.len() as u8);
            out.extend_from_slice(domain.as_bytes());
            out.extend_from_slice(&port.to_be_bytes());
        }
    }
    Ok(out)
}

pub(crate) fn encode_settings(padding: &PaddingScheme) -> Vec<u8> {
    format!("v=2\nclient=rdp/anytls\npadding-md5={}", padding.md5_hex()).into_bytes()
}

pub(crate) fn auth_prefix(password: &str, padding: &PaddingScheme) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut out = Sha256::digest(password.as_bytes()).to_vec();
    let padding_len = padding
        .generate_record_payload_sizes(0)
        .first()
        .copied()
        .filter(|len| *len > 0)
        .unwrap_or_default() as usize;
    assert!(padding_len <= u16::MAX as usize);
    out.extend_from_slice(&(padding_len as u16).to_be_bytes());
    out.resize(out.len() + padding_len, 0);
    out
}

pub(crate) fn encode_waste_frame(padding_len: usize) -> Vec<u8> {
    encode_frame(&Frame {
        cmd: CMD_WASTE,
        sid: 0,
        data: vec![0; padding_len],
    })
}

pub(crate) fn apply_padding(
    pkt: u32,
    padding: &PaddingScheme,
    mut payload: Vec<u8>,
) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let sizes = padding.generate_record_payload_sizes(pkt);
    if sizes.is_empty() {
        out.push(payload);
        return out;
    }

    for size in sizes {
        if size == CHECK_MARK {
            if payload.is_empty() {
                break;
            }
            continue;
        }
        let size = size as usize;
        if payload.len() > size {
            out.push(payload.drain(..size).collect());
        } else if !payload.is_empty() {
            let mut chunk = std::mem::take(&mut payload);
            let padding_len = size.saturating_sub(chunk.len() + HEADER_LEN);
            if padding_len > 0 {
                chunk.extend_from_slice(&encode_waste_frame(padding_len));
            }
            out.push(chunk);
        } else {
            out.push(encode_waste_frame(size));
        }
    }

    if !payload.is_empty() {
        out.push(payload);
    }
    if out.is_empty() {
        out.push(Vec::new());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rd_interface::IntoAddress;
    use sha2::Digest;

    #[test]
    fn frame_header_is_command_stream_id_and_big_endian_length() {
        let bytes = encode_frame(&Frame {
            cmd: CMD_PSH,
            sid: 0x01020304,
            data: b"abc".to_vec(),
        });
        assert_eq!(bytes, vec![2, 1, 2, 3, 4, 0, 3, b'a', b'b', b'c']);
        assert_eq!(
            decode_header(bytes[..HEADER_LEN].try_into().unwrap()),
            (CMD_PSH, 0x01020304, 3)
        );
    }

    #[test]
    fn target_address_uses_socks_addr_port_serialization() {
        let domain = "example.com:443".into_address().unwrap();
        assert_eq!(
            encode_socks_addr(&domain).unwrap(),
            b"\x03\x0bexample.com\x01\xbb".to_vec()
        );

        let ip = "127.0.0.1:80".into_address().unwrap();
        assert_eq!(
            encode_socks_addr(&ip).unwrap(),
            vec![1, 127, 0, 0, 1, 0, 80]
        );
    }

    #[test]
    fn settings_are_string_map_with_v2_and_padding_md5() {
        let padding = PaddingScheme::default();
        let settings = encode_settings(&padding);
        let text = std::str::from_utf8(&settings).unwrap();
        assert!(text.contains("v=2"));
        assert!(text.contains("padding-md5="));
        assert!(text.contains(&padding.md5_hex()));
    }

    #[test]
    fn auth_prefix_is_sha256_password_plus_padding0() {
        let padding = PaddingScheme::parse(b"stop=1\n0=4-4").unwrap();
        let prefix = auth_prefix("password", &padding);
        assert_eq!(&prefix[..32], &sha2::Sha256::digest(b"password")[..]);
        assert_eq!(&prefix[32..34], &[0, 4]);
        assert_eq!(&prefix[34..], &[0, 0, 0, 0]);
    }

    #[test]
    fn apply_padding_splits_payload_and_adds_waste() {
        let padding = PaddingScheme::parse(b"stop=3\n1=20-20,c,20-20\n2=20-20").unwrap();
        assert_eq!(apply_padding(1, &padding, b"hello".to_vec())[0].len(), 20);
        let split = apply_padding(2, &padding, vec![1; 30]);
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].len(), 20);
        assert_eq!(split[1].len(), 10);
    }
}
