use blake2::{
    digest::{Update, VariableOutput},
    Blake2bVar,
};
use rand::RngCore;

pub(crate) fn encode_packet(shared_key: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);

    let key_stream = derive_key_stream(&salt, shared_key);
    let mut out = Vec::with_capacity(16 + payload.len());
    out.extend_from_slice(&salt);
    for (i, b) in payload.iter().enumerate() {
        out.push(b ^ key_stream[i % key_stream.len()]);
    }
    out
}

pub(crate) fn decode_in_place(shared_key: &[u8], buf: &mut [u8]) -> Option<usize> {
    if buf.len() < 16 {
        return None;
    }
    let (salt, payload) = buf.split_at_mut(16);
    let key_stream = derive_key_stream(salt, shared_key);
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= key_stream[i % key_stream.len()];
    }
    buf.copy_within(16.., 0);
    Some(buf.len() - 16)
}

fn derive_key_stream(salt: &[u8], shared_key: &[u8]) -> [u8; 32] {
    let mut hasher = Blake2bVar::new(32).expect("blake2b output size");
    hasher.update(salt);
    hasher.update(shared_key);
    let mut out = [0u8; 32];
    hasher
        .finalize_variable(&mut out)
        .expect("blake2b finalize");
    out
}
