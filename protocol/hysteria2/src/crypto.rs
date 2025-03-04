use blake2::{Blake2b512, Digest};
use rand::RngCore;

pub struct Salamander {
    key: Vec<u8>,
}

impl Salamander {
    pub fn new(password: &str) -> Self {
        Self {
            key: password.as_bytes().to_vec(),
        }
    }

    pub fn encrypt(&self, payload: &mut [u8]) -> Vec<u8> {
        let mut salt = vec![0u8; 8];
        rand::thread_rng().fill_bytes(&mut salt);

        let hash = self.hash_with_salt(&salt);

        // XOR payload with hash
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= hash[i % 32];
        }

        // Prepend salt
        let mut result = Vec::with_capacity(salt.len() + payload.len());
        result.extend_from_slice(&salt);
        result.extend_from_slice(payload);
        result
    }

    pub fn decrypt(&self, input: &[u8]) -> Option<Vec<u8>> {
        if input.len() < 8 {
            return None;
        }

        let (salt, payload) = input.split_at(8);
        let hash = self.hash_with_salt(salt);

        let mut result = payload.to_vec();
        for (i, byte) in result.iter_mut().enumerate() {
            *byte ^= hash[i % 32];
        }

        Some(result)
    }

    fn hash_with_salt(&self, salt: &[u8]) -> [u8; 32] {
        let mut hasher = Blake2b512::new();
        hasher.update(&self.key);
        hasher.update(salt);
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result[..32]);
        hash
    }
}
