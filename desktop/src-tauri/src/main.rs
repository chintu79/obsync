use std::path::PathBuf;
use std::sync::Arc;

use obsync_core::filesystem::io::hash_file_path;
use obsync_core::index::scanner::scan_vault;
use obsync_core::index::state::{FileState, Manifest, SyncState};
use obsync_core::security::identity::DeviceIdentity;
use obsync_core::storage::config::ConfigStore;
use obsync_core::storage::db;
use obsync_core::sync::engine::{SyncEngine, SyncStateMachine};
use serde::Serialize;
use tauri::State;
use tokio::sync::Mutex;
use tracing::info;

struct AppState {
    engine: Arc<Mutex<Option<SyncEngine>>>,
    config_store: Arc<Mutex<Option<ConfigStore>>>,
}

#[derive(Serialize)]
struct VaultInfo {
    path: String,
    file_count: u64,
    state: String,
}

#[derive(Serialize)]
struct DeviceInfo {
    device_id: String,
    device_name: String,
    fingerprint: String,
}

#[tauri::command]
async fn select_vault(
    path: String,
    state: State<'_, AppState>,
) -> Result<VaultInfo, String> {
    let vault_path = PathBuf::from(&path);
    if !vault_path.exists() {
        return Err("Vault path does not exist".into());
    }

    db::ensure_db_directory(&vault_path).map_err(|e| e.to_string())?;

    let device_id = {
        let config_dir = vault_path.join(".obsync");
        let config_path = config_dir.join("config.bin");
        let store = ConfigStore::new(config_path);
        let identity = if let Some(id) = DeviceIdentity::load(&store).map_err(|e| e.to_string())? {
            id
        } else {
            let id = DeviceIdentity::generate("Desktop".into());
            id.save(&store).map_err(|e| e.to_string())?;
            id
        };
        identity.device_id.clone()
    };

    let mut engine = SyncEngine::new(vault_path.clone(), device_id)
        .await
        .map_err(|e| e.to_string())?;
    engine.initial_index().await.map_err(|e| e.to_string())?;

    let file_count = engine.file_count();
    let engine_state = engine.state();

    *state.engine.lock().await = Some(engine);

    Ok(VaultInfo {
        path,
        file_count,
        state: format!("{:?}", engine_state),
    })
}

#[tauri::command]
async fn get_vault_info(state: State<'_, AppState>) -> Result<VaultInfo, String> {
    let guard = state.engine.lock().await;
    let engine = guard.as_ref().ok_or("No vault selected")?;
    Ok(VaultInfo {
        path: "".into(),
        file_count: engine.file_count(),
        state: format!("{:?}", engine.state()),
    })
}

#[tauri::command]
async fn get_device_info(state: State<'_, AppState>) -> Result<DeviceInfo, String> {
    let guard = state.engine.lock().await;
    let engine = guard.as_ref().ok_or("No vault selected")?;

    let config_dir = PathBuf::from(""); // We need the vault path
    let config_path = config_dir.join("config.bin");
    let store = ConfigStore::new(config_path);

    if let Some(identity) = DeviceIdentity::load(&store).map_err(|e| e.to_string())? {
        Ok(DeviceInfo {
            device_id: identity.device_id,
            device_name: identity.device_name,
            fingerprint: identity.fingerprint(),
        })
    } else {
        Err("No device identity".into())
    }
}

#[tauri::command]
async fn get_sync_status(state: State<'_, AppState>) -> Result<String, String> {
    let guard = state.engine.lock().await;
    let engine = guard.as_ref().ok_or("No vault selected")?;
    Ok(format!("{:?}", engine.state()))
}

#[tauri::command]
async fn get_file_list(state: State<'_, AppState>) -> Result<Vec<FileStateInfo>, String> {
    let guard = state.engine.lock().await;
    let engine = guard.as_ref().ok_or("No vault selected")?;
    let manifest = engine.build_manifest();
    Ok(manifest
        .files
        .into_iter()
        .map(|f| FileStateInfo {
            path: f.relative_path.to_string_lossy().to_string(),
            size: f.size,
            modified_at: f.modified_at,
            sync_state: format!("{:?}", f.sync_state),
        })
        .collect())
}

#[derive(Serialize)]
struct FileStateInfo {
    path: String,
    size: u64,
    modified_at: i64,
    sync_state: String,
}

#[tauri::command]
async fn get_paired_devices(_state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    Ok(vec![])
}

#[tauri::command]
async fn get_conflicts(_state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    Ok(vec![])
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    info!("Starting Obsync Desktop");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            engine: Arc::new(Mutex::new(None)),
            config_store: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![
            select_vault,
            get_vault_info,
            get_device_info,
            get_sync_status,
            get_file_list,
            get_paired_devices,
            get_conflicts,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Obsync");
}
