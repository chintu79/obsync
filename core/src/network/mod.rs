pub mod discovery;
pub mod peer;
pub mod protocol;
pub mod transport;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum NetworkError {
    #[error("discovery error: {0}")]
    Discovery(String),

    #[error("connection failed: {0}")]
    Connection(String),

    #[error("peer not found: {0}")]
    PeerNotFound(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("encryption error: {0}")]
    Encryption(String),

    #[error("timeout")]
    Timeout,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
