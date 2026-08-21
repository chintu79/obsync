use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Path as AxumPath, State},
    http::Method,
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        Html,
    },
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
use obsync_core::sync::scope::ScopeEntry;
use serde::Deserialize;
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
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
    /// Recent activity events (in-memory ring buffer)
    activity: Arc<Mutex<VecDeque<ActivityEntry>>>,
    /// Last successful handshake per device fingerprint, for online/offline status
    last_seen: Arc<Mutex<HashMap<String, Instant>>>,
    /// SSE broadcast channel — every recorded activity is echoed here so the
    /// dashboard can refresh instantly instead of polling.
    events: tokio::sync::broadcast::Sender<String>,
    /// Cached `/api/status` payload, rebuilt periodically + on every mutation so
    /// the dashboard never has to lock the engine mutex per request.
    status_cache: Arc<Mutex<serde_json::Value>>,
}

#[derive(Clone)]
struct ActivityEntry {
    ts: i64,
    kind: &'static str,
    detail: String,
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

const ACTIVITY_CAP: usize = 100;

async fn record_activity(state: &Arc<AppState>, kind: &'static str, detail: String) {
    let mut act = state.activity.lock().await;
    act.push_back(ActivityEntry {
        ts: now_millis(),
        kind,
        detail,
    });
    while act.len() > ACTIVITY_CAP {
        act.pop_front();
    }
    let _ = state.events.send(kind.to_string());
}

/// Build the full `/api/status` payload. Takes the engine mutex, so this is the
/// expensive path — it runs from `refresh_status_cache` only.
async fn build_status(state: &Arc<AppState>) -> serde_json::Value {
    let guard = state.engine.lock().await;
    let vault_path = state.vault_path.lock().await.clone();
    let vault = guard.as_ref().map(|e| {
        serde_json::json!({
            "file_count": e.file_count(),
            "state": format!("{:?}", e.state()),
            "path": vault_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        })
    });

    let device = get_device_info_inner(state).await;

    let conflicts = guard
        .as_ref()
        .and_then(|e| e.conflicts().ok())
        .map(|c| c.len())
        .unwrap_or(0);

    let pending_count = state.pending.lock().await.len();

    let (total_devices, online_devices) = {
        let approved = state.approved.lock().await;
        let last_seen = state.last_seen.lock().await;
        let total = approved.len();
        let online = approved
            .keys()
            .filter(|fp| {
                last_seen
                    .get(*fp)
                    .map(|t| t.elapsed() < Duration::from_secs(120))
                    .unwrap_or(false)
            })
            .count();
        (total, online)
    };

    let last_sync_at = {
        let act = state.activity.lock().await;
        act.iter().rev().find(|a| a.kind == "sync").map(|a| a.ts)
    };


    serde_json::json!({
        "vault": vault,
        "device": device,
        "state": vault.as_ref().and_then(|v| v.get("state").and_then(|s| s.as_str().map(String::from))).unwrap_or("no_vault".into()),
        "conflicts": conflicts,
        "pending": pending_count,
        "devices": { "total": total_devices, "online": online_devices },
        "last_sync_at": last_sync_at,
    })
}

/// Recompute and store the status snapshot. Called after every mutation and on
/// a periodic tick so `/api/status` never blocks on the engine mutex.
async fn refresh_status_cache(state: &Arc<AppState>) {
    *state.status_cache.lock().await = build_status(state).await;
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

#[derive(Deserialize)]
struct SetScopeRequest {
    #[serde(default)]
    entries: Vec<ScopeEntry>,
    /// Per-file exclusions. Absent = leave the stored list untouched;
    /// present (even empty) = replace it.
    #[serde(default)]
    excludes: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct SetDeviceScopeRequest {
    #[serde(default)]
    entries: Vec<ScopeEntry>,
    #[serde(default)]
    excludes: Option<Vec<String>>,
    #[serde(default)]
    read_only: Option<bool>,
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

/// Start the HTTP dashboard server (port 42021) and the P2P sync TCP server
/// (port 42042). Runs forever; returns only if the HTTP server shuts down.
pub async fn run_server() -> anyhow::Result<()> {
    // Tolerate an already-initialized global subscriber (e.g. when embedded in
    // the Tauri desktop app).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .try_init();

    let approved = load_approved()?;
    let state = Arc::new(AppState {
        engine: Arc::new(Mutex::new(None)),
        vault_path: Arc::new(Mutex::new(load_vault_path())),
        pending: Arc::new(Mutex::new(HashMap::new())),
        approved: Arc::new(Mutex::new(approved)),
        activity: Arc::new(Mutex::new(VecDeque::new())),
        last_seen: Arc::new(Mutex::new(HashMap::new())),
        events: tokio::sync::broadcast::channel::<String>(256).0,
        status_cache: Arc::new(Mutex::new(serde_json::Value::Null)),
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
    refresh_status_cache(&state).await;

    // Rebuild the status snapshot periodically so online/offline decay and
    // engine state transitions stay fresh without any client polling it.
    {
        let bg_state = state.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(3));
            loop {
                tick.tick().await;
                refresh_status_cache(&bg_state).await;
            }
        });
    }

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/status", get(handle_status))
        .route("/api/select-vault", post(handle_select_vault))
        .route("/api/sync-now", post(handle_sync_now))
        .route("/api/reveal-vault", post(handle_reveal_vault))
        .route("/api/browse-vault", post(handle_browse_vault))
        .route("/api/files", get(handle_files))
        .route("/api/devices", get(handle_devices))
        .route("/api/scopes", get(handle_scopes))
        .route("/api/scopes/shared", post(handle_set_shared_scope))
        .route(
            "/api/devices/:fingerprint/scope",
            post(handle_set_device_scope),
        )
        .route("/api/activity", get(handle_activity))
        .route("/api/events", get(handle_events))
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

async fn handle_sync_connection(mut stream: TcpStream, state: Arc<AppState>) -> anyhow::Result<()> {
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
            info!(
                "Sync handshake from {} ({})",
                hello.device_name, hello.device_id
            );
            let ack = ProtocolMessage::new(MessageType::HelloAck, msg.request_id, vec![]);
            let ack_bytes = ack.to_bytes()?;
            stream.write_u32_le(ack_bytes.len() as u32).await?;
            stream.write_all(&ack_bytes).await?;
            (
                hello.device_id,
                hello.device_name,
                hello.public_key_fingerprint,
            )
        }
        _ => return Err(anyhow::anyhow!("expected Hello")),
    };

    // Require approval for unknown devices
    let approved = {
        let guard = state.approved.lock().await;
        guard.contains_key(&peer_fingerprint)
    };
    let allowed = if approved {
        true
    } else {
        await_approval(&state, &peer_id, &peer_name, &peer_fingerprint).await
    };
    if !allowed {
        info!(
            "Connection from {} ({}) rejected; closing",
            peer_name, peer_id
        );
        return Ok(());
    }

    let peer = PeerConnection {
        device_id: peer_id,
        device_name: peer_name,
        address: stream.peer_addr()?,
        stream: std::sync::Arc::new(tokio::sync::Mutex::new(stream)),
    };

    // Track online/last-seen and surface the handshake in the activity feed.
    state
        .last_seen
        .lock()
        .await
        .insert(peer_fingerprint.clone(), Instant::now());
    record_activity(&state, "device_connected", peer.device_name.clone()).await;

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
    let scope = engine.effective_scope(&peer_fingerprint);
    let read_only = engine.device_read_only(&peer_fingerprint);
    let report: SyncReport = run_server_session(&mut engine, &peer, &scope, read_only).await?;
    record_activity(
        &state,
        "sync",
        format!(
            "{} · {} pushed, {} pulled, {} conflicts",
            peer.device_name, report.pushed_files, report.pulled_files, report.conflicts
        ),
    )
    .await;
    refresh_status_cache(&state).await;

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
    warn!(
        "Device {} ({}) requesting sync access; waiting for approval",
        device_name, device_id
    );

    let result = tokio::time::timeout(APPROVAL_TIMEOUT, rx).await;
    let allowed = matches!(result, Ok(Ok(true)));

    let mut pending = state.pending.lock().await;
    pending.remove(device_id);
    drop(pending);

    if allowed {
        let mut approved = state.approved.lock().await;
        approved.insert(fingerprint.to_string(), device_name.to_string());
        save_approved(&approved);
        drop(approved);
        record_activity(state, "device_approved", device_name.to_string()).await;
    } else {
        record_activity(state, "device_rejected", device_name.to_string()).await;
    }
    refresh_status_cache(state).await;
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

async fn handle_status(state: State<Arc<AppState>>) -> Json<serde_json::Value> {
    // Served from cache — the engine mutex is never touched per request.
    Json(state.status_cache.lock().await.clone())
}

async fn handle_sync_now(state: State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mut guard = state.engine.lock().await;
    let Some(engine) = guard.as_mut() else {
        return Json(serde_json::json!({ "error": "no vault selected" }));
    };
    match engine.refresh_index(true).await {
        Ok(_) => {
            let state_str = format!("{:?}", engine.state());
            let file_count = engine.file_count();
            drop(guard);
            record_activity(&state, "sync", "Manual rescan".into()).await;
            refresh_status_cache(&state).await;
            Json(serde_json::json!({
                "ok": true,
                "file_count": file_count,
                "state": state_str,
            }))
        }
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

async fn handle_reveal_vault(state: State<Arc<AppState>>) -> Json<serde_json::Value> {
    let vault = {
        let guard = state.vault_path.lock().await;
        guard.clone()
    };
    let Some(vault) = vault else {
        return Json(serde_json::json!({ "error": "no vault selected" }));
    };
    match std::process::Command::new("xdg-open").arg(&vault).spawn() {
        Ok(_) => Json(serde_json::json!({ "ok": true, "path": vault.to_string_lossy() })),
        Err(e) => Json(serde_json::json!({ "error": format!("could not open folder: {e}") })),
    }
}

/// Native folder picker: pops a zenity directory dialog on the server's display
/// and returns the chosen path. Runs off-thread so the dialog never blocks the
/// async runtime; falls back to manual path entry if no display/picker exists.
async fn handle_browse_vault() -> Json<serde_json::Value> {
    let picked = tokio::task::spawn_blocking(|| {
        let out = std::process::Command::new("zenity")
            .args([
                "--file-selection",
                "--directory",
                "--title=Select vault folder",
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if path.is_empty() {
            None
        } else {
            Some(path)
        }
    })
    .await;

    match picked {
        Ok(Some(path)) => Json(serde_json::json!({ "ok": true, "path": path })),
        Ok(None) => Json(serde_json::json!({ "cancelled": true })),
        Err(e) => Json(serde_json::json!({ "error": format!("folder picker unavailable: {e}") })),
    }
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
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    record_activity(state, "vault_selected", name).await;
    refresh_status_cache(state).await;
    Ok(file_count)
}

async fn handle_files(state: State<Arc<AppState>>) -> Json<Vec<serde_json::Value>> {
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

async fn handle_devices(state: State<Arc<AppState>>) -> Json<Vec<serde_json::Value>> {
    let self_info = get_device_info_inner(&state).await;
    let approved = state.approved.lock().await.clone();
    let last_seen = state.last_seen.lock().await.clone();

    let scopes: HashMap<String, (Vec<ScopeEntry>, bool)> = {
        let guard = state.engine.lock().await;
        match guard.as_ref() {
            Some(e) => approved
                .keys()
                .map(|fp| {
                    (
                        fp.clone(),
                        (e.device_scope(fp), e.device_read_only(fp)),
                    )
                })
                .collect(),
            None => HashMap::new(),
        }
    };

    let mut out: Vec<serde_json::Value> = Vec::new();
    if let Some(info) = self_info.as_ref() {
        out.push(serde_json::json!({
            "name": info["device_name"],
            "fingerprint": info["fingerprint"],
            "is_self": true,
            "online": true,
            "last_seen_ms": null,
            "scope": [],
            "read_only": false,
        }));
    }
    for (fp, name) in approved {
        let is_self = self_info
            .as_ref()
            .map(|i| i["fingerprint"].as_str() == Some(fp.as_str()))
            .unwrap_or(false);
        if is_self {
            continue;
        }
        let seen = last_seen.get(&fp).copied();
        let online = seen
            .map(|t| t.elapsed() < Duration::from_secs(120))
            .unwrap_or(false);
        let last_seen_ms = seen.map(|t| t.elapsed().as_millis() as i64);
        let (scope, read_only) = scopes.get(&fp).cloned().unwrap_or_default();
        out.push(serde_json::json!({
            "name": name,
            "fingerprint": fp,
            "is_self": false,
            "online": online,
            "last_seen_ms": last_seen_ms,
            "scope": scope,
            "read_only": read_only,
        }));
    }
    Json(out)
}

/// Full scope state for the dashboard: the shared selection plus every
/// approved device's optional scope and read-only flag.
async fn handle_scopes(state: State<Arc<AppState>>) -> Json<serde_json::Value> {
    let approved = state.approved.lock().await.clone();
    let guard = state.engine.lock().await;
    let Some(engine) = guard.as_ref() else {
        return Json(serde_json::json!({ "error": "no vault selected" }));
    };
    let mut devices = serde_json::Map::new();
    for fp in approved.keys() {
        devices.insert(
            fp.clone(),
            serde_json::json!({
                "entries": engine.device_scope(fp),
                "excludes": engine.device_exclusions(fp),
                "read_only": engine.device_read_only(fp),
            }),
        );
    }
    Json(serde_json::json!({
        "shared": engine.shared_scope(),
        "shared_excludes": engine.shared_exclusions(),
        "devices": serde_json::Value::Object(devices),
    }))
}

/// Replace the vault-wide shared selection (synced to every approved device).
async fn handle_set_shared_scope(
    state: State<Arc<AppState>>,
    Json(req): Json<SetScopeRequest>,
) -> Json<serde_json::Value> {
    let guard = state.engine.lock().await;
    let Some(engine) = guard.as_ref() else {
        return Json(serde_json::json!({ "error": "no vault selected" }));
    };
    if let Err(e) = engine.set_shared_scope(&req.entries) {
        return Json(serde_json::json!({ "error": e.to_string() }));
    }
    if let Some(excludes) = &req.excludes {
        if let Err(e) = engine.set_shared_exclusions(excludes) {
            return Json(serde_json::json!({ "error": e.to_string() }));
        }
    }
    drop(guard);
    record_activity(
        &state,
        "scope_updated",
        format!(
            "shared selection: {} entries, {} excluded",
            req.entries.len(),
            req.excludes.as_ref().map(|e| e.len()).unwrap_or(0)
        ),
    )
    .await;
    refresh_status_cache(&state).await;
    Json(serde_json::json!({ "ok": true, "count": req.entries.len() }))
}

/// Replace one approved device's optional scope (+ read-only flag).
async fn handle_set_device_scope(
    state: State<Arc<AppState>>,
    AxumPath(fingerprint): AxumPath<String>,
    Json(req): Json<SetDeviceScopeRequest>,
) -> Json<serde_json::Value> {
    let approved = {
        let guard = state.approved.lock().await;
        guard.contains_key(&fingerprint)
    };
    if !approved {
        return Json(serde_json::json!({ "error": "unknown device" }));
    }
    let guard = state.engine.lock().await;
    let Some(engine) = guard.as_ref() else {
        return Json(serde_json::json!({ "error": "no vault selected" }));
    };
    if let Err(e) = engine.set_device_scope(&fingerprint, &req.entries) {
        return Json(serde_json::json!({ "error": e.to_string() }));
    }
    if let Some(excludes) = &req.excludes {
        if let Err(e) = engine.set_device_exclusions(&fingerprint, excludes) {
            return Json(serde_json::json!({ "error": e.to_string() }));
        }
    }
    if let Some(ro) = req.read_only {
        if let Err(e) = engine.set_device_read_only(&fingerprint, ro) {
            return Json(serde_json::json!({ "error": e.to_string() }));
        }
    }
    drop(guard);
    record_activity(
        &state,
        "scope_updated",
        format!("device {fingerprint}: {} entries", req.entries.len()),
    )
    .await;
    refresh_status_cache(&state).await;
    Json(serde_json::json!({ "ok": true, "fingerprint": fingerprint }))
}

async fn handle_activity(state: State<Arc<AppState>>) -> Json<Vec<serde_json::Value>> {
    let act = state.activity.lock().await;
    Json(
        act.iter()
            .map(|a| serde_json::json!({ "ts": a.ts, "kind": a.kind, "detail": a.detail }))
            .collect(),
    )
}

/// Server-Sent Events stream. Every recorded activity event is pushed to all
/// connected dashboards immediately so the UI never has to poll for updates.
/// `broadcast::Receiver` has no `poll_recv`, so a per-client task forwards
/// messages into an mpsc channel that we can poll as a `Stream`.
struct ActivityStream {
    rx: tokio::sync::mpsc::Receiver<String>,
}

impl futures_core::Stream for ActivityStream {
    type Item = Result<SseEvent, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(msg)) => {
                let ev = SseEvent::default().event("activity").data(msg);
                Poll::Ready(Some(Ok::<_, Infallible>(ev)))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

async fn handle_events(
    state: State<Arc<AppState>>,
) -> Sse<impl futures_core::Stream<Item = Result<SseEvent, Infallible>>> {
    let broadcast_rx = state.events.subscribe();
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    tokio::spawn(async move {
        let mut broadcast_rx = broadcast_rx;
        while let Ok(msg) = broadcast_rx.recv().await {
            if tx.send(msg).await.is_err() {
                break;
            }
        }
    });
    Sse::new(ActivityStream { rx }).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn handle_conflicts(state: State<Arc<AppState>>) -> Json<Vec<serde_json::Value>> {
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
                            "local_hash": e.local_hash.map(hex::encode),
                            "remote_hash": e.remote_hash.map(hex::encode),
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
    let result = match req.resolution.as_str() {
        "KeepLocal" => obsync_core::conflict::resolution::Resolution::KeepLocal,
        "KeepRemote" => obsync_core::conflict::resolution::Resolution::KeepRemote,
        "KeepBoth" => obsync_core::conflict::resolution::Resolution::KeepBoth,
        other => {
            return Json(serde_json::json!({ "error": format!("unknown resolution {other}") }));
        }
    };
    let outcome = engine.resolve_conflict(&req.path, &result).await;
    drop(guard);
    match outcome {
        Ok(()) => {
            record_activity(&state, "conflict_resolved", req.path.clone()).await;
            refresh_status_cache(&state).await;
            Json(serde_json::json!({ "ok": true, "path": req.path }))
        }
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

async fn handle_versions(state: State<Arc<AppState>>) -> Json<Vec<serde_json::Value>> {
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
    if let Err(e) =
        obsync_core::filesystem::versioning::restore_snapshot(&vault, &rel, req.timestamp)
    {
        return Json(serde_json::json!({ "error": e.to_string() }));
    }
    // Re-index so the restored content is treated as a fresh local edit.
    let mut guard = state.engine.lock().await;
    if let Some(engine) = guard.as_mut() {
        let _ = engine.refresh_index(true).await;
    }
    drop(guard);
    record_activity(&state, "restore", req.path.clone()).await;
    refresh_status_cache(&state).await;
    Json(serde_json::json!({ "ok": true, "path": req.path }))
}

async fn handle_identity(state: State<Arc<AppState>>) -> Json<serde_json::Value> {
    match get_device_info_inner(&state).await {
        Some(info) => Json(info),
        None => Json(serde_json::json!({ "error": "no identity" })),
    }
}

/// Best-effort local LAN IP via a UDP socket to a public resolver — this
/// selects the interface that would reach the internet (the phone hotspot).
fn local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip().to_string())
}

/// Return an SVG QR code encoding the pairing payload (host, port, device identity).
async fn handle_pairing_qr(state: State<Arc<AppState>>) -> Json<serde_json::Value> {
    let Some(info) = get_device_info_inner(&state).await else {
        return Json(serde_json::json!({ "error": "no identity" }));
    };
    let device_id = info["device_id"].as_str().unwrap_or("").to_string();
    let device_name = info["device_name"].as_str().unwrap_or("obsync").to_string();
    let fingerprint = info["fingerprint"].as_str().unwrap_or("").to_string();

    let host = local_ip().unwrap_or_else(|| "127.0.0.1".to_string());

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
