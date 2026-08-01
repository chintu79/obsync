use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::filesystem::Blake3Hash;

pub type RevisionId = u64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SyncState {
    Synced,
    PendingCreate,
    PendingUpdate,
    PendingDelete,
    Conflict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileState {
    pub relative_path: PathBuf,
    pub content_hash: Blake3Hash,
    pub size: u64,
    pub modified_at: i64,
    pub revision: RevisionId,
    pub sync_state: SyncState,
    /// Content hash the last sync with a peer agreed on. `None` = no agreement
    /// recorded yet (pre-migration rows); conflict detection falls back to mtime.
    pub synced_hash: Option<Blake3Hash>,
}

impl FileState {
    pub fn new(
        relative_path: PathBuf,
        content_hash: Blake3Hash,
        size: u64,
        modified_at: i64,
        revision: RevisionId,
    ) -> Self {
        Self {
            relative_path,
            content_hash,
            size,
            modified_at,
            revision,
            sync_state: SyncState::Synced,
            synced_hash: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tombstone {
    pub relative_path: PathBuf,
    pub revision: RevisionId,
    pub deleted_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub device_id: String,
    pub files: Vec<FileState>,
    pub tombstones: Vec<Tombstone>,
    pub revision_counter: RevisionId,
}
