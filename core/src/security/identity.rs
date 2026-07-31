use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::storage::config::ConfigStore;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

#[derive(Clone)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub device_name: String,
    pub private_key: StaticSecret,
    pub public_key: PublicKey,
    pub created_at: i64,
}

// Manual debug impl to avoid StaticSecret's non-Debug
impl std::fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceIdentity")
            .field("device_id", &self.device_id)
            .field("device_name", &self.device_name)
            .field("public_key", &self.fingerprint())
            .field("created_at", &self.created_at)
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
struct DeviceIdentityData {
    device_id: String,
    device_name: String,
    private_key_bytes: Vec<u8>,
    public_key_bytes: Vec<u8>,
    created_at: i64,
}

impl DeviceIdentity {
    pub fn generate(device_name: String) -> Self {
        let mut rng = OsRng;
        let private_key = StaticSecret::random_from_rng(&mut rng);
        let public_key = PublicKey::from(&private_key);

        Self {
            device_id: uuid::Uuid::new_v4().to_string(),
            device_name,
            private_key,
            public_key,
            created_at: chrono::Utc::now().timestamp_millis(),
        }
    }

    pub fn fingerprint(&self) -> String {
        let bytes = self.public_key.as_bytes();
        let hash = Sha256::digest(bytes);
        hex::encode(&hash[..8])
    }

    pub fn save(&self, store: &ConfigStore) -> Result<(), anyhow::Error> {
        let data = DeviceIdentityData {
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
            private_key_bytes: self.private_key.to_bytes().to_vec(),
            public_key_bytes: self.public_key.as_bytes().to_vec(),
            created_at: self.created_at,
        };
        let bytes = bincode::serialize(&data)?;
        let encoded = BASE64.encode(&bytes);
        store.set("device_identity", &encoded)?;
        Ok(())
    }

    pub fn load(store: &ConfigStore) -> Result<Option<Self>, anyhow::Error> {
        if let Some(encoded) = store.get("device_identity")? {
            let bytes = BASE64.decode(&encoded)?;
            let data: DeviceIdentityData = bincode::deserialize(&bytes)?;
            let mut pk_arr = [0u8; 32];
            pk_arr.copy_from_slice(&data.private_key_bytes);
            let mut pub_arr = [0u8; 32];
            pub_arr.copy_from_slice(&data.public_key_bytes);
            Ok(Some(Self {
                device_id: data.device_id,
                device_name: data.device_name,
                private_key: StaticSecret::from(pk_arr),
                public_key: PublicKey::from(pub_arr),
                created_at: data.created_at,
            }))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_identity() {
        let identity = DeviceIdentity::generate("Test Desktop".into());
        assert!(!identity.device_id.is_empty());
        assert_eq!(identity.device_name, "Test Desktop");
        assert!(identity.created_at > 0);
    }

    #[test]
    fn test_fingerprint_length() {
        let identity = DeviceIdentity::generate("Test".into());
        assert_eq!(identity.fingerprint().len(), 16); // hex of 8 bytes
    }

    #[test]
    fn test_unique_device_ids() {
        let a = DeviceIdentity::generate("A".into());
        let b = DeviceIdentity::generate("B".into());
        assert_ne!(a.device_id, b.device_id);
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let store = ConfigStore::new(dir.path().join("config.bin"));
        let identity = DeviceIdentity::generate("Test".into());
        identity.save(&store).unwrap();
        let loaded = DeviceIdentity::load(&store).unwrap().unwrap();
        assert_eq!(loaded.device_id, identity.device_id);
        assert_eq!(loaded.device_name, identity.device_name);
        assert_eq!(loaded.fingerprint(), identity.fingerprint());
    }
}
