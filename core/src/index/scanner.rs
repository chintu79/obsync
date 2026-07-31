use std::io;
use std::path::Path;

use tokio::task;

use crate::filesystem::ignore::should_ignore;
use crate::filesystem::io::{file_size, hash_file_path, modified_time};
use crate::filesystem::Blake3Hash;
use crate::index::state::{FileState, RevisionId, SyncState};

pub struct ScanResult {
    pub files: Vec<FileState>,
    pub revision_counter: RevisionId,
}

pub async fn scan_vault(vault_path: &Path) -> io::Result<ScanResult> {
    let vault = vault_path.to_owned();
    let mut files = Vec::new();
    let mut dirs_to_scan = vec![vault.clone()];
    let mut revision_counter = 0u64;

    while let Some(dir) = dirs_to_scan.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if should_ignore(&path) {
                continue;
            }

            if path.is_dir() {
                dirs_to_scan.push(path);
            } else if path.is_file() {
                let relative = path
                    .strip_prefix(&vault)
                    .unwrap_or(&path)
                    .to_owned();
                let size = file_size(&path)?;
                let modified = modified_time(&path)?;

                // Spawn hashing to a blocking thread for large files
                let path_clone = path.clone();
                let hash: Blake3Hash = task::spawn_blocking(move || hash_file_path(&path_clone))
                    .await
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))??;

                revision_counter += 1;
                files.push(FileState {
                    relative_path: relative,
                    content_hash: hash,
                    size,
                    modified_at: modified,
                    revision: revision_counter,
                    sync_state: SyncState::Synced,
                });
            }
        }
    }

    Ok(ScanResult {
        files,
        revision_counter,
    })
}

pub async fn scan_file(vault_path: &Path, relative: &Path) -> io::Result<FileState> {
    let full_path = vault_path.join(relative);
    let size = file_size(&full_path)?;
    let modified = modified_time(&full_path)?;

    let fp = full_path.clone();
    let hash = task::spawn_blocking(move || hash_file_path(&fp))
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))??;

    Ok(FileState {
        relative_path: relative.to_owned(),
        content_hash: hash,
        size,
        modified_at: modified,
        revision: 0, // caller assigns
        sync_state: SyncState::Synced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_scan_empty_vault() {
        let dir = TempDir::new().unwrap();
        let result = scan_vault(dir.path()).await.unwrap();
        assert!(result.files.is_empty());
    }

    #[tokio::test]
    async fn test_scan_single_file() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("note.md"), b"hello").unwrap();
        let result = scan_vault(dir.path()).await.unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].relative_path, PathBuf::from("note.md"));
        assert_eq!(result.files[0].size, 5);
    }

    #[tokio::test]
    async fn test_scan_nested_directories() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/nested.md"), b"nested").unwrap();
        let result = scan_vault(dir.path()).await.unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].relative_path, PathBuf::from("sub/nested.md"));
    }

    #[tokio::test]
    async fn test_scan_ignores_hidden() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".hidden.md"), b"hidden").unwrap();
        std::fs::write(dir.path().join("visible.md"), b"visible").unwrap();
        let result = scan_vault(dir.path()).await.unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].relative_path, PathBuf::from("visible.md"));
    }
}
