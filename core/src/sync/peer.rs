use std::io::Write;
use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::filesystem::atomic::AtomicWriter;
use crate::filesystem::io::hash_file_path;
use crate::filesystem::Blake3Hash;
use crate::index::state::{FileState, Manifest};
use crate::network::peer::PeerConnection;
use crate::network::protocol::{
    FileChunkPayload, FileRequestPayload, MessageType, ProtocolMessage, SyncOperationPayload,
};
use crate::sync::delta::SyncOperation;
use crate::sync::engine::SyncEngine;

const CHUNK_SIZE: usize = 65536;

#[derive(Debug, Default)]
pub struct SyncReport {
    pub pulled_files: usize,
    pub pushed_files: usize,
    pub deleted_files: usize,
    pub conflicts: usize,
}

/// Client-driven sync session. The caller (phone) connects to a laptop server,
/// exchanges manifests, then pulls files it lacks and pushes files the server lacks.
pub async fn run_client_session(
    engine: &mut SyncEngine,
    peer: &PeerConnection,
) -> Result<SyncReport, anyhow::Error> {
    info!("Starting client sync session");

    // 1. Exchange manifests
    let local = engine.build_manifest();
    let local_json = serde_json::to_vec(&local)?;
    peer.send_message(&ProtocolMessage::new(
        MessageType::Manifest,
        1,
        local_json,
    ))
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
        match local_map.get(path) {
            None => {
                if local_tombstones.contains(path) {
                    // Deleted locally before; push the deletion instead
                    continue;
                }
                pull_file(engine, peer, *path, rf, &mut request_id).await?;
                report.pulled_files += 1;
            }
            Some(lf) => {
                if lf.content_hash != rf.content_hash {
                    if lf.revision > 0 && rf.revision > 0 {
                        // Both changed since last sync → conflict
                        warn!("Conflict on {:?}", path);
                        engine.mark_conflict(path)?;
                        report.conflicts += 1;
                    } else if lf.modified_at >= rf.modified_at {
                        // Local newer → push
                        push_file(engine, peer, *path, lf, &mut request_id).await?;
                        report.pushed_files += 1;
                    } else {
                        // Server newer → pull
                        pull_file(engine, peer, *path, rf, &mut request_id).await?;
                        report.pulled_files += 1;
                    }
                }
            }
        }
    }

    // 4. Push: files only on local
    for (path, lf) in &local_map {
        if !remote_map.contains_key(path) && !remote_tombstones.contains(path) {
            push_file(engine, peer, *path, lf, &mut request_id).await?;
            report.pushed_files += 1;
        }
    }

    // 5. Deletes: remote tombstones → delete locally
    for path in &remote_tombstones {
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
    peer.send_message(&ProtocolMessage::new(MessageType::Disconnect, request_id, vec![]))
        .await?;

    info!(
        "Client sync complete: pulled={} pushed={} deleted={} conflicts={}",
        report.pulled_files, report.pushed_files, report.deleted_files, report.conflicts
    );
    Ok(report)
}

/// Server-side sync session. The caller (laptop) accepts a connection and runs this.
pub async fn run_server_session(
    engine: &mut SyncEngine,
    peer: &PeerConnection,
) -> Result<SyncReport, anyhow::Error> {
    info!("Starting server sync session");

    // 1. Receive client manifest
    let msg = peer.receive_message().await?;
    if msg.message_type != MessageType::Manifest {
        return Err(anyhow::anyhow!("expected Manifest from client"));
    }
    let _client_manifest: Manifest = serde_json::from_slice(&msg.payload)?;

    // 2. Send our manifest
    let local = engine.build_manifest();
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
                serve_file(engine, peer, &req.relative_path, &req.content_hash).await?;
                report.pushed_files += 1;
            }
            MessageType::SyncOperation => {
                let op: SyncOperationPayload = bincode::deserialize(&msg.payload)?;
                handle_server_operation(engine, peer, &op).await?;
                match op.operation_type {
                    2 => report.deleted_files += 1,
                    _ => report.pulled_files += 1,
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

    let dest = engine.vault_path().join(path);
    let data = receive_file_data(peer, &dest).await?;

    let hash = hash_file_path(&dest)?;
    if hash != remote.content_hash {
        warn!("Hash mismatch after pull for {:?}", path);
    }

    engine.record_remote_file(path, &hash, data, remote.modified_at)?;
    debug!("Pulled {:?} ({} bytes)", path, data);
    Ok(())
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
) -> Result<(), anyhow::Error> {
    let path = Path::new(relative_path);
    let src = engine.vault_path().join(path);
    send_file_data(peer, path, &src).await?;
    Ok(())
}

async fn handle_server_operation(
    engine: &mut SyncEngine,
    peer: &PeerConnection,
    op: &SyncOperationPayload,
) -> Result<(), anyhow::Error> {
    let path = PathBuf::from(&op.relative_path);
    match op.operation_type {
        0 | 1 => {
            // create/update: receive file data
            let dest = engine.vault_path().join(&path);
            let data = receive_file_data(peer, &dest).await?;
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
            engine.record_remote_file(&path, &hash, data, op.modified_at)?;
            // Ack
            peer.send_message(&ProtocolMessage::new(MessageType::OperationAck, 0, vec![]))
                .await?;
            debug!("Server received {:?}", path);
        }
        2 => {
            engine
                .apply_operation(&SyncOperation::Delete { path })
                .await?;
            peer.send_message(&ProtocolMessage::new(MessageType::OperationAck, 0, vec![]))
                .await?;
        }
        _ => {}
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
    dest: &Path,
) -> Result<u64, anyhow::Error> {
    let parent = dest.parent().unwrap_or(Path::new(""));
    tokio::fs::create_dir_all(parent).await?;

    let mut writer = AtomicWriter::new(dest.to_owned())?;
    let mut total: u64 = 0;

    loop {
        let msg = peer.receive_message().await?;
        if msg.message_type != MessageType::FileChunk {
            return Err(anyhow::anyhow!("expected FileChunk, got {:?}", msg.message_type));
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
            send_file_data(&client_peer, Path::new("f.bin"), &src).await.unwrap();
        });
        let total = receive_file_data(&server_peer, &dest).await.unwrap();
        sender.await.unwrap();

        assert_eq!(total as usize, data.len());
        let written = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(written, data);
    }
}
