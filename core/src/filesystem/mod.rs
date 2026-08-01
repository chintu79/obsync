pub mod atomic;
pub mod ignore;
pub mod io;
pub mod versioning;

pub type Blake3Hash = [u8; 32];

/// Current Unix time in milliseconds.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn compute_hash_streaming(reader: impl std::io::Read) -> std::io::Result<Blake3Hash> {
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 65536];
    let mut reader = reader;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}
