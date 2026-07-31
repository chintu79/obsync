use std::path::PathBuf;

use clap::{Parser, Subcommand};
use obsync_core::filesystem::atomic::cleanup_stale_temps;
use obsync_core::filesystem::io::hash_file_path;
use obsync_core::index::scanner::scan_vault;
use obsync_core::network::peer::PeerConnection;
use obsync_core::security::identity::DeviceIdentity;
use obsync_core::storage::config::ConfigStore;
use obsync_core::storage::db;
use obsync_core::sync::engine::SyncEngine;
use obsync_core::sync::peer::run_client_session;
use tracing::info;

#[derive(Parser)]
#[command(name = "obsync", about = "Obsync CLI - Vault sync tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize and index a vault
    Init {
        /// Path to the vault directory
        path: PathBuf,
    },
    /// Index a vault and show stats
    Index {
        /// Path to the vault directory
        path: PathBuf,
    },
    /// Hash a file
    Hash {
        /// Path to the file
        path: PathBuf,
    },
    /// Show device identity
    Identity {
        /// Path to config directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Initialize a test peer with two directories for local sync testing
    TestPeer {
        /// Directory for peer A
        dir_a: PathBuf,
        /// Directory for peer B
        dir_b: PathBuf,
    },
    /// Sync a vault against a running sync server (e.g. the laptop httpd)
    Sync {
        /// Path to the local vault directory
        path: PathBuf,
        /// Server address (host:port)
        #[arg(default_value = "127.0.0.1:42042")]
        addr: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Init { path } => {
            cmd_init(&path).await?;
        }
        Command::Index { path } => {
            cmd_index(&path).await?;
        }
        Command::Hash { path } => {
            cmd_hash(&path).await?;
        }
        Command::Identity { path } => {
            cmd_identity(&path).await?;
        }
        Command::TestPeer { dir_a, dir_b } => {
            cmd_test_peer(&dir_a, &dir_b).await?;
        }
        Command::Sync { path, addr } => {
            cmd_sync(&path, &addr).await?;
        }
    }

    Ok(())
}

async fn cmd_init(path: &PathBuf) -> anyhow::Result<()> {
    info!("Initializing vault at {:?}", path);
    if !path.exists() {
        anyhow::bail!("Path does not exist: {:?}", path);
    }

    cleanup_stale_temps(path)?;
    db::ensure_db_directory(path)?;
    info!("Vault initialized");

    cmd_index(path).await //TODO: do it right here
}

async fn cmd_index(path: &PathBuf) -> anyhow::Result<()> {
    info!("Scanning vault at {:?}", path);

    let result = scan_vault(path).await?;
    info!("Found {} files", result.files.len());

    let store = db::open_db(path)?;
    for file in &result.files {
        store.upsert_file_state(file)?;
    }
    store.set_config("revision_counter", &result.revision_counter.to_string())?;

    info!("Indexed {} files (revision: {})", result.files.len(), result.revision_counter);

    // Show some stats
    let total_size: u64 = result.files.iter().map(|f| f.size).sum();
    info!(
        "Total size: {} bytes ({} KB, {} MB)",
        total_size,
        total_size / 1024,
        total_size / (1024 * 1024)
    );

    if !result.files.is_empty() {
        info!("First 5 files:");
        for file in result.files.iter().take(5) {
            info!(
                "  {:?} ({} bytes, hash: {:?})",
                file.relative_path,
                file.size,
                hex::encode(&file.content_hash[..4])
            );
        }
    }

    Ok(())
}

async fn cmd_hash(path: &PathBuf) -> anyhow::Result<()> {
    let hash = hash_file_path(path)?;
    println!("{}  {}", hex::encode(hash), path.display());
    Ok(())
}

async fn cmd_identity(path: &PathBuf) -> anyhow::Result<()> {
    let config_path = path.join(".obsync").join("config.bin");
    let store = ConfigStore::new(config_path);

    if let Some(identity) = DeviceIdentity::load(&store)? {
        println!("Device ID:     {}", identity.device_id);
        println!("Device Name:   {}", identity.device_name);
        println!("Fingerprint:   {}", identity.fingerprint());
        println!("Created:       {}", identity.created_at);
    } else {
        println!("No identity found. Creating a new one...");
        let identity = DeviceIdentity::generate("obsync-cli".into());
        identity.save(&store)?;
        println!("Device ID:     {}", identity.device_id);
        println!("Device Name:   {}", identity.device_name);
        println!("Fingerprint:   {}", identity.fingerprint());
    }

    Ok(())
}

async fn cmd_sync(path: &PathBuf, addr: &str) -> anyhow::Result<()> {
    info!("Syncing vault {:?} against {}", path, addr);
    if !path.exists() {
        anyhow::bail!("Vault path does not exist: {:?}", path);
    }
    cleanup_stale_temps(path)?;
    db::ensure_db_directory(path)?;

    let config_path = path.join(".obsync").join("config.bin");
    let store = ConfigStore::new(config_path);
    let (device_id, device_name, fingerprint) = if let Some(identity) = DeviceIdentity::load(&store)? {
        (
            identity.device_id.clone(),
            identity.device_name.clone(),
            identity.fingerprint(),
        )
    } else {
        let identity = DeviceIdentity::generate("obsync-cli".into());
        identity.save(&store)?;
        (
            identity.device_id.clone(),
            identity.device_name.clone(),
            identity.fingerprint(),
        )
    };
    info!("Device: {} ({})", device_name, device_id);

    let socket: std::net::SocketAddr = addr.parse()?;
    let mut engine = SyncEngine::new(path.clone(), device_id.clone()).await?;
    engine.initial_index().await?;
    info!("Local vault: {} files", engine.file_count());

    let peer = PeerConnection::connect(socket, device_id.clone(), device_name, fingerprint).await?;
    info!("Connected to {} ({})", peer.device_name, peer.address);

    let report = run_client_session(&mut engine, &peer).await?;
    info!("Sync complete");
    println!("pulled: {}, pushed: {}, deleted: {}, conflicts: {}",
        report.pulled_files, report.pushed_files, report.deleted_files, report.conflicts);

    Ok(())
}

async fn cmd_test_peer(dir_a: &PathBuf, dir_b: &PathBuf) -> anyhow::Result<()> {
    info!("Starting test peer sync between {:?} and {:?}", dir_a, dir_b);

    for dir in [dir_a, dir_b] {
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
        }
        cleanup_stale_temps(dir)?;
        db::ensure_db_directory(dir)?;
    }

    let mut engine_a = SyncEngine::new(dir_a.clone(), "peer-a".into()).await?;
    let mut engine_b = SyncEngine::new(dir_b.clone(), "peer-b".into()).await?;

    info!("Indexing peer A...");
    engine_a.initial_index().await?;
    info!("Peer A: {} files", engine_a.file_count());

    info!("Indexing peer B...");
    engine_b.initial_index().await?;
    info!("Peer B: {} files", engine_b.file_count());

    // Simulate sync: compare manifests
    let manifest_a = engine_a.build_manifest();
    let manifest_b = engine_b.build_manifest();

    let diff_a = engine_a.reconcile(&manifest_b);
    let diff_b = engine_b.reconcile(&manifest_a);

    info!(
        "Peer A needs {} operations, Peer B needs {} operations",
        diff_a.operations.len(),
        diff_b.operations.len()
    );
    info!("Peer A conflicts: {}", diff_a.conflicts.len());
    info!("Peer B conflicts: {}", diff_b.conflicts.len());

    for op in &diff_a.operations {
        info!("  A -> B: {:?}", op);
    }
    for op in &diff_b.operations {
        info!("  B -> A: {:?}", op);
    }

    info!("Test peer sync complete");
    Ok(())
}
