use std::io::Write;
use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::conflict::detector::{resolve_divergence, SideOutcome};
use crate::filesystem::atomic::AtomicWriter;
use crate::filesystem::io::hash_file_path;
use crate::filesystem::versioning::snapshot_before_overwrite;
use crate::filesystem::Blake3Hash;
use crate::index::state::{FileState, Manifest};
use crate::network::peer::PeerConnection;
use crate::network::protocol::{
    FileChunkPayload, FileRequestPayload, MessageType, ProtocolMessage, SyncOperationPayload,
};
use crate::sync::delta::SyncOperation;
use crate::sync::engine::SyncEngine;
use crate::sync::scope::Scope;

const CHUNK_SIZE: usize = 65536;

#[derive(Debug, Default)]
pub struct SyncReport {
    pub pulled_files: usize,
    pub pushed_files: usize,
    pub deleted_files: usize,
    pub conflicts: usize,
}

/// Client-driven sync session. The caller (phone) connects to a laptop server,
/// exchanges manifests, then pulls files it lacks and pushes files the server
/// lacks. `scope` is what THIS side wants to keep: out-of-scope paths are
/// never advertised, pulled, pushed, or deleted.
pub async fn run_client_session(
    engine: &mut SyncEngine,
    peer: &PeerConnection,
    scope: &Scope,
) -> Result<SyncReport, anyhow::Error> {
    info!("Starting client sync session");

    // 1. Exchange manifests
    let local = engine.build_manifest_scoped(scope);
    let local_json = serde_json::to_vec(&local)?;
    peer.send_message(&ProtocolMessage::new(MessageType::Manifest, 1, local_json))
        .await?;

    let msg = peer.receive_message().await?;
    if msg.message_type != MessageType::Manifest {
        return Err(anyhow::anyhow!("expected Manifest from server"));
    }
    let remote: Manifest = serde_json::from_slice(&msg.payload)?;
    info!(
        "Remote manifest: {} files, {} tombstones",
        remote.files.len(),
        remote.tombstones.len()
    );

    // 2. Compute direction of every path
    let local_map: std::collections::HashMap<&PathBuf, &FileState> =
        local.files.iter().map(|f| (&f.relative_path, f)).collect();
    let remote_map: std::collections::HashMap<&PathBuf, &FileState> =
        remote.files.iter().map(|f| (&f.relative_path, f)).collect();
    let remote_tombstones: std::collections::HashSet<&PathBuf> =
        remote.tombstones.iter().map(|t| &t.relative_path).collect();
    let local_tombstones: std::collections::HashSet<&PathBuf> =
        local.tombstones.iter().map(|t| &t.relative_path).collect();

    let mut report = SyncReport::default();
    let mut request_id = 1u64;

    // 3. Pull: files on server that we don't have (or differ, server newer)
    for (path, rf) in &remote_map {
        if !scope.allows(path) {
            // The server's manifest is already filtered by the device scope,
            // but a narrower local scope must never pull either.
            continue;
        }
        match local_map.get(path) {
            None => {
                if local_tombstones.contains(path) {
                    // Deleted locally before; push the deletion instead
                    continue;
                }
                pull_file(engine, peer, path, rf, &mut request_id).await?;
                report.pulled_files += 1;
            }
            Some(lf) => {
                if lf.content_hash != rf.content_hash {
                    match resolve_divergence(lf, rf) {
                        SideOutcome::Conflict => {
                            // Both edited within the window → real conflict.
                            // Keep the local version at the original path and
                            // pull the remote content into a conflict copy.
                            warn!("Conflict on {:?}", path);
                            if let Some(copy) =
                                engine.plan_conflict_copy(path, &rf.content_hash, true)?
                            {
                                let dest = engine.vault_path().join(&copy);
                                let size = pull_file_to(
                                    peer,
                                    path,
                                    rf,
                                    &dest,
                                    engine.vault_path(),
                                    &mut request_id,
                                )
                                .await?;
                                engine.record_remote_file(
                                    &copy,
                                    &rf.content_hash,
                                    size,
                                    rf.modified_at,
                                )?;
                            }
                            report.conflicts += 1;
                        }
                        SideOutcome::LocalWins => {
                            // Local newer → push
                            push_file(engine, peer, path, lf, &mut request_id).await?;
                            report.pushed_files += 1;
                        }
                        SideOutcome::RemoteWins => {
                            // Server newer → pull
                            pull_file(engine, peer, path, rf, &mut request_id).await?;
                            report.pulled_files += 1;
                        }
                    }
                }
            }
        }
    }

    // 4. Push: files only on local
    for (path, lf) in &local_map {
        if !remote_map.contains_key(path) && !remote_tombstones.contains(path) {
            push_file(engine, peer, path, lf, &mut request_id).await?;
            report.pushed_files += 1;
        }
    }

    // 5. Deletes: remote tombstones → delete locally
    for path in &remote_tombstones {
        if !scope.allows(path) {
            continue;
        }
        if local_map.contains_key(path) {
            debug!("Deleting local {:?} (tombstoned by server)", path);
            engine
                .apply_operation(&SyncOperation::Delete {
                    path: (*path).clone(),
                })
                .await?;
            report.deleted_files += 1;
        }
    }

    // 6. Push local tombstones → tell server to delete
    for path in &local_tombstones {
        if remote_map.contains_key(path) {
            let payload = SyncOperationPayload {
                operation_type: 2,
                relative_path: path.to_string_lossy().into_owned(),
                new_path: None,
                content_hash: None,
                size: 0,
                modified_at: 0,
            };
            peer.send_message(&ProtocolMessage::new(
                MessageType::SyncOperation,
                request_id,
                bincode::serialize(&payload)?,
            ))
            .await?;
            request_id += 1;
        }
    }

    // 7. Done
    peer.send_message(&ProtocolMessage::new(
        MessageType::Disconnect,
        request_id,
        vec![],
    ))
    .await?;

    info!(
        "Client sync complete: pulled={} pushed={} deleted={} conflicts={}",
        report.pulled_files, report.pushed_files, report.deleted_files, report.conflicts
    );
    Ok(report)
}

/// Server-side sync session. The caller (laptop) accepts a connection and runs
/// this. `scope` is what the connecting device may see and sync (its effective
/// scope); `read_only` additionally rejects pushes and deletes from the peer.
pub async fn run_server_session(
    engine: &mut SyncEngine,
    peer: &PeerConnection,
    scope: &Scope,
    read_only: bool,
) -> Result<SyncReport, anyhow::Error> {
    info!("Starting server sync session");

    // 1. Receive client manifest
    let msg = peer.receive_message().await?;
    if msg.message_type != MessageType::Manifest {
        return Err(anyhow::anyhow!("expected Manifest from client"));
    }
    let _client_manifest: Manifest = serde_json::from_slice(&msg.payload)?;

    // 2. Send our manifest — only what the peer's scope allows
    let local = engine.build_manifest_scoped(scope);
    peer.send_message(&ProtocolMessage::new(
        MessageType::Manifest,
        1,
        serde_json::to_vec(&local)?,
    ))
    .await?;

    let mut report = SyncReport::default();

    // 3. Handle requests until disconnect
    loop {
        let msg = match peer.receive_message().await {
            Ok(m) => m,
            Err(_) => break,
        };

        match msg.message_type {
            MessageType::FileRequest => {
                let req: FileRequestPayload = bincode::deserialize(&msg.payload)?;
                serve_file(engine, peer, &req.relative_path, &req.content_hash, scope).await?;
                report.pushed_files += 1;
            }
            MessageType::SyncOperation => {
                let op: SyncOperationPayload = bincode::deserialize(&msg.payload)?;
                let applied = handle_server_operation(engine, peer, &op, scope, read_only).await?;
                if applied {
                    match op.operation_type {
                        2 => report.deleted_files += 1,
                        _ => report.pulled_files += 1,
                    }
                }
            }
            MessageType::Disconnect => {
                info!("Client disconnected");
                break;
            }
            _ => {}
        }
    }

    info!(
        "Server sync complete: pulled={} pushed={} deleted={} conflicts={}",
        report.pulled_files, report.pushed_files, report.deleted_files, report.conflicts
    );
    Ok(report)
}

async fn pull_file(
    engine: &mut SyncEngine,
    peer: &PeerConnection,
    path: &Path,
    remote: &FileState,
    request_id: &mut u64,
) -> Result<(), anyhow::Error> {
    let dest = engine.vault_path().join(path);
    let data = pull_file_to(peer, path, remote, &dest, engine.vault_path(), request_id).await?;
    engine.record_remote_file(path, &remote.content_hash, data, remote.modified_at)?;
    debug!("Pulled {:?} ({} bytes)", path, data);
    Ok(())
}

/// Request `remote` content from the peer and write it to `dest`.
/// Returns the number of bytes received.
async fn pull_file_to(
    peer: &PeerConnection,
    path: &Path,
    remote: &FileState,
    dest: &Path,
    vault: &Path,
    request_id: &mut u64,
) -> Result<u64, anyhow::Error> {
    let req = FileRequestPayload {
        relative_path: path.to_string_lossy().into_owned(),
        content_hash: remote.content_hash,
        offset: 0,
    };
    peer.send_message(&ProtocolMessage::new(
        MessageType::FileRequest,
        *request_id,
        bincode::serialize(&req)?,
    ))
    .await?;
    *request_id += 1;

    let data = receive_file_data(peer, vault, dest).await?;

    let hash = hash_file_path(dest)?;
    if hash != remote.content_hash {
        warn!("Hash mismatch after pull for {:?}", path);
    }
    Ok(data)
}

async fn push_file(
    engine: &mut SyncEngine,
    peer: &PeerConnection,
    path: &Path,
    local: &FileState,
    request_id: &mut u64,
) -> Result<(), anyhow::Error> {
    let payload = SyncOperationPayload {
        operation_type: 0,
        relative_path: path.to_string_lossy().into_owned(),
        new_path: None,
        content_hash: Some(local.content_hash),
        size: local.size,
        modified_at: local.modified_at,
    };
    peer.send_message(&ProtocolMessage::new(
        MessageType::SyncOperation,
        *request_id,
        bincode::serialize(&payload)?,
    ))
    .await?;
    *request_id += 1;

    let src = engine.vault_path().join(path);
    send_file_data(peer, path, &src).await?;

    // Mark as synced locally
    engine.mark_synced(path)?;

    debug!("Pushed {:?}", path);
    Ok(())
}

async fn serve_file(
    engine: &mut SyncEngine,
    peer: &PeerConnection,
    relative_path: &str,
    _expected_hash: &Blake3Hash,
    scope: &Scope,
) -> Result<(), anyhow::Error> {
    let path = Path::new(relative_path);
    if !scope.allows(path) {
        // A scoped client can only request files the server already advertised
        // in its (filtered) manifest — anything else is a bug or a hostile
        // peer. Dropping the whole session is the safe response: we neither
        // serve out-of-scope content nor continue speaking to a misbehaving
        // peer.
        warn!(
            "Rejecting out-of-scope file request and dropping session: {}",
            relative_path
        );
        return Err(anyhow::anyhow!(
            "out-of-scope file request: {relative_path}"
        ));
    }
    let src = engine.vault_path().join(path);
    send_file_data(peer, path, &src).await?;
    Ok(())
}

/// Handle a client sync operation. Returns whether the operation was applied;
/// rejected operations (out of scope or read-only device) are skipped while
/// their file data is drained so the TCP framing stays aligned.
async fn handle_server_operation(
    engine: &mut SyncEngine,
    peer: &PeerConnection,
    op: &SyncOperationPayload,
    scope: &Scope,
    read_only: bool,
) -> Result<bool, anyhow::Error> {
    let path = PathBuf::from(&op.relative_path);
    let allowed = scope.allows(&path) && !read_only;
    match op.operation_type {
        0 | 1 => {
            if !allowed {
                // The client already pushed the content behind this op — drain
                // the chunks so the protocol framing stays in sync, then skip.
                warn!(
                    "Skipping {} push of {} (out of scope or read-only device)",
                    if read_only { "read-only" } else { "out-of-scope" },
                    op.relative_path
                );
                drain_file_data(peer).await?;
                return Ok(false);
            }
            // create/update: receive file data
            let original_dest = engine.vault_path().join(&path);
            // Guard against clobbering unsynced local edits: write the remote
            // content to a conflict copy instead when this would overwrite them.
            let dest = match op.content_hash {
                Some(expected) => engine
                    .plan_conflict_copy(&path, &expected, false)?
                    .map(|copy| engine.vault_path().join(copy))
                    .unwrap_or(original_dest.clone()),
                None => original_dest.clone(),
            };
            let data = receive_file_data(peer, engine.vault_path(), &dest).await?;
            let hash = hash_file_path(&dest)?;
            let hash = match op.content_hash {
                Some(expected) => {
                    if expected != hash {
                        warn!("Hash mismatch receiving {:?}", path);
                    }
                    expected
                }
                None => hash,
            };
            if dest != original_dest {
                // Remote content landed in a conflict copy: index the copy as
                // a new file (it syncs to peers) and leave the original alone.
                if let Ok(copy_rel) = dest.strip_prefix(engine.vault_path()) {
                    engine.record_remote_file(copy_rel, &hash, data, op.modified_at)?;
                }
            } else {
                engine.record_remote_file(&path, &hash, data, op.modified_at)?;
            }
            // Ack
            peer.send_message(&ProtocolMessage::new(MessageType::OperationAck, 0, vec![]))
                .await?;
            debug!("Server received {:?}", path);
        }
        2 => {
            if !allowed {
                warn!(
                    "Skipping {} delete of {}",
                    if read_only { "read-only" } else { "out-of-scope" },
                    op.relative_path
                );
                return Ok(false);
            }
            engine
                .apply_operation(&SyncOperation::Delete { path })
                .await?;
            peer.send_message(&ProtocolMessage::new(MessageType::OperationAck, 0, vec![]))
                .await?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Consume the file chunks the client sends after a push operation without
/// writing anything — used when an operation is rejected so the protocol
/// framing (and the TCP stream) stays aligned for the rest of the session.
async fn drain_file_data(peer: &PeerConnection) -> Result<(), anyhow::Error> {
    loop {
        let msg = peer.receive_message().await?;
        if msg.message_type != MessageType::FileChunk {
            return Err(anyhow::anyhow!(
                "expected FileChunk, got {:?}",
                msg.message_type
            ));
        }
        let chunk: FileChunkPayload = bincode::deserialize(&msg.payload)?;
        if chunk.is_last {
            break;
        }
    }
    Ok(())
}

async fn send_file_data(
    peer: &PeerConnection,
    relative_path: &Path,
    src: &Path,
) -> Result<(), anyhow::Error> {
    let data = tokio::fs::read(src).await?;
    let total = data.len();
    let mut offset = 0usize;
    let mut request_id = 0u64;

    loop {
        let end = (offset + CHUNK_SIZE).min(total);
        let is_last = end == total;
        let chunk = FileChunkPayload {
            relative_path: relative_path.to_string_lossy().into_owned(),
            offset: offset as u64,
            data: data[offset..end].to_vec(),
            is_last,
        };
        peer.send_message(&ProtocolMessage::new(
            MessageType::FileChunk,
            request_id,
            bincode::serialize(&chunk)?,
        ))
        .await?;
        request_id += 1;
        offset = end;
        if is_last {
            break;
        }
    }
    Ok(())
}

async fn receive_file_data(
    peer: &PeerConnection,
    vault: &Path,
    dest: &Path,
) -> Result<u64, anyhow::Error> {
    let parent = dest.parent().unwrap_or(Path::new(""));
    tokio::fs::create_dir_all(parent).await?;

    // Preserve the previous content as a version snapshot before overwriting.
    if let Ok(rel) = dest.strip_prefix(vault) {
        let _ = snapshot_before_overwrite(vault, rel);
    }

    let mut writer = AtomicWriter::new(dest.to_owned())?;
    let mut total: u64 = 0;

    loop {
        let msg = peer.receive_message().await?;
        if msg.message_type != MessageType::FileChunk {
            return Err(anyhow::anyhow!(
                "expected FileChunk, got {:?}",
                msg.message_type
            ));
        }
        let chunk: FileChunkPayload = bincode::deserialize(&msg.payload)?;
        writer.writer().write_all(&chunk.data)?;
        total += chunk.data.len() as u64;
        if chunk.is_last {
            break;
        }
    }

    writer.commit()?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_chunk_roundtrip() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src.bin");
        let dest = dir.path().join("dest.bin");
        let data = vec![7u8; CHUNK_SIZE * 2 + 10];
        tokio::fs::write(&src, &data).await.unwrap();

        // Use two connected TCP sockets via a fake loopback
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            stream
        });

        let client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let server_stream = server.await.unwrap();

        // Build minimal peer connections (no handshake)
        let client_peer = PeerConnection {
            device_id: "c".into(),
            device_name: "client".into(),
            address: addr,
            stream: std::sync::Arc::new(tokio::sync::Mutex::new(client)),
        };
        let server_peer = PeerConnection {
            device_id: "s".into(),
            device_name: "server".into(),
            address: addr,
            stream: std::sync::Arc::new(tokio::sync::Mutex::new(server_stream)),
        };

        let sender = tokio::spawn(async move {
            send_file_data(&client_peer, Path::new("f.bin"), &src)
                .await
                .unwrap();
        });
        let total = receive_file_data(&server_peer, dir.path(), &dest)
            .await
            .unwrap();
        sender.await.unwrap();

        assert_eq!(total as usize, data.len());
        let written = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(written, data);
    }

    /// Engine over a temp vault pre-seeded with files and fully indexed.
    async fn seeded_engine(
        dir: &Path,
        id: &str,
        files: &[(&str, &[u8])],
    ) -> SyncEngine {
        for (rel, data) in files {
            let p = dir.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(p, data).unwrap();
        }
        let mut engine = SyncEngine::new(dir.to_owned(), id.into())
            .await
            .unwrap();
        engine.initial_index().await.unwrap();
        engine
    }

    /// A connected client/server PeerConnection pair over TCP loopback.
    async fn peer_pair() -> (PeerConnection, PeerConnection) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accept = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            stream
        });
        let client_stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let server_stream = accept.await.unwrap();
        (
            PeerConnection {
                device_id: "c".into(),
                device_name: "client".into(),
                address: addr,
                stream: std::sync::Arc::new(tokio::sync::Mutex::new(client_stream)),
            },
            PeerConnection {
                device_id: "s".into(),
                device_name: "server".into(),
                address: addr,
                stream: std::sync::Arc::new(tokio::sync::Mutex::new(server_stream)),
            },
        )
    }

    #[tokio::test]
    async fn test_client_exclusion_never_pulls_excluded_file() {
        let sdir = TempDir::new().unwrap();
        let cdir = TempDir::new().unwrap();
        let mut server_engine = seeded_engine(
            sdir.path(),
            "srv",
            &[
                ("notes/a.md", b"a" as &[u8]),
                ("notes/secret.md", b"secret"),
                ("todo.md", b"todo"),
            ],
        )
        .await;
        let mut client_engine = seeded_engine(cdir.path(), "cli", &[]).await;

        // Client keeps everything except one file (exclusions-only scope).
        let client_scope = Scope {
            entries: Vec::new(),
            excludes: vec!["notes/secret.md".into()],
        };
        let server_scope = Scope::everything();

        let (client_peer, server_peer) = peer_pair().await;
        let srv = tokio::spawn(async move {
            run_server_session(&mut server_engine, &server_peer, &server_scope, false)
                .await
                .unwrap()
        });
        let report = run_client_session(&mut client_engine, &client_peer, &client_scope)
            .await
            .unwrap();
        srv.await.unwrap();

        assert_eq!(report.pulled_files, 2); // notes/a.md + todo.md, not secret
        assert!(cdir.path().join("notes/a.md").exists());
        assert!(cdir.path().join("todo.md").exists());
        assert!(!cdir.path().join("notes/secret.md").exists());
    }

    #[tokio::test]
    async fn test_server_exclusion_rejects_push_and_session_survives() {
        let sdir = TempDir::new().unwrap();
        let cdir = TempDir::new().unwrap();
        // The excluded path exists ONLY on the client.
        let mut server_engine =
            seeded_engine(sdir.path(), "srv", &[("notes/a.md", b"a" as &[u8])]).await;
        let mut client_engine = seeded_engine(
            cdir.path(),
            "cli",
            &[
                ("notes/a.md", b"a" as &[u8]),
                ("notes/secret.md", b"client secret"),
            ],
        )
        .await;

        // Server hides notes/secret.md from every device.
        let server_scope = Scope {
            entries: Vec::new(),
            excludes: vec!["notes/secret.md".into()],
        };
        let client_scope = Scope::everything();

        let (client_peer, server_peer) = peer_pair().await;
        let srv = tokio::spawn(async move {
            run_server_session(&mut server_engine, &server_peer, &server_scope, false)
                .await
                .unwrap()
        });
        let report = run_client_session(&mut client_engine, &client_peer, &client_scope)
            .await
            .unwrap();
        let srv_report = srv.await.unwrap();

        // The push was attempted (server manifest lacked the file)…
        assert_eq!(report.pushed_files, 1);
        // …but the server refused it and stayed alive (framing intact).
        assert!(!sdir.path().join("notes/secret.md").exists());
        assert_eq!(srv_report.pulled_files, 0);
    }

    #[tokio::test]
    async fn test_client_exclusion_ignores_server_tombstone() {
        let sdir = TempDir::new().unwrap();
        let cdir = TempDir::new().unwrap();
        let mut server_engine = seeded_engine(
            sdir.path(),
            "srv",
            &[("notes/a.md", b"a" as &[u8]), ("todo.md", b"todo")],
        )
        .await;
        // Authoritative server deletes todo.md → tombstone.
        std::fs::remove_file(sdir.path().join("todo.md")).unwrap();
        server_engine.refresh_index(true).await.unwrap();

        // Client still has todo.md (with different content) but excludes it.
        let mut client_engine = seeded_engine(
            cdir.path(),
            "cli",
            &[
                ("notes/a.md", b"a" as &[u8]),
                ("todo.md", b"todo-old"),
            ],
        )
        .await;
        let client_scope = Scope {
            entries: Vec::new(),
            excludes: vec!["todo.md".into()],
        };

        let (client_peer, server_peer) = peer_pair().await;
        let srv = tokio::spawn(async move {
            run_server_session(&mut server_engine, &server_peer, &Scope::everything(), false)
                .await
                .unwrap()
        });
        let report = run_client_session(&mut client_engine, &client_peer, &client_scope)
            .await
            .unwrap();
        srv.await.unwrap();

        // Exclusion beats the server tombstone: the local copy survives.
        assert!(cdir.path().join("todo.md").exists());
        assert_eq!(
            std::fs::read(cdir.path().join("todo.md")).unwrap(),
            b"todo-old"
        );
        assert_eq!(report.deleted_files, 0);
    }
}
