pub mod atomic;
pub mod ignore;
pub mod io;
pub mod watcher;

pub type FileSize = u64;
pub type Blake3Hash = [u8; 32];

pub fn compute_hash(data: &[u8]) -> Blake3Hash {
    blake3::hash(data).into()
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
