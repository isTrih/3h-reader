#[cfg(test)]
use aes_gcm::Nonce;
use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

use crate::error::ApiError;

const NONCE_SIZE: usize = 12;

#[derive(Clone)]
pub struct Encryptor {
    cipher: Aes256Gcm,
}

impl Encryptor {
    pub fn new(token: &str) -> Self {
        let key: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        Self {
            cipher: Aes256Gcm::new(&key.into()),
        }
    }

    /// 返回 base64url(nonce || ciphertext || authentication_tag)。
    pub fn encrypt_json(&self, value: &serde_json::Value) -> Result<String, ApiError> {
        let plaintext = serde_json::to_vec(value)
            .map_err(|error| ApiError::internal(format!("序列化响应失败: {error}")))?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext.as_ref())
            .map_err(|_| ApiError::internal("加密响应失败"))?;

        let mut encoded = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        encoded.extend_from_slice(&nonce);
        encoded.extend_from_slice(&ciphertext);
        Ok(URL_SAFE_NO_PAD.encode(encoded))
    }

    #[cfg(test)]
    fn decrypt_json(&self, value: &str) -> serde_json::Value {
        let decoded = URL_SAFE_NO_PAD.decode(value).expect("valid base64url");
        let (nonce, ciphertext) = decoded.split_at(NONCE_SIZE);
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .expect("valid ciphertext");
        serde_json::from_slice(&plaintext).expect("valid json")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn encrypts_with_a_fresh_nonce_and_round_trips() {
        let encryptor = Encryptor::new("a-test-encryption-secret");
        let value = json!({"records": [{"id": 1}]});
        let first = encryptor.encrypt_json(&value).expect("encrypt");
        let second = encryptor.encrypt_json(&value).expect("encrypt");

        assert_ne!(first, second);
        assert_eq!(encryptor.decrypt_json(&first), value);
        assert_eq!(encryptor.decrypt_json(&second), value);
    }
}
