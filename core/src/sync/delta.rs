use std::path::PathBuf;

use crate::filesystem::Blake3Hash;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOperation {
    Create {
        path: PathBuf,
        content_hash: Blake3Hash,
        size: u64,
        modified_at: i64,
    },
    Update {
        path: PathBuf,
        content_hash: Blake3Hash,
        size: u64,
        modified_at: i64,
    },
    Delete {
        path: PathBuf,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
        content_hash: Blake3Hash,
    },
}
