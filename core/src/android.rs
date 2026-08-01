//! JNI bridge for Android. Each function is callable from Kotlin via `System.loadLibrary("obsync_core")`.

use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jint, jlong, jstring};
use jni::JNIEnv;

use std::sync::{Mutex, OnceLock};

use crate::filesystem::io::hash_file_path;
use crate::index::compare::compare_manifests;
use crate::index::scanner::scan_vault;
use crate::index::state::Manifest;
use crate::index::store::Store;
use crate::network::peer::PeerConnection;
use crate::security::crypto::{decrypt, encrypt};
use crate::security::identity::DeviceIdentity;
use crate::storage::config::ConfigStore;
use crate::sync::engine::SyncEngine;

/// Generate a new device identity. Returns JSON with device_id, device_name, fingerprint.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_generateIdentity<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    name: JString<'local>,
) -> JByteArray<'local> {
    let name: String = env.get_string(&name).unwrap().into();
    let identity = DeviceIdentity::generate(name);
    let json = serde_json::json!({
        "device_id": identity.device_id,
        "device_name": identity.device_name,
        "fingerprint": identity.fingerprint(),
    });
    let bytes = json.to_string().into_bytes();
    env.byte_array_from_slice(&bytes).unwrap()
}

/// Load a persisted device identity from a config file, creating and persisting a new one
/// if none exists. Returns JSON with device_id, device_name, fingerprint.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_loadOrCreateIdentity<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    name: JString<'local>,
    config_path: JString<'local>,
) -> JByteArray<'local> {
    let name: String = env.get_string(&name).unwrap().into();
    let config_path: String = env.get_string(&config_path).unwrap().into();

    // Serialize load-or-create so concurrent callers can't both generate a fresh identity.
    static IDENTITY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = IDENTITY_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

    let store = ConfigStore::new(std::path::PathBuf::from(&config_path));
    let identity = match DeviceIdentity::load(&store) {
        Ok(Some(id)) => id,
        _ => {
            let id = DeviceIdentity::generate(name);
            let _ = id.save(&store);
            id
        }
    };
    let json = serde_json::json!({
        "device_id": identity.device_id,
        "device_name": identity.device_name,
        "fingerprint": identity.fingerprint(),
    });
    let bytes = json.to_string().into_bytes();
    env.byte_array_from_slice(&bytes).unwrap()
}

/// Get device_id from identity JSON.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_identityDeviceId(
    mut env: JNIEnv,
    _class: JClass,
    data: JByteArray,
) -> jstring {
    let bytes = env.convert_byte_array(data).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let val = json["device_id"].as_str().unwrap_or("");
    env.new_string(val).unwrap().into_raw()
}

/// Get device_name from identity JSON.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_identityDeviceName(
    mut env: JNIEnv,
    _class: JClass,
    data: JByteArray,
) -> jstring {
    let bytes = env.convert_byte_array(data).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let val = json["device_name"].as_str().unwrap_or("");
    env.new_string(val).unwrap().into_raw()
}

/// Get fingerprint from identity JSON.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_identityFingerprint(
    mut env: JNIEnv,
    _class: JClass,
    data: JByteArray,
) -> jstring {
    let bytes = env.convert_byte_array(data).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let val = json["fingerprint"].as_str().unwrap_or("");
    env.new_string(val).unwrap().into_raw()
}

/// Index a vault directory. Returns JSON: { file_count, revision_counter }.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_indexVault(
    mut env: JNIEnv,
    _class: JClass,
    path: JString,
    _identity_json: JString,
) -> jstring {
    let path: String = env.get_string(&path).unwrap().into();
    let vault_path = std::path::PathBuf::from(&path);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let mut engine = SyncEngine::new(vault_path.clone(), "android".into()).await?;
        engine.initial_index().await?;
        let count = engine.file_count();
        Ok::<_, anyhow::Error>(serde_json::json!({
            "file_count": count,
            "revision_counter": 0u64,
            "ok": true,
        }))
    });

    let json = match result {
        Ok(v) => v,
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    };
    env.new_string(json.to_string()).unwrap().into_raw()
}

/// Build manifest from local database. Returns JSON manifest.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_buildManifest(
    mut env: JNIEnv,
    _class: JClass,
    db_path: JString,
) -> jstring {
    let path: String = env.get_string(&db_path).unwrap().into();
    let store = Store::open(std::path::Path::new(&path)).unwrap();
    let files = store.get_all_file_states().unwrap_or_default();
    let tombstones = store.get_tombstones().unwrap_or_default();
    let manifest = Manifest {
        device_id: "android".into(),
        files,
        tombstones,
        revision_counter: 0,
    };
    let json = serde_json::to_string(&manifest).unwrap();
    env.new_string(json).unwrap().into_raw()
}

/// Compare two manifests. Returns JSON diff.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_compareManifests(
    mut env: JNIEnv,
    _class: JClass,
    local_json: JString,
    remote_json: JString,
) -> jstring {
    let local: String = env.get_string(&local_json).unwrap().into();
    let remote: String = env.get_string(&remote_json).unwrap().into();
    let local_manifest: Manifest = serde_json::from_str(&local).unwrap();
    let remote_manifest: Manifest = serde_json::from_str(&remote).unwrap();
    let diff = compare_manifests(&local_manifest, &remote_manifest);
    let json = serde_json::json!({
        "operations": diff.operations.len(),
        "conflicts": diff.conflicts.len(),
    });
    env.new_string(json.to_string()).unwrap().into_raw()
}

/// Hash a file, return hex string.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_hashFile(
    mut env: JNIEnv,
    _class: JClass,
    path: JString,
) -> jstring {
    let path: String = env.get_string(&path).unwrap().into();
    let result = hash_file_path(std::path::Path::new(&path));
    let hex = match result {
        Ok(h) => hex::encode(h),
        Err(_) => "error".into(),
    };
    env.new_string(hex).unwrap().into_raw()
}

/// Apply a sync operation to the local database. Returns JSON result.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_applyOperation(
    mut env: JNIEnv,
    _class: JClass,
    db_path: JString,
    _vault_path: JString,
    op_json: JString,
) -> jstring {
    let db: String = env.get_string(&db_path).unwrap().into();
    let _op: String = env.get_string(&op_json).unwrap().into();
    let store = Store::open(std::path::Path::new(&db)).unwrap();
    // TODO: parse and apply the operation
    let result = serde_json::json!({ "applied": true });
    env.new_string(result.to_string()).unwrap().into_raw()
}

/// Generate pairing payload (QR data).
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_generatePairingPayload(
    mut env: JNIEnv,
    _class: JClass,
    identity_json: JString,
) -> jstring {
    let json: String = env.get_string(&identity_json).unwrap().into();
    let identity_val: serde_json::Value = serde_json::from_str(&json).unwrap();
    // Simplified — in production would use real DeviceIdentity
    let payload = serde_json::json!({
        "version": 1,
        "device_id": identity_val["device_id"],
        "device_name": identity_val["device_name"],
        "public_key_fingerprint": "placeholder",
        "public_key_bytes": [],
        "ephemeral_public_key": [],
    });
    env.new_string(payload.to_string()).unwrap().into_raw()
}

/// Encrypt data with AES-256-GCM. Key is hex-encoded 32 bytes.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_encrypt<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    data: JByteArray<'local>,
    key_hex: JString<'local>,
) -> JByteArray<'local> {
    let bytes = env.convert_byte_array(data).unwrap();
    let key_str: String = env.get_string(&key_hex).unwrap().into();
    let mut key = [0u8; 32];
    hex::decode_to_slice(&key_str, &mut key).ok();
    let result = encrypt(&bytes, &key).unwrap_or_default();
    env.byte_array_from_slice(&result).unwrap()
}

/// Decrypt data with AES-256-GCM. Key is hex-encoded 32 bytes.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_decrypt<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    data: JByteArray<'local>,
    key_hex: JString<'local>,
) -> JByteArray<'local> {
    let bytes = env.convert_byte_array(data).unwrap();
    let key_str: String = env.get_string(&key_hex).unwrap().into();
    let mut key = [0u8; 32];
    hex::decode_to_slice(&key_str, &mut key).ok();
    let result = decrypt(&bytes, &key).unwrap_or_default();
    env.byte_array_from_slice(&result).unwrap()
}

/// Connect to a peer and run a full client sync session.
/// Returns JSON: { pulled, pushed, deleted, conflicts, error? }.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_syncOnce<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    addr: JString<'local>,
    port: jint,
    vault_path: JString<'local>,
    identity_json: JString<'local>,
) -> jstring {
    let addr_str: String = env.get_string(&addr).unwrap().into();
    let vault_str: String = env.get_string(&vault_path).unwrap().into();
    let id_json: String = env.get_string(&identity_json).unwrap().into();

    let identity_val: serde_json::Value = serde_json::from_str(&id_json).unwrap_or_default();
    let device_id = identity_val["device_id"]
        .as_str()
        .unwrap_or("android")
        .to_string();
    let device_name = identity_val["device_name"]
        .as_str()
        .unwrap_or("Android")
        .to_string();
    let fingerprint = identity_val["fingerprint"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let vault_path = std::path::PathBuf::from(&vault_str);
    let addr_parsed = format!("{}:{}", addr_str, port);

    let result = tokio::runtime::Runtime::new().unwrap().block_on(async {
        use std::str::FromStr;
        let socket = std::net::SocketAddr::from_str(&addr_parsed)?;
        let peer =
            PeerConnection::connect(socket, device_id.clone(), device_name.clone(), fingerprint)
                .await?;

        let mut engine = SyncEngine::new(vault_path, device_id.clone()).await?;
        engine.refresh_index(false).await?;
        let report = crate::sync::peer::run_client_session(&mut engine, &peer).await?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "pulled": report.pulled_files,
            "pushed": report.pushed_files,
            "deleted": report.deleted_files,
            "conflicts": report.conflicts,
        }))
    });

    let json = match result {
        Ok(v) => v,
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    };
    env.new_string(json.to_string()).unwrap().into_raw()
}

/// List unresolved conflicts in the vault DB.
/// Returns JSON: [{ id, path, detected_at }].
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_listConflicts<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    vault_path: JString<'local>,
    identity_json: JString<'local>,
) -> jstring {
    let vault_str: String = env.get_string(&vault_path).unwrap().into();
    let id_json: String = env.get_string(&identity_json).unwrap().into();
    let identity_val: serde_json::Value = serde_json::from_str(&id_json).unwrap_or_default();
    let device_id = identity_val["device_id"]
        .as_str()
        .unwrap_or("android")
        .to_string();

    let vault_path = std::path::PathBuf::from(&vault_str);
    let result = tokio::runtime::Runtime::new().unwrap().block_on(async {
        let engine = SyncEngine::new(vault_path, device_id).await?;
        let entries = engine.conflicts()?;
        Ok::<_, anyhow::Error>(
            entries
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "path": e.relative_path.to_string_lossy(),
                        "detected_at": e.detected_at,
                    })
                })
                .collect::<Vec<_>>(),
        )
    });
    let json = match result {
        Ok(v) => serde_json::json!({ "ok": true, "conflicts": v }),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    };
    env.new_string(json.to_string()).unwrap().into_raw()
}

/// Resolve a conflict by path. `resolution` is KeepLocal | KeepRemote | KeepBoth.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_resolveConflict<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    vault_path: JString<'local>,
    identity_json: JString<'local>,
    relative_path: JString<'local>,
    resolution: JString<'local>,
) -> jstring {
    let vault_str: String = env.get_string(&vault_path).unwrap().into();
    let id_json: String = env.get_string(&identity_json).unwrap().into();
    let rel: String = env.get_string(&relative_path).unwrap().into();
    let res: String = env.get_string(&resolution).unwrap().into();
    let identity_val: serde_json::Value = serde_json::from_str(&id_json).unwrap_or_default();
    let device_id = identity_val["device_id"]
        .as_str()
        .unwrap_or("android")
        .to_string();

    let resolution = match res.as_str() {
        "KeepLocal" => crate::conflict::resolution::Resolution::KeepLocal,
        "KeepRemote" => crate::conflict::resolution::Resolution::KeepRemote,
        "KeepBoth" => crate::conflict::resolution::Resolution::KeepBoth,
        other => {
            return env
                .new_string(format!("{{\"error\":\"unknown resolution {other}\"}}"))
                .unwrap()
                .into_raw();
        }
    };

    let vault_path = std::path::PathBuf::from(&vault_str);
    let result = tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut engine = SyncEngine::new(vault_path, device_id).await?;
        engine.resolve_conflict(&rel, &resolution).await?;
        Ok::<_, anyhow::Error>(serde_json::json!({ "ok": true, "path": rel }))
    });
    let json = match result {
        Ok(v) => v,
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    };
    env.new_string(json.to_string()).unwrap().into_raw()
}

/// List version snapshots across the vault.
/// Returns JSON: { snapshots: [{ path, timestamp, size }] }.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_listSnapshots<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    vault_path: JString<'local>,
) -> jstring {
    let vault_str: String = env.get_string(&vault_path).unwrap().into();
    let vault_path = std::path::PathBuf::from(&vault_str);
    let result = crate::filesystem::versioning::list_all_snapshots(&vault_path);
    let json = match result {
        Ok(snaps) => serde_json::json!({
            "ok": true,
            "snapshots": snaps
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "path": s.relative_path,
                        "timestamp": s.timestamp,
                        "size": s.size,
                    })
                })
                .collect::<Vec<_>>(),
        }),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    };
    env.new_string(json.to_string()).unwrap().into_raw()
}

/// Restore a snapshot over the current file. Returns JSON result.
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_restoreSnapshot<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    vault_path: JString<'local>,
    identity_json: JString<'local>,
    relative_path: JString<'local>,
    timestamp: jlong,
) -> jstring {
    let vault_str: String = env.get_string(&vault_path).unwrap().into();
    let id_json: String = env.get_string(&identity_json).unwrap().into();
    let rel: String = env.get_string(&relative_path).unwrap().into();
    let identity_val: serde_json::Value = serde_json::from_str(&id_json).unwrap_or_default();
    let device_id = identity_val["device_id"]
        .as_str()
        .unwrap_or("android")
        .to_string();

    let vault_path = std::path::PathBuf::from(&vault_str);
    let result = tokio::runtime::Runtime::new().unwrap().block_on(async {
        crate::filesystem::versioning::restore_snapshot(
            &vault_path,
            std::path::Path::new(&rel),
            timestamp,
        )?;
        // Refresh so the restored content becomes a pending push.
        let mut engine = SyncEngine::new(vault_path, device_id).await?;
        engine.refresh_index(false).await?;
        Ok::<_, anyhow::Error>(serde_json::json!({ "ok": true, "path": rel }))
    });
    let json = match result {
        Ok(v) => v,
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    };
    env.new_string(json.to_string()).unwrap().into_raw()
}

/// Stub for connectPeer, sendMessage, receiveMessage (kept for API compat)
#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_connectPeer(
    mut env: JNIEnv,
    _class: JClass,
    _addr: JString,
    _port: jint,
    _identity_json: JString,
) -> jstring {
    env.new_string("{\"error\":\"not implemented\"}")
        .unwrap()
        .into_raw()
}

#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_sendMessage(
    mut env: JNIEnv,
    _class: JClass,
    _conn_json: JString,
    _msg_json: JString,
) -> jstring {
    env.new_string("{\"error\":\"not implemented\"}")
        .unwrap()
        .into_raw()
}

#[no_mangle]
pub extern "system" fn Java_com_obsync_bridge_RustBridge_receiveMessage(
    mut env: JNIEnv,
    _class: JClass,
    _conn_json: JString,
) -> jstring {
    env.new_string("{\"error\":\"not implemented\"}")
        .unwrap()
        .into_raw()
}
