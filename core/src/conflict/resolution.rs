use std::path::{Path, PathBuf};

use crate::conflict::record::ConflictEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    KeepLocal,
    KeepRemote,
    KeepBoth,
    OpenFile(PathBuf),
}

pub struct ConflictResolver;

impl ConflictResolver {
    /// Resolve a conflict by choosing which version to keep.
    /// Returns the path that should contain the final content.
    pub fn resolve(
        entry: &ConflictEntry,
        resolution: &Resolution,
        vault_path: &Path,
        device_id: &str,
    ) -> std::io::Result<PathBuf> {
        match resolution {
            Resolution::KeepLocal => {
                // Delete the conflict copy if it exists
                let conflict_path = vault_path.join(entry.conflict_path(device_id));
                let _ = std::fs::remove_file(&conflict_path);
                Ok(vault_path.join(&entry.relative_path))
            }
            Resolution::KeepRemote => {
                // Rename conflict copy to original path
                let original = vault_path.join(&entry.relative_path);
                let conflict = vault_path.join(entry.conflict_path(device_id));
                if conflict.exists() {
                    std::fs::copy(&conflict, &original)?;
                    let _ = std::fs::remove_file(&conflict);
                }
                Ok(original)
            }
            Resolution::KeepBoth => {
                // Both versions already exist (original + conflict copy)
                Ok(vault_path.join(&entry.relative_path))
            }
            Resolution::OpenFile(path) => Ok(path.clone()),
        }
    }

    /// Generate the conflict filename for a given path and device.
    pub fn generate_conflict_path(path: &Path, device_id: &str) -> PathBuf {
        let stem = path.file_stem().unwrap_or_default();
        let ext = path.extension().unwrap_or_default();
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
        path.with_file_name(new_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_conflict_path() {
        let path = PathBuf::from("notes/idea.md");
        let conflict = ConflictResolver::generate_conflict_path(&path, "pixel");
        assert_eq!(conflict, PathBuf::from("notes/idea.conflict-pixel.md"));
    }
}
