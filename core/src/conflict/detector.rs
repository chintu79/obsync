use std::path::PathBuf;

use crate::filesystem::Blake3Hash;
use crate::index::state::{FileState, RevisionId};

#[derive(Debug, Clone)]
pub struct ConflictRecord {
    pub path: PathBuf,
    pub local_hash: Blake3Hash,
    pub remote_hash: Blake3Hash,
    pub local_revision: RevisionId,
    pub remote_revision: RevisionId,
    pub base_revision: RevisionId,
    pub detected_at: i64,
}

pub enum ConflictStatus {
    NoConflict,
    Conflict(ConflictRecord),
    SameChange, // Both made identical change
}

pub fn detect_conflict(
    local: &FileState,
    remote: &FileState,
    base_revision: RevisionId,
) -> ConflictStatus {
    // Same hash = no conflict (even if both changed, it's the same change)
    if local.content_hash == remote.content_hash {
        return ConflictStatus::SameChange;
    }

    // Only one side changed since base = no conflict
    if local.revision <= base_revision || remote.revision <= base_revision {
        return ConflictStatus::NoConflict;
    }

    // Both changed since base with different content = conflict
    ConflictStatus::Conflict(ConflictRecord {
        path: local.relative_path.clone(),
        local_hash: local.content_hash,
        remote_hash: remote.content_hash,
        local_revision: local.revision,
        remote_revision: remote.revision,
        base_revision,
        detected_at: chrono::Utc::now().timestamp_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::state::SyncState;

    fn make_state(path: &str, hash_byte: u8, rev: u64) -> FileState {
        let mut hash = [0u8; 32];
        hash[0] = hash_byte;
        FileState {
            relative_path: PathBuf::from(path),
            content_hash: hash,
            size: 100,
            modified_at: (rev * 1000) as i64,
            revision: rev,
            sync_state: SyncState::Synced,
        }
    }

    #[test]
    fn test_no_conflict_identical() {
        let s = make_state("a.md", 1, 2);
        let result = detect_conflict(&s, &s, 1);
        assert!(matches!(result, ConflictStatus::SameChange));
    }

    #[test]
    fn test_no_conflict_one_side_unchanged() {
        let local = make_state("a.md", 1, 2);
        let remote = make_state("a.md", 2, 1); // remote hasn't changed
        let result = detect_conflict(&local, &remote, 1);
        assert!(matches!(result, ConflictStatus::NoConflict));
    }

    #[test]
    fn test_conflict_detected() {
        let local = make_state("a.md", 1, 2);
        let remote = make_state("a.md", 2, 2);
        let result = detect_conflict(&local, &remote, 1);
        assert!(matches!(result, ConflictStatus::Conflict(_)));
    }
}
