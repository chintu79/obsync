pub mod peer;
pub mod protocol;

#[derive(Debug)]
pub enum NetworkError {
    Connection(String),
    Protocol(String),
    Encryption(String),
    Timeout,
    Io(std::io::Error),
}

impl From<std::io::Error> for NetworkError {
    fn from(e: std::io::Error) -> Self {
        NetworkError::Io(e)
    }
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::Connection(msg) => write!(f, "connection failed: {msg}"),
            NetworkError::Protocol(msg) => write!(f, "protocol error: {msg}"),
            NetworkError::Encryption(msg) => write!(f, "encryption error: {msg}"),
            NetworkError::Timeout => write!(f, "timeout"),
            NetworkError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for NetworkError {}
