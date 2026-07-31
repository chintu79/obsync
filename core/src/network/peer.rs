use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::info;

use crate::network::protocol::{
    HelloPayload, MessageType, ProtocolMessage, PROTOCOL_VERSION,
};

pub struct PeerConnection {
    pub device_id: String,
    pub device_name: String,
    pub address: SocketAddr,
    pub stream: Arc<Mutex<tokio::net::TcpStream>>,
}

impl PeerConnection {
    /// Timeout for establishing a TCP connection.
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

    pub async fn connect(
        addr: SocketAddr,
        device_id: String,
        device_name: String,
        public_key_fingerprint: String,
    ) -> Result<Self, crate::network::NetworkError> {
        let stream = tokio::time::timeout(Self::CONNECT_TIMEOUT, TcpStream::connect(addr))
            .await
            .map_err(|_| crate::network::NetworkError::Timeout)?
            .map_err(|e| crate::network::NetworkError::Connection(e.to_string()))?;

        let mut connection = Self {
            device_id,
            device_name,
            address: addr,
            stream: Arc::new(Mutex::new(stream)),
        };

        // Perform handshake
        connection.handshake(&public_key_fingerprint).await?;

        Ok(connection)
    }

    async fn handshake(
        &mut self,
        fingerprint: &str,
    ) -> Result<(), crate::network::NetworkError> {
        let hello = HelloPayload {
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
            protocol_version: PROTOCOL_VERSION,
            public_key_fingerprint: fingerprint.to_string(),
        };

        let payload = bincode::serialize(&hello)
            .map_err(|e| crate::network::NetworkError::Protocol(e.to_string()))?;

        let msg = ProtocolMessage::new(MessageType::Hello, 0, payload);
        let bytes = msg
            .to_bytes()
            .map_err(|e| crate::network::NetworkError::Protocol(e.to_string()))?;

        let mut stream = self.stream.lock().await;
        use tokio::io::AsyncWriteExt;
        // Write length-prefixed message
        stream.write_u32_le(bytes.len() as u32).await?;
        stream.write_all(&bytes).await?;

        // Read response
        use tokio::io::AsyncReadExt;
        let len = stream.read_u32_le().await?;
        let mut buf = vec![0u8; len as usize];
        stream.read_exact(&mut buf).await?;

        let response = ProtocolMessage::from_bytes(&buf)
            .map_err(|e| crate::network::NetworkError::Protocol(e.to_string()))?;

        if response.message_type != MessageType::HelloAck {
            return Err(crate::network::NetworkError::Protocol(
                "expected HelloAck".into(),
            ));
        }

        info!("Handshake complete with {}", self.device_name);
        Ok(())
    }

    pub async fn send_message(
        &self,
        msg: &ProtocolMessage,
    ) -> Result<(), crate::network::NetworkError> {
        let bytes = msg
            .to_bytes()
            .map_err(|e| crate::network::NetworkError::Protocol(e.to_string()))?;

        let mut stream = self.stream.lock().await;
        use tokio::io::AsyncWriteExt;
        stream.write_u32_le(bytes.len() as u32).await?;
        stream.write_all(&bytes).await?;
        Ok(())
    }

    pub async fn receive_message(
        &self,
    ) -> Result<ProtocolMessage, crate::network::NetworkError> {
        let mut stream = self.stream.lock().await;
        use tokio::io::AsyncReadExt;

        let len = stream.read_u32_le().await.map_err(|_| {
            crate::network::NetworkError::Connection("connection closed".into())
        })?;

        let mut buf = vec![0u8; len as usize];
        stream.read_exact(&mut buf).await?;

        ProtocolMessage::from_bytes(&buf)
            .map_err(|e| crate::network::NetworkError::Protocol(e.to_string()))
    }
}
