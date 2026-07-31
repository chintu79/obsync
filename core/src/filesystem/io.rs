use std::io;
use std::path::Path;

use crate::filesystem::{compute_hash_streaming, Blake3Hash};

pub fn hash_file(path: &Path) -> io::Result<Blake3Hash> {
    let file = std::fs::File::open(path)?;
    compute_hash_streaming(file)
}

pub fn hash_file_path(path: &Path) -> io::Result<Blake3Hash> {
    hash_file(path)
}

pub fn file_size(path: &Path) -> io::Result<u64> {
    Ok(std::fs::metadata(path)?.len())
}

pub fn modified_time(path: &Path) -> io::Result<i64> {
    let metadata = std::fs::metadata(path)?;
    let modified = metadata.modified()?;
    let duration = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Ok(duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[test]
    fn test_hash_file_consistency() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"hello world").unwrap();
        let hash1 = hash_file_path(file.path()).unwrap();
        let hash2 = hash_file_path(file.path()).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_different_files() {
        let mut f1 = NamedTempFile::new().unwrap();
        f1.write_all(b"content a").unwrap();
        let mut f2 = NamedTempFile::new().unwrap();
        f2.write_all(b"content b").unwrap();
        assert_ne!(
            hash_file_path(f1.path()).unwrap(),
            hash_file_path(f2.path()).unwrap()
        );
    }

    #[test]
    fn test_hash_empty_file() {
        let file = NamedTempFile::new().unwrap();
        let hash = hash_file_path(file.path()).unwrap();
        let expected: [u8; 32] = blake3::hash(b"").into();
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_file_size() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"12345").unwrap();
        assert_eq!(file_size(file.path()).unwrap(), 5);
    }

    #[test]
    fn test_nonexistent_file() {
        let path = Path::new("/nonexistent/path/file.md");
        assert!(hash_file_path(path).is_err());
    }
}
