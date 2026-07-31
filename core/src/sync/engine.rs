use std::path::{Path, PathBuf};

use tracing::{debug, info};

use crate::conflict::detector::{detect_conflict, ConflictStatus};
use crate::filesystem::atomic::cleanup_stale_temps;
use crate::filesystem::io::{file_size, hash_file_path, modified_time};
use crate::filesystem::Blake3Hash;
use crate::filesystem::watcher::WatchEvent;
use crate::index::compare::{compare_manifests, ManifestDiff};
use crate::index::scanner::scan_vault;
use crate::index::state::{FileState, Manifest, RevisionId, SyncState, Tombstone};
use crate::index::store::Store;
use crate::network::peer::PeerConnection;
use crate::storage::db;
use crate::sync::delta::SyncOperation;
use crate::sync::queue::{QueueEntry, SyncQueue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStateMachine {
    Idle,
    Discovering,
    Connecting,
    Syncing,
    Conflict,
    Offline,
    Error,
}

pub struct SyncEngine {
    vault_path: PathBuf,
    store: Store,
    queue: SyncQueue,
    state: SyncStateMachine,
    device_id: String,
    revision_counter: RevisionId,
    _peer: Option<PeerConnection>,
}

impl SyncEngine {
    pub async fn new(vault_path: PathBuf, device_id: String) -> Result<Self, anyhow::Error> {
        db::ensure_db_directory(&vault_path)?;
        cleanup_stale_temps(&vault_path)?;
        let store = db::open_db(&vault_path)?;

        let mut engine = Self {
            vault_path,
            store,
            queue: SyncQueue::new(),
            state: SyncStateMachine::Idle,
            device_id,
            revision_counter: 0,
            _peer: None,
        };

        engine.load_revision_counter()?;
        Ok(engine)
    }

    pub fn state(&self) -> SyncStateMachine {
        self.state
    }

    pub fn vault_path(&self) -> &Path {
        &self.vault_path
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Record a file that came from a remote peer: upsert its state as Synced.
    pub fn record_remote_file(
        &mut self,
        path: &Path,
        content_hash: &Blake3Hash,
        size: u64,
        modified_at: i64,
    ) -> Result<(), anyhow::Error> {
        self.revision_counter += 1;
        let state = FileState::new(
            path.to_owned(),
            *content_hash,
            size,
            modified_at,
            self.revision_counter,
        );
        self.store.upsert_file_state(&state)?;
        self.save_revision_counter()?;
        Ok(())
    }

    /// Mark a file as synced in the local index (after successfully pushing it).
    pub fn mark_synced(&mut self, path: &Path) -> Result<(), anyhow::Error> {
        let rel_str = path.to_string_lossy().to_string();
        if let Some(mut existing) = self.store.get_file_state(&rel_str)? {
            existing.sync_state = SyncState::Synced;
            self.store.upsert_file_state(&existing)?;
        }
        Ok(())
    }

    /// Mark a file as in conflict in the local index.
    pub fn mark_conflict(&mut self, path: &Path) -> Result<(), anyhow::Error> {
        let rel_str = path.to_string_lossy().to_string();
        if let Some(mut existing) = self.store.get_file_state(&rel_str)? {
            existing.sync_state = SyncState::Conflict;
            self.store.upsert_file_state(&existing)?;
        }
        Ok(())
    }

    pub fn set_state(&mut self, new_state: SyncStateMachine) {
        debug!("Sync state: {:?} → {:?}", self.state, new_state);
        self.state = new_state;
    }

    /// Initial index: walk the entire vault and store file states.
    pub async fn initial_index(&mut self) -> Result<(), anyhow::Error> {
        info!("Starting initial index of {:?}", self.vault_path);
        self.set_state(SyncStateMachine::Syncing);

        let result = scan_vault(&self.vault_path).await?;
        for file in &result.files {
            self.store.upsert_file_state(file)?;
        }
        self.revision_counter = result.revision_counter;
        self.save_revision_counter()?;

        info!(
            "Indexed {} files (revision counter: {})",
            result.files.len(),
            self.revision_counter
        );
        self.set_state(SyncStateMachine::Idle);
        Ok(())
    }

    /// Incrementally rescan the vault, updating the index for files that were
    /// created, modified, or removed on disk since the last scan. Unlike
    /// `initial_index`, this preserves the revision of unchanged files so the
    /// sync session does not see spurious conflicts.
    pub async fn refresh_index(&mut self) -> Result<(), anyhow::Error> {
        self.set_state(SyncStateMachine::Syncing);

        let result = scan_vault(&self.vault_path).await?;
        let mut on_disk: std::collections::HashMap<PathBuf, FileState> =
            std::collections::HashMap::new();
        for file in &result.files {
            on_disk.insert(file.relative_path.clone(), file.clone());
        }

        let existing = self.store.get_all_file_states()?;
        for state in &existing {
            match on_disk.get(&state.relative_path) {
                Some(disk) => {
                    if disk.content_hash != state.content_hash
                        || disk.modified_at != state.modified_at
                        || disk.size != state.size
                    {
                        self.revision_counter += 1;
                        let mut updated = disk.clone();
                        updated.revision = self.revision_counter;
                        self.store.upsert_file_state(&updated)?;
                        self.queue.push(SyncOperation::Update {
                            path: updated.relative_path.clone(),
                            content_hash: updated.content_hash,
                            size: updated.size,
                            modified_at: updated.modified_at,
                        });
                    }
                }
                None => {
                    // Removed from disk → tombstone
                    self.revision_counter += 1;
                    self.store.delete_file_state(&state.relative_path.to_string_lossy())?;
                    self.store.upsert_tombstone(&Tombstone {
                        relative_path: state.relative_path.clone(),
                        revision: self.revision_counter,
                        deleted_at: chrono::Utc::now().timestamp_millis(),
                    })?;
                    self.queue.push(SyncOperation::Delete {
                        path: state.relative_path.clone(),
                    });
                }
            }
        }

        // New files that were never indexed
        for disk in &result.files {
            if !existing.iter().any(|s| s.relative_path == disk.relative_path) {
                self.revision_counter += 1;
                let mut state = disk.clone();
                state.revision = self.revision_counter;
                self.store.upsert_file_state(&state)?;
                self.queue.push(SyncOperation::Create {
                    path: state.relative_path.clone(),
                    content_hash: state.content_hash,
                    size: state.size,
                    modified_at: state.modified_at,
                });
            }
        }

        self.save_revision_counter()?;
        self.set_state(SyncStateMachine::Idle);
        Ok(())
    }

    /// Handle a filesystem event from the watcher.
    pub async fn handle_event(&mut self, event: WatchEvent) -> Result<(), anyhow::Error> {
        match event {
            WatchEvent::Created(path) | WatchEvent::Modified(path) => {
                self.handle_file_change(&path).await?;
            }
            WatchEvent::Removed(path) => {
                self.handle_file_deletion(&path).await?;
            }
            WatchEvent::Renamed(from, to) => {
                self.handle_file_rename(&from, &to).await?;
            }
        }
        Ok(())
    }

    async fn handle_file_change(&mut self, path: &Path) -> Result<(), anyhow::Error> {
        if !path.is_file() || crate::filesystem::ignore::should_ignore(path) {
            return Ok(());
        }

        let relative = path.strip_prefix(&self.vault_path)?.to_owned();
        let size = file_size(path)?;
        let modified = modified_time(path)?;
        let hash = hash_file_path(path)?;

        self.revision_counter += 1;

        let state = FileState::new(
            relative.clone(),
            hash,
            size,
            modified,
            self.revision_counter,
        );

        // Check if this was previously in conflict
        if let Some(existing) = self.store.get_file_state(&relative.to_string_lossy())? {
            if existing.sync_state == SyncState::Conflict {
                let mut s = state.clone();
                s.sync_state = SyncState::Synced;
                self.store.upsert_file_state(&s)?;
                return Ok(());
            }
        }

        self.store.upsert_file_state(&state)?;
        self.save_revision_counter()?;

        self.queue.push(SyncOperation::Update {
            path: relative,
            content_hash: hash,
            size,
            modified_at: modified,
        });

        Ok(())
    }

    async fn handle_file_deletion(&mut self, path: &Path) -> Result<(), anyhow::Error> {
        let relative = path.strip_prefix(&self.vault_path)?.to_owned();
        let rel_str = relative.to_string_lossy().to_string();

        if let Some(_state) = self.store.get_file_state(&rel_str)? {
            self.revision_counter += 1;
            self.store.delete_file_state(&rel_str)?;

            self.store.upsert_tombstone(&Tombstone {
                relative_path: relative.clone(),
                revision: self.revision_counter,
                deleted_at: chrono::Utc::now().timestamp_millis(),
            })?;

            self.queue.push(SyncOperation::Delete { path: relative });
            self.save_revision_counter()?;
        }

        Ok(())
    }

    async fn handle_file_rename(
        &mut self,
        from: &Path,
        to: &Path,
    ) -> Result<(), anyhow::Error> {
        let relative_from = from.strip_prefix(&self.vault_path)?.to_owned();
        let relative_to = to.strip_prefix(&self.vault_path)?.to_owned();

        if let Some(state) = self.store.get_file_state(&relative_from.to_string_lossy())? {
            // If content hasn't changed, it's a rename
            if to.is_file() {
                let to_hash = hash_file_path(to)?;
                if to_hash == state.content_hash {
                    self.store.delete_file_state(&relative_from.to_string_lossy())?;
                    let mut new_state = state.clone();
                    new_state.relative_path = relative_to.clone();
                    self.store.upsert_file_state(&new_state)?;

                    self.queue.push(SyncOperation::Rename {
                        from: relative_from,
                        to: relative_to,
                        content_hash: state.content_hash,
                    });
                    return Ok(());
                }
            }
        }

        // Treat as delete + create
        self.handle_file_deletion(from).await?;
        self.handle_file_change(to).await?;

        Ok(())
    }

    /// Generate and return the local manifest for comparison with a peer.
    pub fn build_manifest(&self) -> Manifest {
        let files = self.store.get_all_file_states().unwrap_or_default();
        let tombstones = self.store.get_tombstones().unwrap_or_default();
        Manifest {
            device_id: self.device_id.clone(),
            files,
            tombstones,
            revision_counter: self.revision_counter,
        }
    }

    /// Process a remote manifest and produce sync operations.
    pub fn reconcile(&mut self, remote: &Manifest) -> ManifestDiff {
        let local = self.build_manifest();
        let diff = compare_manifests(&local, remote);

        for (local_file, _remote_file) in &diff.conflicts {
            let rel_str = local_file.relative_path.to_string_lossy().to_string();
            // Update local state to Conflict
            if let Some(mut existing) = self
                .store
                .get_file_state(&rel_str)
                .unwrap_or(None)
            {
                existing.sync_state = SyncState::Conflict;
                let _ = self.store.upsert_file_state(&existing);
            }
        }

        diff
    }

    /// Apply an incoming sync operation (from remote peer).
    pub async fn apply_operation(&mut self, op: &SyncOperation) -> Result<(), anyhow::Error> {
        match op {
            SyncOperation::Create {
                path,
                content_hash,
                size,
                modified_at,
            } => {
                self.apply_create(path, content_hash, *size, *modified_at)
                    .await?;
            }
            SyncOperation::Update {
                path,
                content_hash,
                size,
                modified_at,
            } => {
                self.apply_update(path, content_hash, *size, *modified_at)
                    .await?;
            }
            SyncOperation::Delete { path } => {
                self.apply_delete(path).await?;
            }
            SyncOperation::Rename {
                from,
                to,
                content_hash,
            } => {
                self.apply_rename(from, to, content_hash).await?;
            }
        }
        Ok(())
    }

    async fn apply_create(
        &mut self,
        path: &Path,
        _content_hash: &[u8; 32],
        _size: u64,
        _modified_at: i64,
    ) -> Result<(), anyhow::Error> {
        let _rel_str = path.to_string_lossy().to_string();
        let full_path = self.vault_path.join(path);

        if full_path.exists() {
            let hash = hash_file_path(&full_path)?;
            let size = file_size(&full_path)?;
            let modified = modified_time(&full_path)?;
            self.revision_counter += 1;
            let state = FileState::new(path.to_owned(), hash, size, modified, self.revision_counter);
            self.store.upsert_file_state(&state)?;
        }

        Ok(())
    }

    async fn apply_update(
        &mut self,
        path: &Path,
        content_hash: &[u8; 32],
        size: u64,
        modified_at: i64,
    ) -> Result<(), anyhow::Error> {
        let rel_str = path.to_string_lossy().to_string();

        // Check for conflict
        if let Some(existing) = self.store.get_file_state(&rel_str)? {
            if existing.content_hash != *content_hash {
                // Potential conflict - check if local was modified since last sync
                let conflict_status = detect_conflict(&existing, &FileState::new(
                    path.to_owned(),
                    *content_hash,
                    size,
                    modified_at,
                    self.revision_counter,
                ), 0);

                if matches!(conflict_status, ConflictStatus::Conflict(_)) {
                    info!("Conflict detected for {}", rel_str);
                    return Ok(()); // Surface conflict to UI
                }
            }
        }

        self.revision_counter += 1;
        let state = FileState::new(path.to_owned(), *content_hash, size, modified_at, self.revision_counter);
        self.store.upsert_file_state(&state)?;
        self.save_revision_counter()?;

        Ok(())
    }

    async fn apply_delete(&mut self, path: &Path) -> Result<(), anyhow::Error> {
        let rel_str = path.to_string_lossy().to_string();
        let full_path = self.vault_path.join(path);

        if full_path.exists() {
            let _ = tokio::fs::remove_file(&full_path).await;
        }

        self.store.delete_file_state(&rel_str)?;

        self.revision_counter += 1;
        self.store.upsert_tombstone(&Tombstone {
            relative_path: path.to_owned(),
            revision: self.revision_counter,
            deleted_at: chrono::Utc::now().timestamp_millis(),
        })?;

        Ok(())
    }

    async fn apply_rename(
        &mut self,
        from: &Path,
        to: &Path,
        content_hash: &[u8; 32],
    ) -> Result<(), anyhow::Error> {
        let from_str = from.to_string_lossy().to_string();
        let _to_str = to.to_string_lossy().to_string();

        if let Some(mut state) = self.store.get_file_state(&from_str)? {
            self.store.delete_file_state(&from_str)?;
            state.relative_path = to.to_owned();
            state.content_hash = *content_hash;
            self.store.upsert_file_state(&state)?;
        }

        let full_from = self.vault_path.join(from);
        let full_to = self.vault_path.join(to);
        if full_from.exists() && !full_to.exists() {
            let _ = tokio::fs::rename(&full_from, &full_to).await;
        }

        Ok(())
    }

    /// Process queued operations (priority: small files first).
    pub fn process_queue(&mut self) -> Vec<QueueEntry> {
        self.queue.prioritize_small_files();
        let mut batch = Vec::new();
        while let Some(entry) = self.queue.pop_front() {
            batch.push(entry);
            if batch.len() >= 10 {
                break;
            }
        }
        batch
    }

    /// Queue is non-empty.
    pub fn has_pending(&self) -> bool {
        !self.queue.is_empty()
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Store stats
    pub fn file_count(&self) -> u64 {
        self.store.file_count().unwrap_or(0)
    }

    fn load_revision_counter(&mut self) -> Result<(), anyhow::Error> {
        if let Some(val) = self.store.get_config("revision_counter")? {
            self.revision_counter = val.parse()?;
        }
        Ok(())
    }

    fn save_revision_counter(&self) -> Result<(), anyhow::Error> {
        self.store
            .set_config("revision_counter", &self.revision_counter.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_engine() -> (SyncEngine, TempDir) {
        let dir = TempDir::new().unwrap();
        let engine = SyncEngine::new(dir.path().to_owned(), "test-device".into())
            .await
            .unwrap();
        (engine, dir)
    }

    #[tokio::test]
    async fn test_initial_index_empty() {
        let (mut engine, _dir) = setup_engine().await;
        engine.initial_index().await.unwrap();
        assert_eq!(engine.file_count(), 0);
    }

    #[tokio::test]
    async fn test_initial_index_with_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.md"), b"hello").unwrap();
        std::fs::write(dir.path().join("b.md"), b"world").unwrap();

        let mut engine = SyncEngine::new(dir.path().to_owned(), "test-device".into())
            .await
            .unwrap();
        engine.initial_index().await.unwrap();
        assert_eq!(engine.file_count(), 2);
    }

    #[tokio::test]
    async fn test_refresh_index_detects_new_modified_and_deleted() {
        let dir = TempDir::new().unwrap();
        let keep_path = dir.path().join("keep.md");
        let change_path = dir.path().join("change.md");
        std::fs::write(&keep_path, b"original keep").unwrap();
        std::fs::write(&change_path, b"original change").unwrap();

        let mut engine = SyncEngine::new(dir.path().to_owned(), "test-device".into())
            .await
            .unwrap();
        engine.initial_index().await.unwrap();
        assert_eq!(engine.file_count(), 2);
        let original_revision = engine
            .build_manifest()
            .files
            .iter()
            .find(|f| f.relative_path == PathBuf::from("keep.md"))
            .unwrap()
            .revision;

        // Modify one file, delete another, create a third
        std::fs::write(&change_path, b"changed content").unwrap();
        std::fs::remove_file(&keep_path).unwrap();
        std::fs::write(dir.path().join("new.md"), b"brand new").unwrap();

        engine.refresh_index().await.unwrap();

        let manifest = engine.build_manifest();
        assert_eq!(manifest.files.len(), 2);
        assert!(manifest.files.iter().any(|f| f.relative_path == PathBuf::from("new.md")));
        assert!(manifest.files.iter().any(|f| f.relative_path == PathBuf::from("change.md")));
        // Deleted file became a tombstone
        assert!(manifest.tombstones.iter().any(|t| t.relative_path == PathBuf::from("keep.md")));
        // Unchanged file keeps its revision → no spurious conflicts
        let keep_revision = manifest
            .files
            .iter()
            .find(|f| f.relative_path == PathBuf::from("change.md"))
            .unwrap()
            .revision;
        assert!(keep_revision > original_revision);
    }

    #[tokio::test]
    async fn test_refresh_index_preserves_revision_of_unchanged() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("stable.md"), b"stable content").unwrap();

        let mut engine = SyncEngine::new(dir.path().to_owned(), "test-device".into())
            .await
            .unwrap();
        engine.initial_index().await.unwrap();
        let rev_before = engine
            .build_manifest()
            .files
            .iter()
            .find(|f| f.relative_path == PathBuf::from("stable.md"))
            .unwrap()
            .revision;

        engine.refresh_index().await.unwrap();
        let rev_after = engine
            .build_manifest()
            .files
            .iter()
            .find(|f| f.relative_path == PathBuf::from("stable.md"))
            .unwrap()
            .revision;
        assert_eq!(rev_before, rev_after);
        assert_eq!(engine.file_count(), 1);
    }

    #[tokio::test]
    async fn test_handle_create_event() {
        let dir = TempDir::new().unwrap();
        let mut engine = SyncEngine::new(dir.path().to_owned(), "test-device".into())
            .await
            .unwrap();
        engine.initial_index().await.unwrap();

        let file_path = dir.path().join("new.md");
        std::fs::write(&file_path, b"new file").unwrap();

        engine
            .handle_event(WatchEvent::Created(file_path))
            .await
            .unwrap();
        assert_eq!(engine.file_count(), 1);
        assert!(engine.has_pending());
    }

    #[tokio::test]
    async fn test_handle_delete_event() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("delete.md");
        std::fs::write(&file_path, b"delete me").unwrap();

        let mut engine = SyncEngine::new(dir.path().to_owned(), "test-device".into())
            .await
            .unwrap();
        engine.initial_index().await.unwrap();
        assert_eq!(engine.file_count(), 1);

        std::fs::remove_file(&file_path).unwrap();
        engine
            .handle_event(WatchEvent::Removed(file_path))
            .await
            .unwrap();
        assert_eq!(engine.file_count(), 0);
    }

    #[tokio::test]
    async fn test_manifest_building() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("manifest.md"), b"test").unwrap();

        let mut engine = SyncEngine::new(dir.path().to_owned(), "test-desk".into())
            .await
            .unwrap();
        engine.initial_index().await.unwrap();

        let manifest = engine.build_manifest();
        assert_eq!(manifest.device_id, "test-desk");
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.files[0].relative_path, PathBuf::from("manifest.md"));
    }

    #[tokio::test]
    async fn test_reconcile_identical() {
        let (mut engine, _dir) = setup_engine().await;
        engine.initial_index().await.unwrap();
        let local_manifest = engine.build_manifest();
        let diff = engine.reconcile(&local_manifest);
        assert!(diff.operations.is_empty());
        assert!(diff.conflicts.is_empty());
    }

    #[tokio::test]
    async fn test_reconcile_with_new_remote_files() {
        let (mut engine, _dir) = setup_engine().await;
        engine.initial_index().await.unwrap();

        let remote = Manifest {
            device_id: "remote".into(),
            files: vec![FileState::new(
                "remote_file.md".into(),
                [1u8; 32],
                100,
                1000,
                1,
            )],
            tombstones: vec![],
            revision_counter: 1,
        };

        let diff = engine.reconcile(&remote);
        assert_eq!(diff.operations.len(), 1);
        assert!(matches!(diff.operations[0], SyncOperation::Create { .. }));
    }

    #[tokio::test]
    async fn test_state_machine_transitions() {
        let (mut engine, _dir) = setup_engine().await;
        assert_eq!(engine.state(), SyncStateMachine::Idle);
        engine.set_state(SyncStateMachine::Syncing);
        assert_eq!(engine.state(), SyncStateMachine::Syncing);
        engine.set_state(SyncStateMachine::Idle);
        assert_eq!(engine.state(), SyncStateMachine::Idle);
    }

    #[tokio::test]
    async fn test_process_queue_prioritizes_small() {
        let (mut engine, _dir) = setup_engine().await;
        engine.queue.push(SyncOperation::Create {
            path: "large.bin".into(),
            content_hash: [0u8; 32],
            size: 100_000_000,
            modified_at: 1,
        });
        engine.queue.push(SyncOperation::Create {
            path: "small.md".into(),
            content_hash: [0u8; 32],
            size: 100,
            modified_at: 1,
        });

        let batch = engine.process_queue();
        assert_eq!(batch.len(), 2);
        match &batch[0].operation {
            SyncOperation::Create { size, .. } => assert_eq!(*size, 100),
            _ => panic!("expected small file first"),
        }
    }
}
