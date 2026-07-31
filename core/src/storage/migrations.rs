use rusqlite::Connection;

const SCHEMA_VERSION: i32 = 1;

pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0);

    if version < SCHEMA_VERSION {
        conn.execute_batch(
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

        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrations() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"file_states".to_string()));
        assert!(tables.contains(&"tombstones".to_string()));
        assert!(tables.contains(&"conflicts".to_string()));
        assert!(tables.contains(&"sync_queue".to_string()));
        assert!(tables.contains(&"device_identity".to_string()));
        assert!(tables.contains(&"config".to_string()));

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn test_idempotent_migrations() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap(); // Should not error
    }
}
