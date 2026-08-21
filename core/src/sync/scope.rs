//! Per-device sync scopes: which vault paths a given device may pull and push.
//!
//! The vault index stays full and authoritative on every side; a scope is a
//! pure filter applied when building the manifest a peer sees and when the
//! session diff decides what to pull/push, plus a server-side guard so
//! out-of-scope paths are never served or accepted.
//!
//! An empty entry list means "the whole vault" (the backward-compatible
//! default), so enabling scopes is opt-in: devices approved before scopes
//! existed keep syncing everything until a scope is set.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Whether a scope entry selects a single file or a folder (and everything
/// below it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    File,
    Folder,
}

/// One selection: a single file, or a folder prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeEntry {
    pub kind: ScopeKind,
    /// Vault-relative "/"-separated path.
    pub path: String,
}

/// A set of selections. `entries` empty = whole vault, minus `excludes`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub entries: Vec<ScopeEntry>,
    /// Per-file exclusions (vault-relative "/"-separated paths). An excluded
    /// path never syncs — exclusion wins over any file/folder include. The
    /// field is optional on the wire/config so pre-exclusion JSON loads as-is.
    #[serde(default)]
    pub excludes: Vec<String>,
}

impl Scope {
    /// The whole-vault scope (backward-compatible default).
    pub fn everything() -> Self {
        Self {
            entries: Vec::new(),
            excludes: Vec::new(),
        }
    }

    pub fn is_everything(&self) -> bool {
        self.entries.is_empty() && self.excludes.is_empty()
    }

    /// True when `rel` may sync under this scope. Exclusions are checked
    /// first and win over every include.
    pub fn allows(&self, rel: &Path) -> bool {
        let s = rel.to_string_lossy().replace('\\', "/");
        if self.excludes.iter().any(|e| e == &s) {
            return false;
        }
        if self.entries.is_empty() {
            return true;
        }
        self.entries.iter().any(|e| match e.kind {
            ScopeKind::File => e.path == s,
            ScopeKind::Folder => s == e.path || s.starts_with(&format!("{}/", e.path)),
        })
    }

    /// Combine two scopes: a path is allowed if either allows it and neither
    /// excludes it. An empty entry list contributes no entries (it means
    /// "whole vault" only when the combined result is also empty — see
    /// `effective_scope`); exclusions always union.
    pub fn merge(&self, other: &Scope) -> Scope {
        let mut entries = self.entries.clone();
        entries.extend(other.entries.iter().cloned());
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries.dedup();
        let mut excludes = self.excludes.clone();
        excludes.extend(other.excludes.iter().cloned());
        excludes.sort();
        excludes.dedup();
        Scope { entries, excludes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: ScopeKind, path: &str) -> ScopeEntry {
        ScopeEntry {
            kind,
            path: path.to_string(),
        }
    }

    #[test]
    fn test_everything_allows_all() {
        let scope = Scope::everything();
        assert!(scope.allows(Path::new("notes/a.md")));
        assert!(scope.allows(Path::new("x/y/z.txt")));
        assert!(scope.is_everything());
    }

    #[test]
    fn test_folder_entry_matches_prefix() {
        let scope = Scope {
            entries: vec![entry(ScopeKind::Folder, "notes")],
            excludes: Vec::new(),
        };
        assert!(scope.allows(Path::new("notes")));
        assert!(scope.allows(Path::new("notes/a.md")));
        assert!(scope.allows(Path::new("notes/deep/nested.md")));
        assert!(!scope.allows(Path::new("notes2/a.md")));
        assert!(!scope.allows(Path::new("other/a.md")));
        assert!(!scope.allows(Path::new("notes.md")));
    }

    #[test]
    fn test_file_entry_matches_exactly() {
        let scope = Scope {
            entries: vec![entry(ScopeKind::File, "todo.md")],
            excludes: Vec::new(),
        };
        assert!(scope.allows(Path::new("todo.md")));
        assert!(!scope.allows(Path::new("todo.md.bak")));
        assert!(!scope.allows(Path::new("sub/todo.md")));
        assert!(!scope.allows(Path::new("other.md")));
    }

    #[test]
    fn test_backslash_normalized() {
        let scope = Scope {
            entries: vec![entry(ScopeKind::Folder, "notes")],
            excludes: Vec::new(),
        };
        assert!(scope.allows(Path::new(r"notes\a.md")));
    }

    #[test]
    fn test_merge_union_and_dedup() {
        let a = Scope {
            entries: vec![entry(ScopeKind::Folder, "notes")],
            excludes: Vec::new(),
        };
        let b = Scope {
            excludes: Vec::new(),
            entries: vec![
                entry(ScopeKind::File, "todo.md"),
                entry(ScopeKind::Folder, "notes"),
            ],
        };
        let merged = a.merge(&b);
        assert_eq!(merged.entries.len(), 2);
        assert!(merged.allows(Path::new("notes/a.md")));
        assert!(merged.allows(Path::new("todo.md")));
        assert!(!merged.allows(Path::new("other.md")));
    }

    #[test]
    fn test_merge_is_plain_union() {
        let a = Scope::everything();
        let b = Scope {
            entries: vec![entry(ScopeKind::File, "todo.md")],
            excludes: Vec::new(),
        };
        let merged = a.merge(&b);
        // An empty side contributes nothing — the union carries b's entries
        // (the "whole vault" meaning is only recovered when BOTH are empty).
        assert_eq!(merged.entries.len(), 1);
        assert!(merged.allows(Path::new("todo.md")));
        assert!(!merged.allows(Path::new("other.md")));

        let empty = Scope::everything().merge(&Scope::everything());
        assert!(empty.is_everything());
    }

    #[test]
    fn test_exclude_beats_folder_include() {
        let scope = Scope {
            entries: vec![entry(ScopeKind::Folder, "notes")],
            excludes: vec!["notes/secret.md".to_string()],
        };
        assert!(scope.allows(Path::new("notes/a.md")));
        assert!(scope.allows(Path::new("notes/deep/b.md")));
        assert!(!scope.allows(Path::new("notes/secret.md")));
        assert!(!scope.is_everything());
    }

    #[test]
    fn test_exclude_with_no_entries_is_everything_minus_files() {
        let scope = Scope {
            entries: Vec::new(),
            excludes: vec!["todo.md".to_string()],
        };
        assert!(scope.allows(Path::new("other.md")));
        assert!(scope.allows(Path::new("sub/other.md")));
        assert!(!scope.allows(Path::new("todo.md")));
        // Not "everything": the manifest shortcut must not fire.
        assert!(!scope.is_everything());
    }

    #[test]
    fn test_exclude_matches_exact_path_only() {
        let scope = Scope {
            entries: Vec::new(),
            excludes: vec!["notes/a.md".to_string()],
        };
        assert!(!scope.allows(Path::new("notes/a.md")));
        assert!(scope.allows(Path::new("notes/a.md.bak")));
        assert!(scope.allows(Path::new("notes2/a.md")));
    }

    #[test]
    fn test_exclude_normalizes_queried_backslashes() {
        let scope = Scope {
            entries: Vec::new(),
            excludes: vec!["notes/a.md".to_string()],
        };
        // Excludes are stored with "/" separators (dashboard/plugin normalize
        // before persisting); a Windows-style query path still matches.
        assert!(!scope.allows(Path::new(r"notes\a.md")));
    }

    #[test]
    fn test_merge_unions_excludes() {
        let a = Scope {
            entries: vec![entry(ScopeKind::Folder, "notes")],
            excludes: vec!["notes/a.md".to_string()],
        };
        let b = Scope {
            entries: Vec::new(),
            excludes: vec!["todo.md".to_string(), "notes/a.md".to_string()],
        };
        let merged = a.merge(&b);
        assert_eq!(merged.excludes, vec!["notes/a.md", "todo.md"]);
        assert!(merged.allows(Path::new("notes/b.md")));
        assert!(!merged.allows(Path::new("notes/a.md")));
        assert!(!merged.allows(Path::new("todo.md")));

        // Exclusion on either side wins over an include on the other.
        let c = Scope {
            entries: vec![entry(ScopeKind::File, "todo.md")],
            excludes: Vec::new(),
        };
        let d = Scope {
            entries: Vec::new(),
            excludes: vec!["todo.md".to_string()],
        };
        assert!(!c.merge(&d).allows(Path::new("todo.md")));
    }

    #[test]
    fn test_serde_backward_compat() {
        // Pre-exclusion JSON (no `excludes` key) loads with an empty list.
        let old: Scope = serde_json::from_str(r#"{"entries":[{"kind":"folder","path":"notes"}]}"#)
            .unwrap();
        assert_eq!(old.excludes.len(), 0);
        assert!(old.allows(Path::new("notes/a.md")));
        // New JSON round-trips.
        let new_scope = Scope {
            entries: Vec::new(),
            excludes: vec!["todo.md".to_string()],
        };
        let json = serde_json::to_string(&new_scope).unwrap();
        let back: Scope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, new_scope);
    }
}