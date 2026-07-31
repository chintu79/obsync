use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use x25519_dalek::{EphemeralSecret, PublicKey, SharedSecret, StaticSecret};

use crate::network::NetworkError;

const NONCE_SIZE: usize = 12;

/// Derive a shared secret from a static keypair and a peer's public key.
pub fn derive_shared_secret(
    private_key: &StaticSecret,
    peer_public: &PublicKey,
) -> SharedSecret {
    private_key.diffie_hellman(peer_public)
}

/// Derive an AES-256 key from a shared secret.
pub fn derive_encryption_key(shared: &SharedSecret) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(shared.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// Encrypt data with AES-256-GCM.
pub fn encrypt(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, NetworkError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| NetworkError::Encryption(e.to_string()))?;

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

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| NetworkError::Encryption(e.to_string()))?;

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| NetworkError::Encryption(e.to_string()))
}

pub fn generate_ephemeral_keypair() -> (EphemeralSecret, PublicKey) {
    let mut rng = OsRng;
    let secret = EphemeralSecret::random_from_rng(&mut rng);
    let public = PublicKey::from(&secret);
    (secret, public)
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

    #[test]
    fn test_derive_shared_secret() {
        let alice_private = StaticSecret::random_from_rng(OsRng);
        let alice_public = PublicKey::from(&alice_private);
        let bob_private = StaticSecret::random_from_rng(OsRng);
        let bob_public = PublicKey::from(&bob_private);

        let alice_shared = derive_shared_secret(&alice_private, &bob_public);
        let bob_shared = derive_shared_secret(&bob_private, &alice_public);

        assert_eq!(alice_shared.as_bytes(), bob_shared.as_bytes());
    }
}
