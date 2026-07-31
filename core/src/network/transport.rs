use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::info;

use crate::network::peer::PeerConnection;
use crate::network::protocol::{HelloPayload, MessageType, ProtocolMessage, PROTOCOL_VERSION};
use crate::network::NetworkError;

pub enum TransportMode {
    Tcp,
}

pub struct TransportConfig {
    pub mode: TransportMode,
    pub bind_address: String,
    pub port: u16,
    pub device_id: String,
    pub device_name: String,
    pub public_key_fingerprint: String,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            mode: TransportMode::Tcp,
            bind_address: "0.0.0.0".into(),
            port: 42042,
            device_id: String::new(),
            device_name: String::new(),
            public_key_fingerprint: String::new(),
        }
    }
}

pub struct Transport {
    config: TransportConfig,
    listener: Arc<Mutex<Option<TcpListener>>>,
    _connections: Arc<Mutex<Vec<PeerConnection>>>,
}

impl Transport {
    pub fn new(config: TransportConfig) -> Self {
        Self {
            config,
            listener: Arc::new(Mutex::new(None)),
            _connections: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn start(&self) -> Result<(), NetworkError> {
        let addr = format!("{}:{}", self.config.bind_address, self.config.port);
        let listener = TcpListener::bind(&addr).await?;
        info!("Transport listening on {}", addr);

        let listener_arc = self.listener.clone();
        *listener_arc.lock().await = Some(listener);

        Ok(())
    }

    pub async fn accept_once(&self) -> Result<PeerConnection, NetworkError> {
        let guard = self.listener.lock().await;
        let listener = guard
            .as_ref()
            .ok_or_else(|| NetworkError::Connection("not listening".into()))?;

        let (stream, addr) = listener.accept().await?;
        info!("Incoming connection from {}", addr);

        let mut stream = stream;
        let len = stream.read_u32_le().await?;
        let mut buf = vec![0u8; len as usize];
        stream.read_exact(&mut buf).await?;

        let msg = ProtocolMessage::from_bytes(&buf)
            .map_err(|e| NetworkError::Protocol(e.to_string()))?;

        if msg.version != PROTOCOL_VERSION {
            return Err(NetworkError::Protocol("version mismatch".into()));
        }

        match msg.message_type {
            MessageType::Hello => {
                let hello: HelloPayload = bincode::deserialize(&msg.payload)
                    .map_err(|e| NetworkError::Protocol(e.to_string()))?;

                let ack = ProtocolMessage::new(MessageType::HelloAck, msg.request_id, vec![]);
                let ack_bytes = ack
                    .to_bytes()
                    .map_err(|e| NetworkError::Protocol(e.to_string()))?;
                stream.write_u32_le(ack_bytes.len() as u32).await?;
                stream.write_all(&ack_bytes).await?;

                Ok(PeerConnection {
                    device_id: hello.device_id,
                    device_name: hello.device_name,
                    address: addr,
                    stream: Arc::new(Mutex::new(stream)),
                })
            }
            _ => Err(NetworkError::Protocol("expected Hello".into())),
        }
    }

    pub async fn stop(&self) {
        let mut guard = self.listener.lock().await;
        if let Some(listener) = guard.take() {
            drop(listener);
            info!("Transport stopped");
        }
    }

    pub fn config(&self) -> &TransportConfig {
        &self.config
    }
}
