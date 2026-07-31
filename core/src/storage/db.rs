use std::path::{Path, PathBuf};

use crate::index::store::Store;

pub const DB_FILENAME: &str = "obsync.db";

pub fn open_db(vault_path: &Path) -> rusqlite::Result<Store> {
    let db_path = db_path(vault_path);
    Store::open(&db_path)
}

pub fn db_path(vault_path: &Path) -> PathBuf {
    vault_path.join(".obsync").join(DB_FILENAME)
}

pub fn ensure_db_directory(vault_path: &Path) -> std::io::Result<()> {
    let dir = vault_path.join(".obsync");
    std::fs::create_dir_all(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_db_path() {
        let dir = TempDir::new().unwrap();
        let path = db_path(dir.path());
        assert!(path.to_string_lossy().contains(".obsync/obsync.db"));
    }

    #[test]
    fn test_ensure_db_directory() {
        let dir = TempDir::new().unwrap();
        ensure_db_directory(dir.path()).unwrap();
        assert!(dir.path().join(".obsync").exists());
    }
}
