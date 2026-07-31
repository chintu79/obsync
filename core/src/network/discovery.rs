use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::info;

const SERVICE_TYPE: &str = "_obsync._tcp.local.";

#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub device_id: String,
    pub device_name: String,
    pub addresses: Vec<SocketAddr>,
    pub port: u16,
    pub last_seen: i64,
}

pub struct DiscoveryService {
    peers: Arc<Mutex<HashMap<String, PeerInfo>>>,
    _device_id: String,
    _device_name: String,
}

impl DiscoveryService {
    pub fn new(device_id: String, device_name: String) -> Self {
        Self {
            peers: Arc::new(Mutex::new(HashMap::new())),
            _device_id: device_id,
            _device_name: device_name,
        }
    }

    pub async fn start(&self) -> Result<(), crate::network::NetworkError> {
        info!("Starting mDNS discovery for {}", SERVICE_TYPE);
        Ok(())
    }

    pub async fn stop(&self) {
        info!("Stopping mDNS discovery");
    }

    pub async fn get_peers(&self) -> Vec<PeerInfo> {
        self.peers.lock().await.values().cloned().collect()
    }

    pub async fn get_peer(&self, device_id: &str) -> Option<PeerInfo> {
        self.peers.lock().await.get(device_id).cloned()
    }

    pub async fn add_peer(&self, info: PeerInfo) {
        self.peers.lock().await.insert(info.device_id.clone(), info);
    }

    pub async fn remove_peer(&self, device_id: &str) {
        self.peers.lock().await.remove(device_id);
    }
}
