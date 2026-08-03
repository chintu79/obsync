use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::rngs::OsRng;
use rand::RngCore;

use crate::network::NetworkError;

const NONCE_SIZE: usize = 12;

/// Encrypt data with AES-256-GCM.
pub fn encrypt(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, NetworkError> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| NetworkError::Encryption(e.to_string()))?;

    let mut nonce_bytes = [0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| NetworkError::Encryption(e.to_string()))?;

    let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Decrypt data with AES-256-GCM.
pub fn decrypt(encrypted: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, NetworkError> {
    if encrypted.len() < NONCE_SIZE {
        return Err(NetworkError::Encryption("data too short".into()));
    }

    let (nonce_bytes, ciphertext) = encrypted.split_at(NONCE_SIZE);
    let nonce = Nonce::from_slice(nonce_bytes);

    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| NetworkError::Encryption(e.to_string()))?;

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| NetworkError::Encryption(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let data = b"hello world, this is a secret message";
        let encrypted = encrypt(data, &key).unwrap();
        let decrypted = decrypt(&encrypted, &key).unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn test_encrypt_wrong_key_fails() {
        let mut key1 = [0u8; 32];
        let mut key2 = [0u8; 32];
        OsRng.fill_bytes(&mut key1);
        OsRng.fill_bytes(&mut key2);
        let data = b"secret";
        let encrypted = encrypt(data, &key1).unwrap();
        let result = decrypt(&encrypted, &key2);
        assert!(result.is_err());
    }

    #[test]
    fn test_different_ciphertexts_for_same_data() {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let data = b"same data";
        let a = encrypt(data, &key).unwrap();
        let b = encrypt(data, &key).unwrap();
        // Nonces should differ
        assert_ne!(a, b);
    }

    /// Cross-language conformance: the TS plugin's crypto.test.ts asserts the
    /// same known-answer tag (GCM spec Appendix B, empty PT) via
    /// `@noble/ciphers/aes`. If either implementation stops producing this tag
    /// the two languages have diverged on AES-256-GCM.
    #[test]
    fn test_gcm_known_answer_matches_plugin() {
        use aes_gcm::aead::{AeadInPlace, KeyInit as _};
        let key = [0u8; 32];
        let nonce = [0u8; 12];
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let mut buffer = Vec::new();
        cipher
            .encrypt_in_place(Nonce::from_slice(&nonce), b"", &mut buffer)
            .unwrap();
        assert_eq!(hex::encode(&buffer), "530f8afbc74536b9a963b4f1c4cb738b");
    }
}
