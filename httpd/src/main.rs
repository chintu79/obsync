use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{Path as AxumPath, State},
    http::Method,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use obsync_core::network::peer::PeerConnection;
use obsync_core::network::protocol::{
    HelloPayload, MessageType, ProtocolMessage, PROTOCOL_VERSION,
};
use obsync_core::security::identity::DeviceIdentity;
use obsync_core::storage::config::ConfigStore;
use obsync_core::storage::db;
use obsync_core::sync::engine::SyncEngine;
use obsync_core::sync::peer::{run_server_session, SyncReport};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex};
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

struct AppState {
    engine: Arc<Mutex<Option<SyncEngine>>>,
    vault_path: Arc<Mutex<Option<PathBuf>>>,
    /// Pending permission requests keyed by device_id
    pending: Arc<Mutex<HashMap<String, PendingPeer>>>,
    /// Approved devices keyed by public-key fingerprint -> device name
    approved: Arc<Mutex<HashMap<String, String>>>,
}

struct PendingPeer {
    device_id: String,
    device_name: String,
    fingerprint: String,
    since: Instant,
    tx: oneshot::Sender<bool>,
}

#[derive(Deserialize)]
struct SelectVaultRequest {
    path: String,
}

#[derive(Deserialize)]
struct ResolveConflictRequest {
    path: String,
    resolution: String,
}

#[derive(Deserialize)]
struct RestoreSnapshotRequest {
    path: String,
    timestamp: i64,
}

fn vault_file() -> PathBuf {
    dirs_home().join(".obsync-server-vault.json")
}

fn load_vault_path() -> Option<PathBuf> {
    let path = vault_file();
    if !path.exists() {
        return None;
    }
    let data = std::fs::read(&path).ok()?;
    let parsed: serde_json::Value = serde_json::from_slice(&data).ok()?;
    let p = parsed.get("path")?.as_str()?;
    let pb = PathBuf::from(p);
    pb.is_dir().then_some(pb)
}

fn save_vault_path(vault: &Path) {
    let data = serde_json::json!({ "path": vault.to_string_lossy() });
    if let Ok(bytes) = serde_json::to_vec(&data) {
        let _ = std::fs::write(vault_file(), bytes);
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let approved = load_approved()?;
    let state = Arc::new(AppState {
        engine: Arc::new(Mutex::new(None)),
        vault_path: Arc::new(Mutex::new(load_vault_path())),
        pending: Arc::new(Mutex::new(HashMap::new())),
        approved: Arc::new(Mutex::new(approved)),
    });

    // Re-select the persisted vault so the daemon restarts with its vault.
    {
        let vault = state.vault_path.lock().await.clone();
        if let Some(vault) = vault {
            match select_vault_impl(&state, vault).await {
                Ok(_) => info!("Restored vault selection from disk"),
                Err(e) => warn!("Could not restore vault from disk: {e}"),
            }
        }
    }

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/status", get(handle_status))
        .route("/api/select-vault", post(handle_select_vault))
        .route("/api/files", get(handle_files))
        .route("/api/devices", get(handle_devices))
        .route("/api/conflicts", get(handle_conflicts))
        .route("/api/conflicts/resolve", post(handle_resolve_conflict))
        .route("/api/versions", get(handle_versions))
        .route("/api/restore", post(handle_restore))
        .route("/api/identity", get(handle_identity))
        .route("/api/pairing-qr", get(handle_pairing_qr))
        .route("/api/pending", get(handle_pending))
        .route("/api/approve/:device_id", post(handle_approve))
        .route("/api/reject/:device_id", post(handle_reject))
        .route("/", get(handle_ui))
        .layer(cors)
        .with_state(state.clone());

    let addr = "0.0.0.0:42021";
    info!("Obsync HTTP daemon starting on http://{}", addr);
    info!("Open http://localhost:42021 in your browser");

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Spawn the P2P sync TCP server on a separate port
    let sync_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = run_sync_tcp_server(sync_state).await {
            eprintln!("sync server error: {e}");
        }
    });

    axum::serve(listener, app).await?;

    Ok(())
}

const SYNC_PORT: u16 = 42042;

async fn run_sync_tcp_server(state: Arc<AppState>) -> anyhow::Result<()> {
    let addr = format!("0.0.0.0:{SYNC_PORT}");
    let listener = TcpListener::bind(&addr).await?;
    info!("P2P sync server listening on {addr}");

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_sync_connection(stream, state).await {
                eprintln!("sync connection error: {e}");
            }
        });
    }
}

async fn handle_sync_connection(
    mut stream: TcpStream,
    state: Arc<AppState>,
) -> anyhow::Result<()> {
    // Handshake: read Hello, respond HelloAck
    let len = stream.read_u32_le().await?;
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    let msg = ProtocolMessage::from_bytes(&buf)?;
    if msg.version != PROTOCOL_VERSION {
        return Err(anyhow::anyhow!("protocol version mismatch"));
    }

    let (peer_id, peer_name, peer_fingerprint) = match msg.message_type {
        MessageType::Hello => {
            let hello: HelloPayload = bincode::deserialize(&msg.payload)?;
            info!("Sync handshake from {} ({})", hello.device_name, hello.device_id);
            let ack = ProtocolMessage::new(MessageType::HelloAck, msg.request_id, vec![]);
            let ack_bytes = ack.to_bytes()?;
            stream.write_u32_le(ack_bytes.len() as u32).await?;
            stream.write_all(&ack_bytes).await?;
            (hello.device_id, hello.device_name, hello.public_key_fingerprint)
        }
        _ => return Err(anyhow::anyhow!("expected Hello")),
    };

    // Require approval for unknown devices
    let approved = {
        let guard = state.approved.lock().await;
        guard.contains_key(&peer_fingerprint)
    };
    if !approved {
        if !await_approval(&state, &peer_id, &peer_name, &peer_fingerprint).await {
            info!("Connection from {} ({}) rejected; closing", peer_name, peer_id);
            return Ok(());
        }
    }

    let peer = PeerConnection {
        device_id: peer_id,
        device_name: peer_name,
        address: stream.peer_addr()?,
        stream: std::sync::Arc::new(tokio::sync::Mutex::new(stream)),
    };

    // Run a server session against the currently selected vault, if any
    let vault_path = {
        let guard = state.vault_path.lock().await;
        guard.clone()
    };

    let Some(vault_path) = vault_path else {
        info!("No vault selected; closing sync connection");
        return Ok(());
    };

    let device_id = {
        let config_path = vault_path.join(".obsync").join("config.bin");
        let store = ConfigStore::new(config_path);
        DeviceIdentity::load(&store)
            .ok()
            .flatten()
            .map(|id| id.device_id)
            .unwrap_or_else(|| "desktop".into())
    };

    let mut engine = SyncEngine::new(vault_path, device_id).await?;
    engine.refresh_index(true).await?;
    let _report: SyncReport = run_server_session(&mut engine, &peer).await?;

    Ok(())
}

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

/// Wait for the user to approve/reject a connecting device. Returns true if approved.
async fn await_approval(
    state: &Arc<AppState>,
    device_id: &str,
    device_name: &str,
    fingerprint: &str,
) -> bool {
    let (tx, rx) = oneshot::channel();
    {
        let mut pending = state.pending.lock().await;
        pending.insert(
            device_id.to_string(),
            PendingPeer {
                device_id: device_id.to_string(),
                device_name: device_name.to_string(),
                fingerprint: fingerprint.to_string(),
                since: Instant::now(),
                tx,
            },
        );
    }
    warn!("Device {} ({}) requesting sync access; waiting for approval", device_name, device_id);

    let result = tokio::time::timeout(APPROVAL_TIMEOUT, rx).await;
    let allowed = matches!(result, Ok(Ok(true)));

    let mut pending = state.pending.lock().await;
    pending.remove(device_id);

    if allowed {
        let mut approved = state.approved.lock().await;
        approved.insert(fingerprint.to_string(), device_name.to_string());
        save_approved(&approved);
    }
    allowed
}

fn approved_file() -> PathBuf {
    dirs_home().join(".obsync-approved.json")
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn load_approved() -> anyhow::Result<HashMap<String, String>> {
    let path = approved_file();
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let data = std::fs::read(&path)?;
    Ok(serde_json::from_slice(&data).unwrap_or_default())
}

fn save_approved(approved: &HashMap<String, String>) {
    let path = approved_file();
    if let Ok(data) = serde_json::to_vec(approved) {
        let _ = std::fs::write(path, data);
    }
}

async fn handle_ui() -> Html<&'static str> {
    Html(include_str!("webui.html"))
}

async fn handle_status(
    state: State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let guard = state.engine.lock().await;
    let vault = guard.as_ref().map(|e| {
        serde_json::json!({
            "file_count": e.file_count(),
            "state": format!("{:?}", e.state()),
        })
    });

    let device = get_device_info_inner(&state).await;

    Json(serde_json::json!({
        "vault": vault,
        "device": device,
        "state": vault.as_ref().and_then(|v| v.get("state").and_then(|s| s.as_str().map(String::from))).unwrap_or("no_vault".into()),
    }))
}

async fn handle_select_vault(
    state: State<Arc<AppState>>,
    Json(req): Json<SelectVaultRequest>,
) -> Json<serde_json::Value> {
    let path = PathBuf::from(&req.path);
    if !path.exists() {
        return Json(serde_json::json!({ "error": "path not found" }));
    }
    match select_vault_impl(&state, path).await {
        Ok(file_count) => Json(serde_json::json!({
            "ok": true,
            "file_count": file_count,
            "state": "Idle"
        })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

async fn select_vault_impl(state: &Arc<AppState>, path: PathBuf) -> anyhow::Result<u64> {
    if let Err(e) = db::ensure_db_directory(&path) {
        return Err(anyhow::anyhow!(e.to_string()));
    }

    let device_id = {
        let config_dir = path.join(".obsync");
        let config_path = config_dir.join("config.bin");
        let store = ConfigStore::new(config_path);
        let identity = match DeviceIdentity::load(&store) {
            Ok(Some(id)) => id,
            _ => {
                let id = DeviceIdentity::generate("obsync".into());
                let _ = id.save(&store);
                id
            }
        };
        identity.device_id.clone()
    };

    let mut engine = SyncEngine::new(path.clone(), device_id).await?;
    engine.initial_index().await?;
    let file_count = engine.file_count();
    *state.vault_path.lock().await = Some(path.clone());
    *state.engine.lock().await = Some(engine);
    save_vault_path(&path);
    Ok(file_count)
}

async fn handle_files(
    state: State<Arc<AppState>>,
) -> Json<Vec<serde_json::Value>> {
    let guard = state.engine.lock().await;
    if let Some(engine) = guard.as_ref() {
        let manifest = engine.build_manifest();
        Json(
            manifest
                .files
                .into_iter()
                .map(|f| {
                    serde_json::json!({
                        "path": f.relative_path.to_string_lossy(),
                        "size": f.size,
                        "modified_at": f.modified_at,
                        "sync_state": format!("{:?}", f.sync_state),
                    })
                })
                .collect(),
        )
    } else {
        Json(vec![])
    }
}

async fn handle_devices() -> Json<Vec<serde_json::Value>> {
    Json(vec![])
}

async fn handle_conflicts(
    state: State<Arc<AppState>>,
) -> Json<Vec<serde_json::Value>> {
    let guard = state.engine.lock().await;
    if let Some(engine) = guard.as_ref() {
        match engine.conflicts() {
            Ok(entries) => Json(
                entries
                    .into_iter()
                    .map(|e| {
                        serde_json::json!({
                            "id": e.id,
                            "path": e.relative_path.to_string_lossy(),
                            "local_hash": e.local_hash.map(|h| hex::encode(h)),
                            "remote_hash": e.remote_hash.map(|h| hex::encode(h)),
                            "detected_at": e.detected_at,
                        })
                    })
                    .collect(),
            ),
            Err(_) => Json(vec![]),
        }
    } else {
        Json(vec![])
    }
}

async fn handle_resolve_conflict(
    state: State<Arc<AppState>>,
    Json(req): Json<ResolveConflictRequest>,
) -> Json<serde_json::Value> {
    let mut guard = state.engine.lock().await;
    let Some(engine) = guard.as_mut() else {
        return Json(serde_json::json!({ "error": "no vault selected" }));
    };
    let resolution = match req.resolution.as_str() {
        "KeepLocal" => obsync_core::conflict::resolution::Resolution::KeepLocal,
        "KeepRemote" => obsync_core::conflict::resolution::Resolution::KeepRemote,
        "KeepBoth" => obsync_core::conflict::resolution::Resolution::KeepBoth,
        other => {
            return Json(serde_json::json!({ "error": format!("unknown resolution {other}") }));
        }
    };
    match engine.resolve_conflict(&req.path, &resolution).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "path": req.path })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

async fn handle_versions(
    state: State<Arc<AppState>>,
) -> Json<Vec<serde_json::Value>> {
    let vault = {
        let guard = state.vault_path.lock().await;
        guard.clone()
    };
    let Some(vault) = vault else {
        return Json(vec![]);
    };
    match obsync_core::filesystem::versioning::list_all_snapshots(&vault) {
        Ok(snaps) => Json(
            snaps
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "path": s.relative_path,
                        "timestamp": s.timestamp,
                        "size": s.size,
                    })
                })
                .collect(),
        ),
        Err(_) => Json(vec![]),
    }
}

async fn handle_restore(
    state: State<Arc<AppState>>,
    Json(req): Json<RestoreSnapshotRequest>,
) -> Json<serde_json::Value> {
    let vault = {
        let guard = state.vault_path.lock().await;
        guard.clone()
    };
    let Some(vault) = vault else {
        return Json(serde_json::json!({ "error": "no vault selected" }));
    };
    let rel = PathBuf::from(&req.path);
    let full = vault.join(&rel);
    let canon_vault = vault.canonicalize().unwrap_or(vault.clone());
    let canon_full = full.canonicalize().unwrap_or(full.clone());
    if !canon_full.starts_with(&canon_vault) {
        return Json(serde_json::json!({ "error": "path escapes vault" }));
    }
    if let Err(e) = obsync_core::filesystem::versioning::restore_snapshot(
        &vault,
        &rel,
        req.timestamp,
    ) {
        return Json(serde_json::json!({ "error": e.to_string() }));
    }
    // Re-index so the restored content is treated as a fresh local edit.
    let mut guard = state.engine.lock().await;
    if let Some(engine) = guard.as_mut() {
        let _ = engine.refresh_index(true).await;
    }
    Json(serde_json::json!({ "ok": true, "path": req.path }))
}

async fn handle_identity(
    state: State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    match get_device_info_inner(&state).await {
        Some(info) => Json(info),
        None => Json(serde_json::json!({ "error": "no identity" })),
    }
}

/// Return an SVG QR code encoding the pairing payload (host, port, device identity).
async fn handle_pairing_qr(
    state: State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let Some(info) = get_device_info_inner(&state).await else {
        return Json(serde_json::json!({ "error": "no identity" }));
    };
    let device_id = info["device_id"].as_str().unwrap_or("").to_string();
    let device_name = info["device_name"].as_str().unwrap_or("obsync").to_string();
    let fingerprint = info["fingerprint"].as_str().unwrap_or("").to_string();

    let host = local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string());

    let payload = serde_json::json!({
        "type": "obsync",
        "version": 1,
        "host": host,
        "port": SYNC_PORT,
        "device_id": device_id,
        "device_name": device_name,
        "public_key_fingerprint": fingerprint,
    });

    let payload_str = payload.to_string();
    let svg = match qrcode::QrCode::new(payload_str.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::svg::Color>()
            .module_dimensions(8, 8)
            .build(),
        Err(_) => String::new(),
    };

    Json(serde_json::json!({
        "payload": payload,
        "svg": svg,
    }))
}

async fn handle_pending(state: State<Arc<AppState>>) -> Json<Vec<serde_json::Value>> {
    let pending = state.pending.lock().await;
    Json(
        pending
            .values()
            .map(|p| {
                serde_json::json!({
                    "device_id": p.device_id,
                    "device_name": p.device_name,
                    "fingerprint": p.fingerprint,
                    "since_ms": p.since.elapsed().as_millis(),
                })
            })
            .collect(),
    )
}

async fn handle_approve(
    state: State<Arc<AppState>>,
    AxumPath(device_id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let mut pending = state.pending.lock().await;
    match pending.remove(&device_id) {
        Some(p) => {
            let _ = p.tx.send(true);
            Json(serde_json::json!({ "ok": true, "device_id": device_id }))
        }
        None => Json(serde_json::json!({ "error": "no pending request" })),
    }
}

async fn handle_reject(
    state: State<Arc<AppState>>,
    AxumPath(device_id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let mut pending = state.pending.lock().await;
    match pending.remove(&device_id) {
        Some(p) => {
            let _ = p.tx.send(false);
            Json(serde_json::json!({ "ok": true, "device_id": device_id }))
        }
        None => Json(serde_json::json!({ "error": "no pending request" })),
    }
}

async fn get_device_info_inner(state: &Arc<AppState>) -> Option<serde_json::Value> {
    let vault_path = state.vault_path.lock().await.clone()?;
    let config_path = vault_path.join(".obsync").join("config.bin");
    let store = ConfigStore::new(config_path);
    match DeviceIdentity::load(&store) {
        Ok(Some(id)) => Some(serde_json::json!({
            "device_id": id.device_id,
            "device_name": id.device_name,
            "fingerprint": id.fingerprint(),
        })),
        _ => None,
    }
}
