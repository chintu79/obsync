use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::filesystem::Blake3Hash;
use crate::index::state::RevisionId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictEntry {
    pub id: i64,
    pub relative_path: PathBuf,
    pub local_hash: Option<Blake3Hash>,
    pub remote_hash: Option<Blake3Hash>,
    pub local_revision: Option<RevisionId>,
    pub remote_revision: Option<RevisionId>,
    pub detected_at: i64,
    pub resolved: bool,
}

impl ConflictEntry {
    pub fn conflict_path(&self, device_id: &str) -> PathBuf {
        let stem = self.relative_path.file_stem().unwrap_or_default();
        let ext = self.relative_path.extension().unwrap_or_default();
        let new_name = format!(
            "{}.conflict-{}{}",
            stem.to_string_lossy(),
            device_id,
            if ext.is_empty() {
                String::new()
            } else {
                format!(".{}", ext.to_string_lossy())
            }
        );
        self.relative_path.with_file_name(new_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conflict_path() {
        let entry = ConflictEntry {
            id: 1,
            relative_path: PathBuf::from("notes/idea.md"),
            local_hash: None,
            remote_hash: None,
            local_revision: None,
            remote_revision: None,
            detected_at: 1000,
            resolved: false,
        };
        let conflict_path = entry.conflict_path("pixel");
        assert_eq!(
            conflict_path,
            PathBuf::from("notes/idea.conflict-pixel.md")
        );
    }

    #[test]
    fn test_conflict_path_no_extension() {
        let entry = ConflictEntry {
            id: 1,
            relative_path: PathBuf::from("notes/README"),
            local_hash: None,
            remote_hash: None,
            local_revision: None,
            remote_revision: None,
            detected_at: 1000,
            resolved: false,
        };
        let conflict_path = entry.conflict_path("desktop");
        assert_eq!(conflict_path, PathBuf::from("notes/README.conflict-desktop"));
    }
}
