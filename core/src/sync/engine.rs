use std::path::{Path, PathBuf};

use tracing::{debug, info};

use crate::filesystem::atomic::cleanup_stale_temps;
use crate::filesystem::io::{file_size, hash_file_path, modified_time};
use crate::filesystem::Blake3Hash;
use crate::index::compare::{compare_manifests, ManifestDiff};
use crate::index::scanner::{scan_vault, scan_vault_incremental};
use crate::index::state::{FileState, Manifest, RevisionId, SyncState, Tombstone};
use crate::index::store::Store;
use crate::network::peer::PeerConnection;
use crate::storage::db;
use crate::sync::delta::SyncOperation;
use crate::sync::scope::{Scope, ScopeEntry};

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
        let mut state = FileState::new(
            path.to_owned(),
            *content_hash,
            size,
            modified_at,
            self.revision_counter,
        );
        // This content was just agreed with the peer — record it as the
        // last-synced hash so future sequential edits become updates.
        state.synced_hash = Some(*content_hash);
        self.store.upsert_file_state(&state)?;
        self.save_revision_counter()?;
        Ok(())
    }

    /// Mark a file as synced in the local index (after successfully pushing it).
    pub fn mark_synced(&mut self, path: &Path) -> Result<(), anyhow::Error> {
        let rel_str = path.to_string_lossy().to_string();
        if let Some(mut existing) = self.store.get_file_state(&rel_str)? {
            existing.sync_state = SyncState::Synced;
            existing.synced_hash = Some(existing.content_hash);
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

    /// Record an unresolved conflict entry in the conflicts table.
    pub fn store_record_conflict(
        &self,
        path: &str,
        local_hash: Option<&Blake3Hash>,
        remote_hash: Option<&Blake3Hash>,
    ) -> Result<(), anyhow::Error> {
        self.store.record_conflict(path, local_hash, remote_hash)?;
        Ok(())
    }

    /// Decide whether applying `remote_hash` to `path` would clobber unsynced
    /// local edits. When it would, record the conflict entry, mark the file
    /// as conflicted, and return the path where the remote content should be
    /// written instead (the conflict copy).
    ///
    /// `force` is used by the client side, which already computed a conflict
    /// via the diff; the server side relies on the synced_hash heuristic.
    pub fn plan_conflict_copy(
        &mut self,
        path: &Path,
        remote_hash: &Blake3Hash,
        force: bool,
    ) -> Result<Option<PathBuf>, anyhow::Error> {
        let rel_str = path.to_string_lossy().to_string();
        let Some(local) = self.store.get_file_state(&rel_str)? else {
            return Ok(None);
        };
        if local.content_hash == *remote_hash {
            return Ok(None);
        }
        let local_unsynced =
            local.synced_hash.is_some() && local.synced_hash != Some(local.content_hash);
        if !force && !local_unsynced {
            return Ok(None);
        }

        let copy = self.conflict_copy_path(path, remote_hash)?;
        self.mark_conflict(path)?;
        self.store
            .record_conflict(&rel_str, Some(&local.content_hash), Some(remote_hash))?;
        info!("Conflict on {:?} -> {}", path, copy.display());
        Ok(Some(copy))
    }

    fn conflict_copy_path(
        &self,
        path: &Path,
        remote_hash: &Blake3Hash,
    ) -> Result<PathBuf, anyhow::Error> {
        use crate::conflict::resolution::ConflictResolver;
        let base = ConflictResolver::generate_conflict_path(path, &self.device_id);
        let full = self.vault_path.join(&base);
        if !full.exists() {
            return Ok(base);
        }
        let stem = base.file_stem().unwrap_or_default().to_string_lossy();
        let ext = base
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let hash8 = hex::encode(&remote_hash[..4]);
        for i in 0..16u32 {
            let suffix = if i == 0 {
                format!("-{hash8}")
            } else {
                format!("-{hash8}-{i}")
            };
            let name = format!("{stem}{suffix}{ext}");
            let candidate = base.with_file_name(&name);
            if !self.vault_path.join(&candidate).exists() {
                return Ok(candidate);
            }
        }
        anyhow::bail!(
            "could not allocate a conflict copy name for {}",
            path.display()
        )
    }

    /// Resolve an unresolved conflict by applying the chosen resolution and
    /// re-indexing the affected path so the result syncs on the next session.
    pub async fn resolve_conflict(
        &mut self,
        rel: &str,
        resolution: &crate::conflict::resolution::Resolution,
    ) -> Result<(), anyhow::Error> {
        use crate::conflict::resolution::ConflictResolver;
        use std::fs;

        let entry = self
            .store
            .get_unresolved_conflicts()?
            .into_iter()
            .find(|e| e.relative_path.to_string_lossy() == rel)
            .ok_or_else(|| anyhow::anyhow!("no unresolved conflict for {rel}"))?;

        let original = self.vault_path.join(&entry.relative_path);
        let copy = ConflictResolver::find_conflict_copy(&original);

        match resolution {
            crate::conflict::resolution::Resolution::KeepLocal => {
                if let Some(c) = &copy {
                    let _ = fs::remove_file(c);
                }
            }
            crate::conflict::resolution::Resolution::KeepRemote => {
                if let Some(c) = &copy {
                    if c.is_file() {
                        fs::copy(c, &original)?;
                        let _ = fs::remove_file(c);
                    }
                }
            }
            crate::conflict::resolution::Resolution::KeepBoth => {}
            crate::conflict::resolution::Resolution::OpenFile(_) => {}
        }

        self.store.mark_conflict_resolved(entry.id)?;

        let hash = hash_file_path(&original)?;
        let size = file_size(&original)?;
        let modified = modified_time(&original)?;
        self.revision_counter += 1;
        let mut state = FileState::new(
            entry.relative_path.clone(),
            hash,
            size,
            modified,
            self.revision_counter,
        );
        if let Some(existing) = self.store.get_file_state(rel)? {
            state.synced_hash = existing.synced_hash;
        }
        self.store.upsert_file_state(&state)?;
        self.save_revision_counter()?;
        info!("Resolved conflict on {rel} ({resolution:?})");
        Ok(())
    }

    /// Unresolved conflicts recorded in the store.
    pub fn conflicts(&self) -> Result<Vec<crate::conflict::record::ConflictEntry>, anyhow::Error> {
        Ok(self.store.get_unresolved_conflicts()?)
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
    /// created or modified on disk since the last scan. Unlike `initial_index`,
    /// this preserves the revision of unchanged files so the sync session does
    /// not see spurious conflicts.
    ///
    /// `detect_deletions` controls whether files that vanished from disk are
    /// tombstoned. Only the authoritative side (the laptop server) should pass
    /// `true`; clients must NOT auto-tombstone because their disk may be an
    /// incomplete replica (e.g. the phone's Obsidian app managing files), and a
    /// phantom tombstone would delete data on the authoritative vault.
    pub async fn refresh_index(&mut self, detect_deletions: bool) -> Result<(), anyhow::Error> {
        self.set_state(SyncStateMachine::Syncing);

        // Incremental scan: only re-hash files whose stat changed since last time.
        let existing = self.store.get_all_file_states()?;
        let existing_map: std::collections::HashMap<PathBuf, FileState> = existing
            .iter()
            .map(|s| (s.relative_path.clone(), s.clone()))
            .collect();
        let result = scan_vault_incremental(&self.vault_path, Some(&existing_map)).await?;
        let mut on_disk: std::collections::HashMap<PathBuf, FileState> =
            std::collections::HashMap::new();
        for file in &result.files {
            on_disk.insert(file.relative_path.clone(), file.clone());
        }

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
                        // Keep the agreement marker: this edit hasn't been
                        // synced yet, so the last-synced hash stays the old one.
                        updated.synced_hash = state.synced_hash;
                        self.store.upsert_file_state(&updated)?;
                    }
                }
                None if detect_deletions => {
                    // Removed from disk → tombstone
                    self.revision_counter += 1;
                    self.store
                        .delete_file_state(&state.relative_path.to_string_lossy())?;
                    self.store.upsert_tombstone(&Tombstone {
                        relative_path: state.relative_path.clone(),
                        revision: self.revision_counter,
                        deleted_at: crate::filesystem::now_millis(),
                    })?;
                }
                None => {
                    // Missing from disk but we don't trust this side's disk view:
                    // leave the state as-is so the authoritative peer decides.
                }
            }
        }

        // New files that were never indexed
        for disk in &result.files {
            if !existing
                .iter()
                .any(|s| s.relative_path == disk.relative_path)
            {
                self.revision_counter += 1;
                let mut state = disk.clone();
                state.revision = self.revision_counter;
                self.store.upsert_file_state(&state)?;
            }
        }

        self.save_revision_counter()?;
        self.set_state(SyncStateMachine::Idle);
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

    /// Like `build_manifest`, but only paths allowed by `scope` are included.
    /// Out-of-scope paths must never be advertised to a peer: omitting them
    /// without a scope would be read as "new files to push" by the peer.
    pub fn build_manifest_scoped(&self, scope: &Scope) -> Manifest {
        if scope.is_everything() {
            return self.build_manifest();
        }
        let files = self
            .store
            .get_all_file_states()
            .unwrap_or_default()
            .into_iter()
            .filter(|f| scope.allows(&f.relative_path))
            .collect();
        let tombstones = self
            .store
            .get_tombstones()
            .unwrap_or_default()
            .into_iter()
            .filter(|t| scope.allows(&t.relative_path))
            .collect();
        Manifest {
            device_id: self.device_id.clone(),
            files,
            tombstones,
            revision_counter: self.revision_counter,
        }
    }

    // ---- Sync scopes ----

    /// The vault-wide shared selection (synced to every approved device).
    pub fn shared_scope(&self) -> Vec<ScopeEntry> {
        self.store.get_shared_scope().unwrap_or_default()
    }

    /// Replace the shared selection.
    pub fn set_shared_scope(&self, entries: &[ScopeEntry]) -> Result<(), anyhow::Error> {
        Ok(self.store.set_shared_scope(entries)?)
    }

    /// Extra paths a specific approved device subscribes to.
    pub fn device_scope(&self, fingerprint: &str) -> Vec<ScopeEntry> {
        self.store.get_device_scope(fingerprint).unwrap_or_default()
    }

    /// Replace a device's optional scope (empty = shared selection only).
    pub fn set_device_scope(
        &self,
        fingerprint: &str,
        entries: &[ScopeEntry],
    ) -> Result<(), anyhow::Error> {
        Ok(self.store.set_device_scope(fingerprint, entries)?)
    }

    /// Vault-wide per-file exclusions (apply to every approved device).
    pub fn shared_exclusions(&self) -> Vec<String> {
        self.store.get_shared_exclusions().unwrap_or_default()
    }

    /// Replace the vault-wide exclusions.
    pub fn set_shared_exclusions(&self, paths: &[String]) -> Result<(), anyhow::Error> {
        Ok(self.store.set_shared_exclusions(paths)?)
    }

    /// Per-file exclusions for one approved device.
    pub fn device_exclusions(&self, fingerprint: &str) -> Vec<String> {
        self.store
            .get_device_exclusions(fingerprint)
            .unwrap_or_default()
    }

    /// Replace one device's exclusions.
    pub fn set_device_exclusions(
        &self,
        fingerprint: &str,
        paths: &[String],
    ) -> Result<(), anyhow::Error> {
        Ok(self.store.set_device_exclusions(fingerprint, paths)?)
    }

    /// Whether a device may only pull (never push/delete).
    pub fn device_read_only(&self, fingerprint: &str) -> bool {
        self.store
            .get_device_read_only(fingerprint)
            .unwrap_or(false)
    }

    pub fn set_device_read_only(
        &self,
        fingerprint: &str,
        read_only: bool,
    ) -> Result<(), anyhow::Error> {
        Ok(self.store.set_device_read_only(fingerprint, read_only)?)
    }

    /// What a peer may see and sync: the shared selection plus the device's own
    /// optional selection. Whole vault when neither is set. Exclusions from
    /// either side always apply.
    pub fn effective_scope(&self, fingerprint: &str) -> Scope {
        let shared = Scope {
            entries: self.shared_scope(),
            excludes: self.shared_exclusions(),
        };
        let device = Scope {
            entries: self.device_scope(fingerprint),
            excludes: self.device_exclusions(fingerprint),
        };
        shared.merge(&device)
    }

    /// This side's own scope (what this device wants to keep). Persisted in the
    /// vault config; empty = whole vault. Used by clients connecting to a
    /// server — the server decides what it sends, this decides what we keep.
    pub fn local_scope(&self) -> Scope {
        match self.store.get_config("local_scope").ok().flatten() {
            Some(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| Scope::everything()),
            None => Scope::everything(),
        }
    }

    pub fn set_local_scope(&self, scope: &Scope) -> Result<(), anyhow::Error> {
        self.store
            .set_config("local_scope", &serde_json::to_string(scope)?)?;
        Ok(())
    }

    /// Process a remote manifest and produce sync operations.
    pub fn reconcile(&mut self, remote: &Manifest) -> ManifestDiff {
        let local = self.build_manifest();
        let diff = compare_manifests(&local, remote);

        for (local_file, _remote_file) in &diff.conflicts {
            let rel_str = local_file.relative_path.to_string_lossy().to_string();
            // Update local state to Conflict
            if let Some(mut existing) = self.store.get_file_state(&rel_str).unwrap_or(None) {
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
            let state =
                FileState::new(path.to_owned(), hash, size, modified, self.revision_counter);
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

        // Reject only when we have local edits that haven't been synced yet —
        // the client's diff already decided this update is the newer side.
        if let Some(existing) = self.store.get_file_state(&rel_str)? {
            if existing.content_hash != *content_hash
                && existing.synced_hash.is_some()
                && existing.synced_hash != Some(existing.content_hash)
            {
                info!(
                    "Ignoring update for {} (local edits not yet synced)",
                    rel_str
                );
                return Ok(()); // Surface conflict to UI
            }
        }

        self.revision_counter += 1;
        let mut state = FileState::new(
            path.to_owned(),
            *content_hash,
            size,
            modified_at,
            self.revision_counter,
        );
        state.synced_hash = Some(*content_hash);
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
            deleted_at: crate::filesystem::now_millis(),
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
            .find(|f| f.relative_path == *"keep.md")
            .unwrap()
            .revision;

        // Modify one file, delete another, create a third
        std::fs::write(&change_path, b"changed content").unwrap();
        std::fs::remove_file(&keep_path).unwrap();
        std::fs::write(dir.path().join("new.md"), b"brand new").unwrap();

        engine.refresh_index(true).await.unwrap();

        let manifest = engine.build_manifest();
        assert_eq!(manifest.files.len(), 2);
        assert!(manifest.files.iter().any(|f| f.relative_path == *"new.md"));
        assert!(manifest
            .files
            .iter()
            .any(|f| f.relative_path == *"change.md"));
        // Deleted file became a tombstone
        assert!(manifest
            .tombstones
            .iter()
            .any(|t| t.relative_path == *"keep.md"));
        // Unchanged file keeps its revision → no spurious conflicts
        let keep_revision = manifest
            .files
            .iter()
            .find(|f| f.relative_path == *"change.md")
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
            .find(|f| f.relative_path == *"stable.md")
            .unwrap()
            .revision;

        engine.refresh_index(true).await.unwrap();
        let rev_after = engine
            .build_manifest()
            .files
            .iter()
            .find(|f| f.relative_path == *"stable.md")
            .unwrap()
            .revision;
        assert_eq!(rev_before, rev_after);
        assert_eq!(engine.file_count(), 1);
    }

    #[tokio::test]
    async fn test_refresh_index_without_deletion_detection_keeps_state() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ghost.md");
        std::fs::write(&path, b"content").unwrap();

        let mut engine = SyncEngine::new(dir.path().to_owned(), "test-device".into())
            .await
            .unwrap();
        engine.initial_index().await.unwrap();
        assert_eq!(engine.file_count(), 1);

        // File vanishes from disk (e.g. an app managing the folder).
        std::fs::remove_file(&path).unwrap();

        // Non-authoritative side must NOT tombstone phantom deletions.
        engine.refresh_index(false).await.unwrap();

        let manifest = engine.build_manifest();
        assert_eq!(manifest.files.len(), 1);
        assert!(manifest.tombstones.is_empty());
        assert_eq!(engine.file_count(), 1);
    }

    #[tokio::test]
    async fn test_refresh_index_with_deletion_detection_tombstones() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gone.md");
        std::fs::write(&path, b"content").unwrap();

        let mut engine = SyncEngine::new(dir.path().to_owned(), "test-device".into())
            .await
            .unwrap();
        engine.initial_index().await.unwrap();
        std::fs::remove_file(&path).unwrap();

        // Authoritative side tombstones the deletion.
        engine.refresh_index(true).await.unwrap();

        let manifest = engine.build_manifest();
        assert!(manifest.files.is_empty());
        assert_eq!(manifest.tombstones.len(), 1);
        assert_eq!(
            manifest.tombstones[0].relative_path,
            PathBuf::from("gone.md")
        );
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
        assert_eq!(
            manifest.files[0].relative_path,
            PathBuf::from("manifest.md")
        );
    }

    #[tokio::test]
    async fn test_scoped_manifest_filters_files_and_tombstones() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("notes")).unwrap();
        std::fs::write(dir.path().join("notes/a.md"), b"a").unwrap();
        std::fs::write(dir.path().join("notes/b.md"), b"b").unwrap();
        std::fs::write(dir.path().join("todo.md"), b"todo").unwrap();
        std::fs::write(dir.path().join("old.md"), b"old").unwrap();

        let mut engine = SyncEngine::new(dir.path().to_owned(), "test-desk".into())
            .await
            .unwrap();
        engine.initial_index().await.unwrap();
        // Tombstone old.md
        std::fs::remove_file(dir.path().join("old.md")).unwrap();
        engine.refresh_index(true).await.unwrap();

        let scope = Scope {
            entries: vec![ScopeEntry {
                kind: crate::sync::scope::ScopeKind::Folder,
                path: "notes".into(),
            }],
            excludes: Vec::new(),
        };
        let manifest = engine.build_manifest_scoped(&scope);
        assert_eq!(manifest.files.len(), 2);
        assert!(manifest
            .files
            .iter()
            .all(|f| f.relative_path.starts_with("notes")));
        // The old.md tombstone is out of scope and must not leak
        assert!(manifest.tombstones.is_empty());

        let everything = engine.build_manifest();
        assert_eq!(everything.files.len(), 3);
        assert_eq!(everything.tombstones.len(), 1);
    }

    #[tokio::test]
    async fn test_scoped_manifest_applies_exclusions() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("notes")).unwrap();
        std::fs::write(dir.path().join("notes/a.md"), b"a").unwrap();
        std::fs::write(dir.path().join("notes/secret.md"), b"secret").unwrap();
        std::fs::write(dir.path().join("todo.md"), b"todo").unwrap();

        let mut engine = SyncEngine::new(dir.path().to_owned(), "test-desk".into())
            .await
            .unwrap();
        engine.initial_index().await.unwrap();

        // Whole vault minus two files: no includes, exclusions only.
        let scope = Scope {
            entries: Vec::new(),
            excludes: vec!["notes/secret.md".into(), "todo.md".into()],
        };
        assert!(!scope.is_everything());
        let manifest = engine.build_manifest_scoped(&scope);
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(
            manifest.files[0].relative_path.to_string_lossy(),
            "notes/a.md"
        );

        // Folder include + file exclusion inside it.
        let scope = Scope {
            entries: vec![ScopeEntry {
                kind: crate::sync::scope::ScopeKind::Folder,
                path: "notes".into(),
            }],
            excludes: vec!["notes/secret.md".into()],
        };
        let manifest = engine.build_manifest_scoped(&scope);
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(
            manifest.files[0].relative_path.to_string_lossy(),
            "notes/a.md"
        );

        // The full index is untouched by scoping — exclusion is a pure filter.
        assert_eq!(engine.build_manifest().files.len(), 3);
    }

    #[tokio::test]
    async fn test_effective_scope_merges_shared_and_device() {
        let dir = TempDir::new().unwrap();
        let engine = SyncEngine::new(dir.path().to_owned(), "test-desk".into())
            .await
            .unwrap();
        engine
            .set_shared_scope(&[ScopeEntry {
                kind: crate::sync::scope::ScopeKind::Folder,
                path: "shared".into(),
            }])
            .unwrap();
        engine
            .set_device_scope(
                "fp-phone",
                &[ScopeEntry {
                    kind: crate::sync::scope::ScopeKind::File,
                    path: "secret.md".into(),
                }],
            )
            .unwrap();
        engine.set_device_read_only("fp-phone", true).unwrap();

        let scope = engine.effective_scope("fp-phone");
        assert_eq!(scope.entries.len(), 2);
        assert!(scope.allows(Path::new("shared/x.md")));
        assert!(scope.allows(Path::new("secret.md")));
        assert!(!scope.allows(Path::new("other.md")));
        assert!(engine.device_read_only("fp-phone"));
        // Another device with no optional scope sees only the shared selection
        let other = engine.effective_scope("fp-other");
        assert_eq!(other.entries.len(), 1);
        assert!(other.allows(Path::new("shared/x.md")));
        assert!(!other.allows(Path::new("secret.md")));
        assert!(!engine.device_read_only("fp-other"));
    }

    #[tokio::test]
    async fn test_effective_scope_unions_exclusions() {
        let dir = TempDir::new().unwrap();
        let engine = SyncEngine::new(dir.path().to_owned(), "test-desk".into())
            .await
            .unwrap();
        // No includes at all: whole vault, minus exclusions.
        engine
            .set_shared_exclusions(&["notes/secret.md".into()])
            .unwrap();
        engine
            .set_device_exclusions("fp-phone", &["todo.md".into()])
            .unwrap();

        let scope = engine.effective_scope("fp-phone");
        assert!(scope.allows(Path::new("anything/else.md")));
        assert!(!scope.allows(Path::new("notes/secret.md")));
        assert!(!scope.allows(Path::new("todo.md")));

        // Other devices only inherit the shared exclusion.
        let other = engine.effective_scope("fp-other");
        assert!(other.allows(Path::new("todo.md")));
        assert!(!other.allows(Path::new("notes/secret.md")));

        // Exclusion wins over an include from the other side.
        engine
            .set_shared_scope(&[ScopeEntry {
                kind: crate::sync::scope::ScopeKind::Folder,
                path: "notes".into(),
            }])
            .unwrap();
        let scope = engine.effective_scope("fp-phone");
        assert!(scope.allows(Path::new("notes/a.md")));
        assert!(!scope.allows(Path::new("notes/secret.md")));
    }

    #[tokio::test]
    async fn test_local_scope_roundtrip() {
        let dir = TempDir::new().unwrap();
        let engine = SyncEngine::new(dir.path().to_owned(), "test-mobile".into())
            .await
            .unwrap();
        assert!(engine.local_scope().is_everything());

        let scope = Scope {
            entries: vec![ScopeEntry {
                kind: crate::sync::scope::ScopeKind::Folder,
                path: "family".into(),
            }],
            excludes: Vec::new(),
        };
        engine.set_local_scope(&scope).unwrap();
        drop(engine);

        // A fresh engine on the same vault sees the stored scope
        let engine2 = SyncEngine::new(dir.path().to_owned(), "test-mobile".into())
            .await
            .unwrap();
        assert_eq!(engine2.local_scope(), scope);
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
}
