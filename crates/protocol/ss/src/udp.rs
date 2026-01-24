// Copy from https://docs.rs/shadowsocks/1.10.3/src/shadowsocks/relay/udprelay/crypto_io.rs.html, LICENSE: MIT

//! Crypto protocol for ShadowSocks UDP
//!
//! Payload with stream cipher
//! ```plain
//! +-------+----------+
//! |  IV   | Payload  |
//! +-------+----------+
//! | Fixed | Variable |
//! +-------+----------+
//! ```
//!
//! Payload with AEAD cipher
//!
//! ```plain
//! UDP (after encryption, *ciphertext*)
//! +--------+-----------+-----------+
//! | NONCE  |  *Data*   |  Data_TAG |
//! +--------+-----------+-----------+
//! | Fixed  | Variable  |   Fixed   |
//! +--------+-----------+-----------+
//! ```
use std::io::{self, Cursor, ErrorKind};

use bytes::{BufMut, BytesMut};
use shadowsocks::crypto::v1::{random_iv_or_salt, Cipher};
use shadowsocks::crypto::{CipherCategory, CipherKind};
use socks5_protocol::{sync::FromIO, Address};

fn write_to_buf(addr: &Address, buf: &mut BytesMut) -> io::Result<()> {
    let mut writer = buf.writer();
    addr.write_to(&mut writer).map_err(|e| e.to_io_err())?;

    Ok(())
}

/// Encrypt payload into ShadowSocks UDP encrypted packet
pub fn encrypt_payload(
    method: CipherKind,
    key: &[u8],
    addr: &Address,
    payload: &[u8],
    dst: &mut BytesMut,
) -> io::Result<()> {
    match method.category() {
        CipherCategory::None => {
            dst.reserve(addr.serialized_len().map_err(|e| e.to_io_err())? + payload.len());
            write_to_buf(addr, dst)?;
            dst.put_slice(payload);
        }
        CipherCategory::Stream => encrypt_payload_stream(method, key, addr, payload, dst)?,
        CipherCategory::Aead => encrypt_payload_aead(method, key, addr, payload, dst)?,
    };

    Ok(())
}

fn encrypt_payload_stream(
    method: CipherKind,
    key: &[u8],
    addr: &Address,
    payload: &[u8],
    dst: &mut BytesMut,
) -> io::Result<()> {
    let iv_len = method.iv_len();
    let addr_len = addr.serialized_len().map_err(|e| e.to_io_err())?;

    // Packet = IV + ADDRESS + PAYLOAD
    dst.reserve(iv_len + addr_len + payload.len());

    // Generate IV
    dst.resize(iv_len, 0);
    let iv = &mut dst[..iv_len];

    if iv_len > 0 {
        random_iv_or_salt(iv);
    }

    let mut cipher = Cipher::new(method, key, iv);

    write_to_buf(addr, dst)?;
    dst.put_slice(payload);
    let m = &mut dst[iv_len..];
    cipher.encrypt_packet(m);

    Ok(())
}

fn encrypt_payload_aead(
    method: CipherKind,
    key: &[u8],
    addr: &Address,
    payload: &[u8],
    dst: &mut BytesMut,
) -> io::Result<()> {
    let salt_len = method.salt_len();
    let addr_len = addr.serialized_len().map_err(|e| e.to_io_err())?;

    // Packet = IV + ADDRESS + PAYLOAD + TAG
    dst.reserve(salt_len + addr_len + payload.len() + method.tag_len());

    // Generate IV
    dst.resize(salt_len, 0);
    let salt = &mut dst[..salt_len];

    if salt_len > 0 {
        random_iv_or_salt(salt);
    }

    let mut cipher = Cipher::new(method, key, salt);

    write_to_buf(addr, dst)?;
    dst.put_slice(payload);

    unsafe {
        dst.advance_mut(method.tag_len());
    }

    let m = &mut dst[salt_len..];
    cipher.encrypt_packet(m);

    Ok(())
}

/// Decrypt payload from ShadowSocks UDP encrypted packet
pub fn decrypt_payload(
    method: CipherKind,
    key: &[u8],
    payload: &mut [u8],
) -> io::Result<(usize, Address)> {
    match method.category() {
        CipherCategory::None => {
            let mut cur = Cursor::new(payload);
            match Address::read_from(&mut cur) {
                Ok(address) => {
                    let pos = cur.position() as usize;
                    let payload = cur.into_inner();
                    payload.copy_within(pos.., 0);
                    Ok((payload.len() - pos, address))
                }
                Err(..) => {
                    let err =
                        io::Error::new(ErrorKind::InvalidData, "parse udp packet Address failed");
                    Err(err)
                }
            }
        }
        CipherCategory::Stream => decrypt_payload_stream(method, key, payload),
        CipherCategory::Aead => decrypt_payload_aead(method, key, payload),
    }
}

fn decrypt_payload_stream(
    method: CipherKind,
    key: &[u8],
    payload: &mut [u8],
) -> io::Result<(usize, Address)> {
    let plen = payload.len();
    let iv_len = method.iv_len();

    if plen < iv_len {
        let err = io::Error::new(ErrorKind::InvalidData, "udp packet too short for iv");
        return Err(err);
    }

    let (iv, data) = payload.split_at_mut(iv_len);

    let mut cipher = Cipher::new(method, key, iv);

    assert!(cipher.decrypt_packet(data));

    let (dn, addr) = parse_packet(data)?;

    let data_start_idx = iv_len + dn;
    let data_length = payload.len() - data_start_idx;
    payload.copy_within(data_start_idx.., 0);

    Ok((data_length, addr))
}

fn decrypt_payload_aead(
    method: CipherKind,
    key: &[u8],
    payload: &mut [u8],
) -> io::Result<(usize, Address)> {
    let plen = payload.len();
    let salt_len = method.salt_len();
    if plen < salt_len {
        let err = io::Error::new(ErrorKind::InvalidData, "udp packet too short for salt");
        return Err(err);
    }

    let (salt, data) = payload.split_at_mut(salt_len);

    let mut cipher = Cipher::new(method, key, salt);
    let tag_len = cipher.tag_len();

    if data.len() < tag_len {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "udp packet too short for tag",
        ));
    }

    if !cipher.decrypt_packet(data) {
        return Err(io::Error::new(io::ErrorKind::Other, "invalid tag-in"));
    }

    // Truncate TAG
    let data_len = data.len() - tag_len;
    let data = &mut data[..data_len];

    let (dn, addr) = parse_packet(data)?;

    let data_length = data_len - dn;
    let data_start_idx = salt_len + dn;
    let data_end_idx = data_start_idx + data_length;

    payload.copy_within(data_start_idx..data_end_idx, 0);

    Ok((data_length, addr))
}

fn parse_packet(buf: &[u8]) -> io::Result<(usize, Address)> {
    let mut cur = Cursor::new(buf);
    match Address::read_from(&mut cur) {
        Ok(address) => {
            let pos = cur.position() as usize;
            Ok((pos, address))
        }
        Err(..) => {
            let err = io::Error::new(ErrorKind::InvalidData, "parse udp packet Address failed");
            Err(err)
        }
    }
}
