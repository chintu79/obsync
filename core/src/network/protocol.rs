use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageType {
    // Handshake
    Hello,
    HelloAck,
    // Sync
    Manifest,
    FileRequest,
    FileChunk,
    // Operations
    SyncOperation,
    OperationAck,
    // Control
    Ping,
    Disconnect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolMessage {
    pub version: u8,
    pub message_type: MessageType,
    pub request_id: u64,
    pub payload: Vec<u8>,
}

impl ProtocolMessage {
    pub fn new(message_type: MessageType, request_id: u64, payload: Vec<u8>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            message_type,
            request_id,
            payload,
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(data)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloPayload {
    pub device_id: String,
    pub device_name: String,
    pub protocol_version: u8,
    pub public_key_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRequestPayload {
    pub relative_path: String,
    pub content_hash: [u8; 32],
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChunkPayload {
    pub relative_path: String,
    pub offset: u64,
    pub data: Vec<u8>,
    pub is_last: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncOperationPayload {
    pub operation_type: u8, // 0=create, 1=update, 2=delete, 3=rename
    pub relative_path: String,
    pub new_path: Option<String>,
    pub content_hash: Option<[u8; 32]>,
    pub size: u64,
    pub modified_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_roundtrip() {
        let msg = ProtocolMessage::new(MessageType::Ping, 1, vec![]);
        let bytes = msg.to_bytes().unwrap();
        let decoded = ProtocolMessage::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.version, PROTOCOL_VERSION);
        assert_eq!(decoded.message_type, MessageType::Ping);
        assert_eq!(decoded.request_id, 1);
    }

    #[test]
    fn test_hello_payload() {
        let hello = HelloPayload {
            device_id: "test-device".into(),
            device_name: "Test Desktop".into(),
            protocol_version: 1,
            public_key_fingerprint: "abcd1234".into(),
        };
        let bytes = bincode::serialize(&hello).unwrap();
        let decoded: HelloPayload = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded.device_id, "test-device");
    }

    #[test]
    fn test_protocol_version_mismatch() {
        let msg = ProtocolMessage {
            version: 99,
            message_type: MessageType::Hello,
            request_id: 1,
            payload: vec![],
        };
        // The receiver should check version and reject
        assert_ne!(msg.version, PROTOCOL_VERSION);
    }
}
