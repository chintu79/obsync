use serde::{Deserialize, Serialize};

use crate::security::identity::DeviceIdentity;

pub const PAIRING_PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingPayload {
    pub version: u8,
    pub device_id: String,
    pub device_name: String,
    pub public_key_fingerprint: String,
    pub public_key_bytes: Vec<u8>,
    pub ephemeral_public_key: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingConfirmation {
    pub device_id: String,
    pub device_name: String,
    pub public_key_fingerprint: String,
    pub public_key_bytes: Vec<u8>,
    pub encrypted_ack: Vec<u8>,
}

impl PairingPayload {
    pub fn new(identity: &DeviceIdentity, ephemeral_public: &[u8]) -> Self {
        Self {
            version: PAIRING_PROTOCOL_VERSION,
            device_id: identity.device_id.clone(),
            device_name: identity.device_name.clone(),
            public_key_fingerprint: identity.fingerprint(),
            public_key_bytes: identity.public_key.as_bytes().to_vec(),
            ephemeral_public_key: ephemeral_public.to_vec(),
        }
    }

    pub fn to_qr_data(&self) -> Result<String, Box<dyn std::error::Error>> {
        let json = serde_json::to_string(self)?;
        Ok(json)
    }

    pub fn from_qr_data(data: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(data)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::crypto::generate_ephemeral_keypair;

    #[test]
    fn test_pairing_payload_roundtrip() {
        let identity = DeviceIdentity::generate("Desktop".into());
        let (_, ephemeral_pub) = generate_ephemeral_keypair();
        let payload = PairingPayload::new(&identity, ephemeral_pub.as_bytes());
        let qr = payload.to_qr_data().unwrap();
        let decoded = PairingPayload::from_qr_data(&qr).unwrap();
        assert_eq!(decoded.device_id, identity.device_id);
        assert_eq!(decoded.device_name, "Desktop");
        assert_eq!(decoded.public_key_fingerprint, identity.fingerprint());
    }

    #[test]
    fn test_pairing_payload_no_secrets() {
        let identity = DeviceIdentity::generate("Desktop".into());
        let (_, ephemeral_pub) = generate_ephemeral_keypair();
        let payload = PairingPayload::new(&identity, ephemeral_pub.as_bytes());
        let qr = payload.to_qr_data().unwrap();
        // The QR data should NOT contain the private key
        assert!(!qr.contains("private_key"));
        assert!(!qr.contains("private"));
    }
}
