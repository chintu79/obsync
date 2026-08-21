use std::path::Path;

use rusqlite::{params, Connection, Result as SqlResult};

use crate::conflict::record::ConflictEntry;
use crate::filesystem::Blake3Hash;
use crate::index::state::{FileState, SyncState, Tombstone};
use crate::sync::scope::{ScopeEntry, ScopeKind};

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> SqlResult<Self> {
        let conn = Connection::open(path)?;
        // WAL lets the dashboard session and sync sessions (separate
        // connections) read/write concurrently; the busy timeout turns
        // lock contention into a short wait instead of SQLITE_BUSY errors.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> SqlResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> SqlResult<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS file_states (
                relative_path TEXT PRIMARY KEY,
                content_hash BLOB NOT NULL,
                size INTEGER NOT NULL,
                modified_at INTEGER NOT NULL,
                revision INTEGER NOT NULL,
                sync_state INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS tombstones (
                relative_path TEXT PRIMARY KEY,
                revision INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS conflicts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                relative_path TEXT NOT NULL,
                local_hash BLOB,
                remote_hash BLOB,
                local_revision INTEGER,
                remote_revision INTEGER,
                detected_at INTEGER NOT NULL,
                resolved INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS device_identity (
                device_id TEXT PRIMARY KEY,
                public_key BLOB NOT NULL,
                label TEXT,
                paired_at INTEGER NOT NULL,
                last_seen INTEGER
            );

            CREATE TABLE IF NOT EXISTS config (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS shared_scope (
                path TEXT PRIMARY KEY,
                kind INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS device_scopes (
                fingerprint TEXT NOT NULL,
                path TEXT NOT NULL,
                kind INTEGER NOT NULL,
                PRIMARY KEY (fingerprint, path)
            );

            CREATE TABLE IF NOT EXISTS device_flags (
                fingerprint TEXT PRIMARY KEY,
                read_only INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS scope_exclusions (
                fingerprint TEXT NOT NULL,
                path TEXT NOT NULL,
                PRIMARY KEY (fingerprint, path)
            );
            ",
        )?;

        // v2: track the content hash each side last synced per file, so a
        // sequential edit (A edits → sync → B edits → sync) is an update, not
        // a conflict. Existing rows get NULL = "no agreement yet".
        let has_synced_hash: bool = self
            .conn
            .prepare("PRAGMA table_info(file_states)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "synced_hash");
        if !has_synced_hash {
            self.conn
                .execute("ALTER TABLE file_states ADD COLUMN synced_hash BLOB", [])?;
        }

        Ok(())
    }

    pub fn upsert_file_state(&self, state: &FileState) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO file_states (relative_path, content_hash, size, modified_at, revision, sync_state, synced_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                state.relative_path.to_string_lossy().as_ref(),
                &state.content_hash[..],
                state.size,
                state.modified_at,
                state.revision,
                state.sync_state.clone() as i32,
                state.synced_hash.as_ref().map(|h| &h[..]),
            ],
        )?;
        Ok(())
    }

    pub fn get_file_state(&self, path: &str) -> SqlResult<Option<FileState>> {
        let mut stmt = self.conn.prepare(
            "SELECT relative_path, content_hash, size, modified_at, revision, sync_state, synced_hash
             FROM file_states WHERE relative_path = ?1",
        )?;

        let mut rows = stmt.query(params![path])?;
        if let Some(row) = rows.next()? {
            let hash_bytes: Vec<u8> = row.get(1)?;
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&hash_bytes);
            Ok(Some(FileState {
                relative_path: row.get::<_, String>(0)?.into(),
                content_hash: hash,
                size: row.get(2)?,
                modified_at: row.get(3)?,
                revision: row.get(4)?,
                sync_state: match row.get::<_, i32>(5)? {
                    0 => SyncState::Synced,
                    1 => SyncState::PendingCreate,
                    2 => SyncState::PendingUpdate,
                    3 => SyncState::PendingDelete,
                    4 => SyncState::Conflict,
                    _ => SyncState::Synced,
                },
                synced_hash: row.get::<_, Option<Vec<u8>>>(6)?.map(|bytes| {
                    let mut h = [0u8; 32];
                    h.copy_from_slice(&bytes);
                    h
                }),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn delete_file_state(&self, path: &str) -> SqlResult<()> {
        self.conn.execute(
            "DELETE FROM file_states WHERE relative_path = ?1",
            params![path],
        )?;
        Ok(())
    }

    pub fn get_all_file_states(&self) -> SqlResult<Vec<FileState>> {
        let mut stmt = self.conn.prepare(
            "SELECT relative_path, content_hash, size, modified_at, revision, sync_state, synced_hash FROM file_states",
        )?;

        let rows = stmt.query_map([], |row| {
            let hash_bytes: Vec<u8> = row.get(1)?;
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&hash_bytes);
            Ok(FileState {
                relative_path: row.get::<_, String>(0)?.into(),
                content_hash: hash,
                size: row.get(2)?,
                modified_at: row.get(3)?,
                revision: row.get(4)?,
                sync_state: match row.get::<_, i32>(5)? {
                    0 => SyncState::Synced,
                    1 => SyncState::PendingCreate,
                    2 => SyncState::PendingUpdate,
                    3 => SyncState::PendingDelete,
                    4 => SyncState::Conflict,
                    _ => SyncState::Synced,
                },
                synced_hash: row.get::<_, Option<Vec<u8>>>(6)?.map(|bytes| {
                    let mut h = [0u8; 32];
                    h.copy_from_slice(&bytes);
                    h
                }),
            })
        })?;

        let mut files = Vec::new();
        for row in rows {
            files.push(row?);
        }
        Ok(files)
    }

    pub fn file_count(&self) -> SqlResult<u64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM file_states", [], |row| row.get(0))
    }

    // Tombstones
    pub fn upsert_tombstone(&self, tombstone: &Tombstone) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO tombstones (relative_path, revision, deleted_at)
             VALUES (?1, ?2, ?3)",
            params![
                tombstone.relative_path.to_string_lossy().as_ref(),
                tombstone.revision,
                tombstone.deleted_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_tombstones(&self) -> SqlResult<Vec<Tombstone>> {
        let mut stmt = self
            .conn
            .prepare("SELECT relative_path, revision, deleted_at FROM tombstones")?;
        let rows = stmt.query_map([], |row| {
            Ok(Tombstone {
                relative_path: row.get::<_, String>(0)?.into(),
                revision: row.get(1)?,
                deleted_at: row.get(2)?,
            })
        })?;
        let mut tombstones = Vec::new();
        for row in rows {
            tombstones.push(row?);
        }
        Ok(tombstones)
    }

    // Config
    pub fn set_config(&self, key: &str, value: &str) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_config(&self, key: &str) -> SqlResult<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM config WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    // Sync scopes

    fn scope_kind_to_i32(kind: &ScopeKind) -> i32 {
        match kind {
            ScopeKind::File => 0,
            ScopeKind::Folder => 1,
        }
    }

    fn scope_kind_from_i32(v: i32) -> ScopeKind {
        match v {
            0 => ScopeKind::File,
            _ => ScopeKind::Folder,
        }
    }

    /// Paths in the vault-wide "shared" selection (synced to every approved
    /// device). Empty when nothing is shared yet.
    pub fn get_shared_scope(&self) -> SqlResult<Vec<ScopeEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, kind FROM shared_scope ORDER BY path")?;
        let rows = stmt.query_map([], |row| {
            Ok(ScopeEntry {
                path: row.get(0)?,
                kind: Self::scope_kind_from_i32(row.get(1)?),
            })
        })?;
        rows.collect()
    }

    /// Replace the whole shared selection.
    pub fn set_shared_scope(&self, entries: &[ScopeEntry]) -> SqlResult<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM shared_scope", [])?;
        for e in entries {
            tx.execute(
                "INSERT OR REPLACE INTO shared_scope (path, kind) VALUES (?1, ?2)",
                params![e.path, Self::scope_kind_to_i32(&e.kind)],
            )?;
        }
        tx.commit()
    }

    /// Extra paths a specific approved device subscribes to, on top of the
    /// shared selection.
    pub fn get_device_scope(&self, fingerprint: &str) -> SqlResult<Vec<ScopeEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, kind FROM device_scopes WHERE fingerprint = ?1 ORDER BY path",
        )?;
        let rows = stmt.query_map(params![fingerprint], |row| {
            Ok(ScopeEntry {
                path: row.get(0)?,
                kind: Self::scope_kind_from_i32(row.get(1)?),
            })
        })?;
        rows.collect()
    }

    /// Replace a device's optional scope. Empty = shared selection only.
    pub fn set_device_scope(&self, fingerprint: &str, entries: &[ScopeEntry]) -> SqlResult<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM device_scopes WHERE fingerprint = ?1",
            params![fingerprint],
        )?;
        for e in entries {
            tx.execute(
                "INSERT OR REPLACE INTO device_scopes (fingerprint, path, kind) VALUES (?1, ?2, ?3)",
                params![fingerprint, e.path, Self::scope_kind_to_i32(&e.kind)],
            )?;
        }
        tx.commit()
    }

    /// Sentinel fingerprint for the vault-wide (shared) exclusion list.
    pub const SHARED_EXCLUSION_OWNER: &'static str = "";

    /// Per-file exclusions for one owner: `SHARED_EXCLUSION_OWNER` for the
    /// vault-wide list, otherwise an approved device's fingerprint. Excluded
    /// paths never sync regardless of folder/file includes.
    pub fn get_scope_exclusions(&self, owner: &str) -> SqlResult<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT path FROM scope_exclusions WHERE fingerprint = ?1 ORDER BY path",
        )?;
        let rows = stmt.query_map(params![owner], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    /// Replace one owner's exclusion list.
    pub fn set_scope_exclusions(&self, owner: &str, paths: &[String]) -> SqlResult<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM scope_exclusions WHERE fingerprint = ?1",
            params![owner],
        )?;
        for p in paths {
            tx.execute(
                "INSERT OR REPLACE INTO scope_exclusions (fingerprint, path) VALUES (?1, ?2)",
                params![owner, p],
            )?;
        }
        tx.commit()
    }

    /// The vault-wide per-file exclusions.
    pub fn get_shared_exclusions(&self) -> SqlResult<Vec<String>> {
        self.get_scope_exclusions(Self::SHARED_EXCLUSION_OWNER)
    }

    /// Replace the vault-wide exclusions.
    pub fn set_shared_exclusions(&self, paths: &[String]) -> SqlResult<()> {
        self.set_scope_exclusions(Self::SHARED_EXCLUSION_OWNER, paths)
    }

    /// One approved device's per-file exclusions.
    pub fn get_device_exclusions(&self, fingerprint: &str) -> SqlResult<Vec<String>> {
        self.get_scope_exclusions(fingerprint)
    }

    /// Replace one device's exclusions.
    pub fn set_device_exclusions(&self, fingerprint: &str, paths: &[String]) -> SqlResult<()> {
        self.set_scope_exclusions(fingerprint, paths)
    }

    /// Whether a device is read-only: it may pull but never push/delete.
    pub fn get_device_read_only(&self, fingerprint: &str) -> SqlResult<bool> {
        use rusqlite::OptionalExtension;
        let ro: Option<i32> = self
            .conn
            .query_row(
                "SELECT read_only FROM device_flags WHERE fingerprint = ?1",
                params![fingerprint],
                |row| row.get(0),
            )
            .optional()?;
        Ok(ro.map(|v| v != 0).unwrap_or(false))
    }

    pub fn set_device_read_only(&self, fingerprint: &str, read_only: bool) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO device_flags (fingerprint, read_only) VALUES (?1, ?2)",
            params![fingerprint, i32::from(read_only)],
        )?;
        Ok(())
    }

    /// Record a conflict for `relative_path`, replacing any previous
    /// unresolved entry for the same path.
    pub fn record_conflict(
        &self,
        relative_path: &str,
        local_hash: Option<&Blake3Hash>,
        remote_hash: Option<&Blake3Hash>,
    ) -> SqlResult<()> {
        self.conn.execute(
            "DELETE FROM conflicts WHERE relative_path = ?1 AND resolved = 0",
            params![relative_path],
        )?;
        self.conn.execute(
            "INSERT INTO conflicts (relative_path, local_hash, remote_hash, detected_at, resolved)
             VALUES (?1, ?2, ?3, ?4, 0)",
            params![
                relative_path,
                local_hash.map(|h| &h[..]),
                remote_hash.map(|h| &h[..]),
                crate::filesystem::now_millis(),
            ],
        )?;
        Ok(())
    }

    /// List unresolved conflicts, newest first.
    pub fn get_unresolved_conflicts(&self) -> SqlResult<Vec<ConflictEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, relative_path, local_hash, remote_hash, detected_at
             FROM conflicts WHERE resolved = 0 ORDER BY detected_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let local_hash: Option<Vec<u8>> = row.get(2)?;
            let remote_hash: Option<Vec<u8>> = row.get(3)?;
            Ok(ConflictEntry {
                id: row.get(0)?,
                relative_path: row.get::<_, String>(1)?.into(),
                local_hash: local_hash.map(blake_from_vec),
                remote_hash: remote_hash.map(blake_from_vec),
                local_revision: None,
                remote_revision: None,
                detected_at: row.get(4)?,
                resolved: false,
            })
        })?;
        rows.collect()
    }

    /// Mark a conflict entry as resolved.
    pub fn mark_conflict_resolved(&self, id: i64) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE conflicts SET resolved = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }
}

fn blake_from_vec(bytes: Vec<u8>) -> Blake3Hash {
    let mut h = [0u8; 32];
    let n = bytes.len().min(32);
    h[..n].copy_from_slice(&bytes[..n]);
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::Blake3Hash;
    use crate::sync::scope::{ScopeEntry, ScopeKind};
    use std::path::PathBuf;

    fn test_hash() -> Blake3Hash {
        let h: [u8; 32] = blake3::hash(b"test").into();
        h
    }

    #[test]
    fn test_store_file_state() {
        let store = Store::open_in_memory().unwrap();
        let state = FileState::new("notes/test.md".into(), test_hash(), 100, 1000, 1);
        store.upsert_file_state(&state).unwrap();
        let retrieved = store.get_file_state("notes/test.md").unwrap().unwrap();
        assert_eq!(retrieved.relative_path, state.relative_path);
        assert_eq!(retrieved.content_hash, state.content_hash);
        assert_eq!(retrieved.size, 100);
    }

    #[test]
    fn test_store_file_count() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.file_count().unwrap(), 0);
        store
            .upsert_file_state(&FileState::new("a.md".into(), test_hash(), 10, 1, 1))
            .unwrap();
        store
            .upsert_file_state(&FileState::new("b.md".into(), test_hash(), 10, 1, 2))
            .unwrap();
        assert_eq!(store.file_count().unwrap(), 2);
    }

    #[test]
    fn test_delete_file_state() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_file_state(&FileState::new("a.md".into(), test_hash(), 10, 1, 1))
            .unwrap();
        store.delete_file_state("a.md").unwrap();
        assert!(store.get_file_state("a.md").unwrap().is_none());
    }

    #[test]
    fn test_tombstones() {
        let store = Store::open_in_memory().unwrap();
        let t = Tombstone {
            relative_path: "dead.md".into(),
            revision: 5,
            deleted_at: 1000,
        };
        store.upsert_tombstone(&t).unwrap();
        let tombstones = store.get_tombstones().unwrap();
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].relative_path, PathBuf::from("dead.md"));
    }

    #[test]
    fn test_config() {
        let store = Store::open_in_memory().unwrap();
        store.set_config("vault_path", "/test/path").unwrap();
        assert_eq!(
            store.get_config("vault_path").unwrap(),
            Some("/test/path".into())
        );
        assert_eq!(store.get_config("nonexistent").unwrap(), None);
    }

    #[test]
    fn test_shared_scope_roundtrip_and_replace() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.get_shared_scope().unwrap().is_empty());

        let entries = vec![
            ScopeEntry {
                kind: ScopeKind::Folder,
                path: "notes".into(),
            },
            ScopeEntry {
                kind: ScopeKind::File,
                path: "todo.md".into(),
            },
        ];
        store.set_shared_scope(&entries).unwrap();
        assert_eq!(store.get_shared_scope().unwrap(), entries);

        store.set_shared_scope(&[]).unwrap();
        assert!(store.get_shared_scope().unwrap().is_empty());
    }

    #[test]
    fn test_device_scope_is_per_fingerprint() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_device_scope(
                "fp-a",
                &[ScopeEntry {
                    kind: ScopeKind::Folder,
                    path: "family".into(),
                }],
            )
            .unwrap();
        store
            .set_device_scope(
                "fp-b",
                &[ScopeEntry {
                    kind: ScopeKind::File,
                    path: "secret.md".into(),
                }],
            )
            .unwrap();

        assert_eq!(store.get_device_scope("fp-a").unwrap().len(), 1);
        assert_eq!(store.get_device_scope("fp-a").unwrap()[0].path, "family");
        assert_eq!(store.get_device_scope("fp-b").unwrap()[0].path, "secret.md");
        assert!(store.get_device_scope("fp-c").unwrap().is_empty());
    }

    #[test]
    fn test_device_read_only_flag() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.get_device_read_only("fp-a").unwrap());
        store.set_device_read_only("fp-a", true).unwrap();
        assert!(store.get_device_read_only("fp-a").unwrap());
        store.set_device_read_only("fp-a", false).unwrap();
        assert!(!store.get_device_read_only("fp-a").unwrap());
    }

    #[test]
    fn test_shared_exclusions_roundtrip_and_replace() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.get_shared_exclusions().unwrap().is_empty());

        store
            .set_shared_exclusions(&["notes/secret.md".into(), "todo.md".into()])
            .unwrap();
        let mut got = store.get_shared_exclusions().unwrap();
        got.sort();
        assert_eq!(got, vec!["notes/secret.md".to_string(), "todo.md".to_string()]);

        // Replace-whole-set semantics: an empty list clears.
        store.set_shared_exclusions(&[]).unwrap();
        assert!(store.get_shared_exclusions().unwrap().is_empty());
    }

    #[test]
    fn test_device_exclusions_are_per_fingerprint() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_device_exclusions("fp-a", &["private/a.md".into()])
            .unwrap();
        store
            .set_device_exclusions("fp-b", &["work/b.md".into(), "work/c.md".into()])
            .unwrap();

        assert_eq!(
            store.get_device_exclusions("fp-a").unwrap(),
            vec!["private/a.md".to_string()]
        );
        assert_eq!(store.get_device_exclusions("fp-b").unwrap().len(), 2);
        assert!(store.get_device_exclusions("fp-c").unwrap().is_empty());

        // Shared list is independent of device lists.
        store.set_shared_exclusions(&["shared-only.md".into()]).unwrap();
        assert_eq!(
            store.get_shared_exclusions().unwrap(),
            vec!["shared-only.md".to_string()]
        );
        assert_eq!(store.get_device_exclusions("fp-a").unwrap().len(), 1);

        // Replacing one device's list leaves the other untouched.
        store.set_device_exclusions("fp-a", &[]).unwrap();
        assert!(store.get_device_exclusions("fp-a").unwrap().is_empty());
        assert_eq!(store.get_device_exclusions("fp-b").unwrap().len(), 2);
    }
}
