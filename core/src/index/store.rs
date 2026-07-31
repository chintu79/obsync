use std::path::Path;

use rusqlite::{params, Connection, Result as SqlResult};

use crate::index::state::{FileState, SyncState, Tombstone};

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> SqlResult<Self> {
        let conn = Connection::open(path)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> SqlResult<Self> {
        let conn = Connection::open_in_memory()?;
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

            CREATE TABLE IF NOT EXISTS sync_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                operation INTEGER NOT NULL,
                relative_path TEXT NOT NULL,
                content_hash BLOB,
                revision INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                retries INTEGER NOT NULL DEFAULT 0
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
            ",
        )?;
        Ok(())
    }

    pub fn upsert_file_state(&self, state: &FileState) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO file_states (relative_path, content_hash, size, modified_at, revision, sync_state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                state.relative_path.to_string_lossy().as_ref(),
                &state.content_hash[..],
                state.size,
                state.modified_at,
                state.revision,
                state.sync_state.clone() as i32,
            ],
        )?;
        Ok(())
    }

    pub fn get_file_state(&self, path: &str) -> SqlResult<Option<FileState>> {
        let mut stmt = self.conn.prepare(
            "SELECT relative_path, content_hash, size, modified_at, revision, sync_state
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
            }))
        } else {
            Ok(None)
        }
    }

    pub fn delete_file_state(&self, path: &str) -> SqlResult<()> {
        self.conn
            .execute("DELETE FROM file_states WHERE relative_path = ?1", params![path])?;
        Ok(())
    }

    pub fn get_all_file_states(&self) -> SqlResult<Vec<FileState>> {
        let mut stmt = self.conn.prepare(
            "SELECT relative_path, content_hash, size, modified_at, revision, sync_state FROM file_states",
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::Blake3Hash;
    use std::path::PathBuf;

    fn test_hash() -> Blake3Hash {
        let h: [u8; 32] = blake3::hash(b"test").into();
        h
    }

    #[test]
    fn test_store_file_state() {
        let store = Store::open_in_memory().unwrap();
        let state = FileState::new(
            "notes/test.md".into(),
            test_hash(),
            100,
            1000,
            1,
        );
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
}
