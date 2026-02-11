use blake2::{
    digest::{Update, VariableOutput},
    Blake2bVar,
};
use rand::RngCore;

pub(crate) fn encode_packet(shared_key: &[u8], payload: &[u8]) -> Vec<u8> {
    // HY2 Salamander: 8-byte salt + XOR payload with BLAKE2b-256(key || salt)
    // Ref: https://v2.hysteria.network/docs/developers/Protocol/
    let mut salt = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut salt);

    let key_stream = derive_key_stream(&salt, shared_key);
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&salt);
    for (i, b) in payload.iter().enumerate() {
        out.push(b ^ key_stream[i % key_stream.len()]);
    }
    out
}

pub(crate) fn decode_in_place(shared_key: &[u8], buf: &mut [u8]) -> Option<usize> {
    if buf.len() < 8 {
        return None;
    }
    let (salt, payload) = buf.split_at_mut(8);
    let key_stream = derive_key_stream(salt, shared_key);
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= key_stream[i % key_stream.len()];
    }
    buf.copy_within(8.., 0);
    Some(buf.len() - 8)
}

fn derive_key_stream(salt: &[u8], shared_key: &[u8]) -> [u8; 32] {
    let mut hasher = Blake2bVar::new(32).expect("blake2b output size");
    hasher.update(shared_key);
    hasher.update(salt);
    let mut out = [0u8; 32];
    hasher
        .finalize_variable(&mut out)
        .expect("blake2b finalize");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let key = b"test-key";
        let payload = b"hello world";
        let enc = encode_packet(key, payload);
        assert_eq!(enc.len(), payload.len() + 8);

        let mut buf = enc.clone();
        let new_len = decode_in_place(key, &mut buf).unwrap();
        assert_eq!(new_len, payload.len());
        assert_eq!(&buf[..new_len], payload);
    }

    #[test]
    fn test_salt_randomizes_output() {
        let key = b"test-key";
        let payload = b"same payload";
        let a = encode_packet(key, payload);
        let b = encode_packet(key, payload);
        assert_ne!(a, b);
    }
}
