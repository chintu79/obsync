use std::collections::HashMap;
use std::path::PathBuf;

use crate::conflict::detector::{resolve_divergence, SideOutcome};
use crate::index::state::{FileState, Manifest, RevisionId, Tombstone};
use crate::sync::delta::SyncOperation;

pub struct ManifestDiff {
    pub operations: Vec<SyncOperation>,
    pub conflicts: Vec<(FileState, FileState)>, // (local, remote)
    pub remote_revision_counter: RevisionId,
}

pub fn compare_manifests(
    local: &Manifest,
    remote: &Manifest,
) -> ManifestDiff {
    let local_map: HashMap<&PathBuf, &FileState> =
        local.files.iter().map(|f| (&f.relative_path, f)).collect();
    let remote_map: HashMap<&PathBuf, &FileState> =
        remote.files.iter().map(|f| (&f.relative_path, f)).collect();

    let local_tombstones: HashMap<&PathBuf, &Tombstone> =
        local.tombstones.iter().map(|t| (&t.relative_path, t)).collect();
    let remote_tombstones: HashMap<&PathBuf, &Tombstone> =
        remote.tombstones.iter().map(|t| (&t.relative_path, t)).collect();

    let mut operations = Vec::new();
    let mut conflicts = Vec::new();

    // Files in remote but not in local → create
    for (path, remote_file) in &remote_map {
        if !local_map.contains_key(path) && !local_tombstones.contains_key(path) {
            operations.push(SyncOperation::Create {
                path: (*path).clone(),
                content_hash: remote_file.content_hash,
                size: remote_file.size,
                modified_at: remote_file.modified_at,
            });
        }
    }

    // Files in local but not in remote → create (upload)
    for (path, local_file) in &local_map {
        if !remote_map.contains_key(path) && !remote_tombstones.contains_key(path) {
            operations.push(SyncOperation::Create {
                path: (*path).clone(),
                content_hash: local_file.content_hash,
                size: local_file.size,
                modified_at: local_file.modified_at,
            });
        }
    }

    // Files in both → compare hashes
    for (path, local_file) in &local_map {
        if let Some(remote_file) = remote_map.get(path) {
            if local_file.content_hash != remote_file.content_hash {
                match resolve_divergence(local_file, remote_file) {
                    SideOutcome::Conflict => {
                        conflicts.push(((*local_file).clone(), (*remote_file).clone()))
                    }
                    SideOutcome::LocalWins => {
                        operations.push(SyncOperation::Update {
                            path: (*path).clone(),
                            content_hash: local_file.content_hash,
                            size: local_file.size,
                            modified_at: local_file.modified_at,
                        });
                    }
                    SideOutcome::RemoteWins => {
                        operations.push(SyncOperation::Update {
                            path: (*path).clone(),
                            content_hash: remote_file.content_hash,
                            size: remote_file.size,
                            modified_at: remote_file.modified_at,
                        });
                    }
                }
            }
        }
    }

    // Tombstone handling: deleted on one side
    for (path, _local_file) in &local_map {
        if remote_tombstones.contains_key(path) {
            operations.push(SyncOperation::Delete {
                path: (*path).clone(),
            });
        }
    }
    for (path, _remote_file) in &remote_map {
        if local_tombstones.contains_key(path) {
            operations.push(SyncOperation::Delete {
                path: (*path).clone(),
            });
        }
    }

    ManifestDiff {
        operations,
        conflicts,
        remote_revision_counter: remote.revision_counter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::state::SyncState;
    use std::path::PathBuf;

    fn make_file(path: &str, hash_byte: u8, rev: u64) -> FileState {
        let mut hash = [0u8; 32];
        hash[0] = hash_byte;
        FileState {
            relative_path: PathBuf::from(path),
            content_hash: hash,
            size: 100,
            modified_at: (rev * 1000) as i64,
            revision: rev,
            sync_state: SyncState::Synced,
            synced_hash: None,
        }
    }

    #[test]
    fn test_identical_manifests_no_ops() {
        let local = Manifest {
            device_id: "local".into(),
            files: vec![make_file("a.md", 1, 1)],
            tombstones: vec![],
            revision_counter: 1,
        };
        let remote = Manifest {
            device_id: "remote".into(),
            files: vec![make_file("a.md", 1, 1)],
            tombstones: vec![],
            revision_counter: 1,
        };
        let diff = compare_manifests(&local, &remote);
        assert!(diff.operations.is_empty());
        assert!(diff.conflicts.is_empty());
    }

    #[test]
    fn test_new_file_on_remote() {
        let local = Manifest {
            device_id: "local".into(),
            files: vec![],
            tombstones: vec![],
            revision_counter: 0,
        };
        let remote = Manifest {
            device_id: "remote".into(),
            files: vec![make_file("new.md", 2, 1)],
            tombstones: vec![],
            revision_counter: 1,
        };
        let diff = compare_manifests(&local, &remote);
        assert_eq!(diff.operations.len(), 1);
        assert!(matches!(diff.operations[0], SyncOperation::Create { .. }));
    }

    #[test]
    fn test_deleted_on_remote() {
        let local = Manifest {
            device_id: "local".into(),
            files: vec![make_file("gone.md", 1, 1)],
            tombstones: vec![],
            revision_counter: 1,
        };
        let remote = Manifest {
            device_id: "remote".into(),
            files: vec![],
            tombstones: vec![Tombstone {
                relative_path: PathBuf::from("gone.md"),
                revision: 2,
                deleted_at: 2000,
            }],
            revision_counter: 2,
        };
        let diff = compare_manifests(&local, &remote);
        assert!(diff.operations.iter().any(|op| matches!(op, SyncOperation::Delete { .. })));
    }

    #[test]
    fn test_conflicting_changes() {
        // Both sides changed since the agreement (synced_hash = H0) → conflict.
        let mut local = make_file("c.md", 1, 2);
        local.synced_hash = Some(make_file("c.md", 0, 0).content_hash);
        let mut remote = make_file("c.md", 2, 2);
        remote.synced_hash = Some(make_file("c.md", 0, 0).content_hash);
        let local = Manifest {
            device_id: "local".into(),
            files: vec![local],
            tombstones: vec![],
            revision_counter: 2,
        };
        let remote = Manifest {
            device_id: "remote".into(),
            files: vec![remote],
            tombstones: vec![],
            revision_counter: 2,
        };
        let diff = compare_manifests(&local, &remote);
        assert_eq!(diff.conflicts.len(), 1);
    }

    #[test]
    fn test_rename_detection() {
        // Same content hash, different paths
        let mut h = [0u8; 32];
        h[0] = 42;
        let local = Manifest {
            device_id: "local".into(),
            files: vec![FileState {
                relative_path: PathBuf::from("old.md"),
                content_hash: h,
                size: 50,
                modified_at: 1000,
                revision: 1,
                sync_state: SyncState::Synced,
                synced_hash: None,
            }],
            tombstones: vec![],
            revision_counter: 1,
        };
        let remote = Manifest {
            device_id: "remote".into(),
            files: vec![FileState {
                relative_path: PathBuf::from("new.md"),
                content_hash: h,
                size: 50,
                modified_at: 1000,
                revision: 1,
                sync_state: SyncState::Synced,
                synced_hash: None,
            }],
            tombstones: vec![Tombstone {
                relative_path: PathBuf::from("old.md"),
                revision: 1,
                deleted_at: 1000,
            }],
            revision_counter: 1,
        };
        let diff = compare_manifests(&local, &remote);
        assert!(!diff.operations.is_empty());
    }
}
