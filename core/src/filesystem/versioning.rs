//! Version snapshots: keep a copy of every file before it is overwritten,
//! stored under `.obsync/versions/<relative path>/<epoch millis>`.

use std::cmp::Reverse;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::filesystem::now_millis;

/// Maximum snapshots kept per file.
const MAX_PER_FILE: usize = 32;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SnapshotInfo {
    pub relative_path: String,
    pub timestamp: i64,
    pub size: u64,
}

fn versions_root(vault: &Path) -> PathBuf {
    vault.join(".obsync").join("versions")
}

fn snapshot_dir(vault: &Path, rel: &Path) -> PathBuf {
    versions_root(vault).join(rel)
}

/// Copy the current content of `rel` into the versions store, if the file
/// exists. Prunes old snapshots beyond [`MAX_PER_FILE`].
pub fn snapshot_before_overwrite(vault: &Path, rel: &Path) -> io::Result<()> {
    let src = vault.join(rel);
    if !src.is_file() {
        return Ok(());
    }
    let dir = snapshot_dir(vault, rel);
    fs::create_dir_all(&dir)?;
    let stamp = now_millis();
    let mut dest = dir.join(stamp.to_string());
    if dest.exists() {
        let mut i = 1;
        while dest.exists() {
            dest = dir.join(format!("{stamp}-{i}"));
            i += 1;
        }
    }
    fs::copy(&src, &dest)?;

    // Prune to the newest MAX_PER_FILE snapshots.
    let mut names: Vec<PathBuf> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    names.sort();
    while names.len() > MAX_PER_FILE {
        if let Some(oldest) = names.first() {
            let _ = fs::remove_file(oldest);
        }
        names.remove(0);
    }
    Ok(())
}

/// List snapshots for one file, newest first.
pub fn list_snapshots(vault: &Path, rel: &Path) -> io::Result<Vec<SnapshotInfo>> {
    let dir = snapshot_dir(vault, rel);
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let timestamp = path.file_name().and_then(|n| {
            let name = n.to_string_lossy();
            let millis = name.split('-').next().unwrap_or(&name);
            millis.parse::<i64>().ok()
        });
        let Some(timestamp) = timestamp else {
            continue;
        };
        let size = entry.metadata()?.len();
        out.push(SnapshotInfo {
            relative_path: rel.to_string_lossy().into_owned(),
            timestamp,
            size,
        });
    }
    out.sort_by_key(|a| Reverse(a.timestamp));
    Ok(out)
}

/// List every snapshot across the whole vault, newest first.
pub fn list_all_snapshots(vault: &Path) -> io::Result<Vec<SnapshotInfo>> {
    let root = versions_root(vault);
    let mut out = Vec::new();
    if !root.is_dir() {
        return Ok(out);
    }
    fn walk(dir: &Path, prefix: &Path, out: &mut Vec<SnapshotInfo>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                walk(&path, &prefix.join(entry.file_name()), out)?;
            } else if let Some(timestamp) = entry
                .file_name()
                .to_string_lossy()
                .split('-')
                .next()
                .and_then(|s| s.parse::<i64>().ok())
            {
                out.push(SnapshotInfo {
                    relative_path: prefix.to_string_lossy().into_owned(),
                    timestamp,
                    size: entry.metadata()?.len(),
                });
            }
        }
        Ok(())
    }
    walk(&root, Path::new(""), &mut out)?;
    out.sort_by_key(|a| Reverse(a.timestamp));
    Ok(out)
}

/// Restore a snapshot over the current file, preserving the snapshot's
/// timestamp as the file mtime.
pub fn restore_snapshot(vault: &Path, rel: &Path, timestamp: i64) -> io::Result<()> {
    let dir = snapshot_dir(vault, rel);
    let mut src = dir.join(timestamp.to_string());
    if !src.is_file() {
        let mut names: Vec<PathBuf> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_string_lossy().split('-').next()?.parse::<i64>().ok())
                    == Some(timestamp)
                    && p.is_file()
            })
            .collect();
        names.sort();
        let Some(found) = names.last() else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("snapshot {timestamp} not found for {}", rel.display()),
            ));
        };
        src = found.clone();
    }
    // Keep the pre-restore content as a snapshot too.
    snapshot_before_overwrite(vault, rel)?;
    let dest = vault.join(rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&src, &dest)?;
    // Restore mtime so the sync engine sees this as a fresh local edit.
    let mtime = filetime_from_millis(timestamp);
    let _ = filetime_set(&dest, mtime);
    Ok(())
}

fn filetime_set(path: &Path, ts: std::time::SystemTime) -> io::Result<()> {
    std::fs::File::open(path)?.set_modified(ts)
}

fn filetime_from_millis(ms: i64) -> std::time::SystemTime {
    std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_snapshot_roundtrip() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let notes = vault.join("notes");
        fs::create_dir_all(&notes).unwrap();
        let file = notes.join("idea.md");
        fs::write(&file, b"v1").unwrap();

        snapshot_before_overwrite(vault, Path::new("notes/idea.md")).unwrap();
        fs::write(&file, b"v2").unwrap();
        snapshot_before_overwrite(vault, Path::new("notes/idea.md")).unwrap();

        let snaps = list_snapshots(vault, Path::new("notes/idea.md")).unwrap();
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].relative_path, "notes/idea.md");

        restore_snapshot(vault, Path::new("notes/idea.md"), snaps[1].timestamp).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "v1");
    }

    #[test]
    fn test_prune_keeps_newest() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let notes = vault.join("notes");
        fs::create_dir_all(&notes).unwrap();
        let file = notes.join("idea.md");
        for i in 0..(MAX_PER_FILE + 10) {
            fs::write(&file, format!("v{i}")).unwrap();
            snapshot_before_overwrite(vault, Path::new("notes/idea.md")).unwrap();
        }
        let snaps = list_snapshots(vault, Path::new("notes/idea.md")).unwrap();
        assert!(snaps.len() <= MAX_PER_FILE);
    }

    #[test]
    fn test_restore_missing_snapshot_errors() {
        let dir = TempDir::new().unwrap();
        let err = restore_snapshot(dir.path(), Path::new("x.md"), 12345);
        assert!(err.is_err());
    }
}
