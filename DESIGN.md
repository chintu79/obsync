# Obsync — Technical Design Document

## System Architecture

```
                 ┌─────────────────────┐
                 │      Sync Core      │
                 │        Rust         │
                 └──────────┬──────────┘
                            │
              ┌─────────────┴─────────────┐
              │                           │
        Desktop Client              Android Client
        Tauri + React               Native/Flutter
              │                           │
        Desktop Vault                Mobile Vault
              └────────── P2P ───────────┘
```

## Core Library (`core/`)

A shared Rust library containing all synchronization logic. Both desktop and Android clients embed this library.

### Module Map

```
core/
├── filesystem/       # File I/O, watcher abstraction, atomic writes
│   ├── watcher.rs    # Platform-native filesystem event watcher
│   ├── io.rs         # Streaming read/write, hashing
│   ├── atomic.rs     # Safe atomic write operations
│   └── ignore.rs     # Editor temp file filtering
│
├── index/            # Vault state tracking
│   ├── state.rs      # FileState record, serialization
│   ├── scanner.rs    # Full vault walk + hash
│   ├── store.rs      # SQLite persistence for metadata
│   └── compare.rs    # Manifest diff (two device states)
│
├── sync/             # Core sync orchestration
│   ├── engine.rs     # Main sync state machine
│   ├── queue.rs      # Persistent change queue
│   ├── transfer.rs   # Chunked file transfer
│   └── delta.rs      # Operations from state comparison
│
├── conflict/         # Conflict detection & resolution
│   ├── detector.rs   # Concurrent edit detection
│   ├── record.rs     # Conflict metadata
│   └── resolution.rs # Version preservation logic
│
├── network/          # P2P networking
│   ├── discovery.rs  # mDNS peer discovery
│   ├── transport.rs  # Encrypted stream (QUIC/TCP+TLS)
│   ├── peer.rs       # Connected peer management
│   └── protocol.rs   # Message types, serialization
│
├── security/         # Cryptography & identity
│   ├── identity.rs   # Device keypair generation & storage
│   ├── pairing.rs    # QR code pairing protocol
│   └── crypto.rs     # Encrypt/decrypt, key exchange
│
└── storage/          # Persistent storage
    ├── config.rs     # Application configuration
    ├── db.rs         # SQLite connection management
    └── migrations.rs # Schema migrations
```

### Data Flow: File Change → Sync

```
Filesystem Event
      ↓
Watcher (debounced)
      ↓
Indexer (hash if changed)
      ↓
State Updated (SQLite)
      ↓
Sync Engine (determine operation)
      ↓
Conflict Check
      ↓
Queue Change
      ↓
Transfer (encrypted, chunked)
      ↓
Remote applies (atomic write)
      ↓
Remote Index Updated
```

### Data Flow: Full Sync on Connect

```
Peer Connected
      ↓
Exchange Manifests
      ↓
Compare States (manifest diff)
      ↓
Generate Operations (create/update/delete)
      ↓
Process Queue (prioritized)
      ↓
   small files → transfer directly
   large files → chunked streaming
      ↓
Verify Integrity
      ↓
Apply + Index
```

---

## Data Model

### FileState

```rust
struct FileState {
    relative_path: PathBuf,
    content_hash: Blake3Hash,    // 32 bytes
    size: u64,
    modified_at: Timestamp,
    revision: RevisionId,        // monotonic per-device counter
    sync_state: SyncState,
}
```

### Tombstone

```rust
struct Tombstone {
    relative_path: PathBuf,
    revision: RevisionId,
    deleted_at: Timestamp,
}
```

### Manifest

```rust
struct Manifest {
    device_id: DeviceId,
    files: Vec<FileState>,
    tombstones: Vec<Tombstone>,
    revision_counter: u64,
}
```

### SyncOperation

```rust
enum SyncOperation {
    Create { path: PathBuf, content_hash: Blake3Hash },
    Update { path: PathBuf, content_hash: Blake3Hash, base_revision: RevisionId },
    Delete { path: PathBuf, revision: RevisionId },
    Rename { from: PathBuf, to: PathBuf, content_hash: Blake3Hash },
}
```

### ProtocolMessage (wire format)

```rust
struct ProtocolMessage {
    version: u8,             // protocol version
    message_type: MessageType,
    request_id: u64,
    payload: Vec<u8>,        // encrypted
}
```

---

## State Machine

```
                 ┌──────────┐
                 │   IDLE    │
                 └────┬─────┘
                      │ discover
                      v
              ┌───────────────┐
              │  DISCOVERING  │
              └───────┬───────┘
                      │ peer found
                      v
              ┌───────────────┐
              │  CONNECTING   │
              └───────┬───────┘
                 ┌────┴────┐
                 │         │
                 v         v
           ┌─────────┐  ┌──────────┐
           │ SYNCING │  │  OFFLINE │
           └────┬────┘  └────┬─────┘
                │            │ peer found
                v            │
           ┌──────────┐      │
           │ CONFLICT │      │
           └────┬─────┘      │
                │ resolved    │
                v             v
              ┌──────────┐
              │   IDLE    │
              └──────────┘
```

---

## Conflict Detection Algorithm

```
1. For each file being synced:
   a. Local revision: R_local
   b. Remote revision: R_remote
   c. Common base: R_base (tracked in metadata)

2. Conflict if:
   R_local > R_base AND R_remote > R_base
   AND content_hash(local) != content_hash(remote)

3. On conflict:
   - Write local → filename.md
   - Write remote → filename.conflict-{device_id}.md
   - Record ConflictRecord in DB
   - Surface in UI
```

---

## Encryption Design

### Identity

```
DeviceIdentity:
  - device_id: UUID v4
  - keypair: X25519 (static)
  - created_at: Timestamp
  - label: human-readable name
```

### Pairing Flow

```
Desktop                          Phone
   │                               │
   │── Generate ephemeral key ────→│ (QR content)
   │←── Encrypted payload ────────│
   │── Verify ────────────────────→│
   │←── Confirm ──────────────────│
   │                               │
   Both persist peer's static key
```

### Transport Security

- Noise Protocol Framework (NX pattern) or equivalent
- Each message encrypted with per-session key derived via X25519 + AES-256-GCM / ChaCha20-Poly1305

---

## Storage Schema (SQLite)

```sql
CREATE TABLE file_states (
    relative_path TEXT PRIMARY KEY,
    content_hash BLOB NOT NULL,       -- BLAKE3 (32 bytes)
    size INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,     -- unix ms
    revision INTEGER NOT NULL,
    sync_state INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE tombstones (
    relative_path TEXT PRIMARY KEY,
    revision INTEGER NOT NULL,
    deleted_at INTEGER NOT NULL
);

CREATE TABLE conflicts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    relative_path TEXT NOT NULL,
    local_hash BLOB,
    remote_hash BLOB,
    local_revision INTEGER,
    remote_revision INTEGER,
    detected_at INTEGER NOT NULL,
    resolved INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE sync_queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation INTEGER NOT NULL,       -- 0=create, 1=update, 2=delete
    relative_path TEXT NOT NULL,
    content_hash BLOB,
    revision INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    retries INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE device_identity (
    device_id TEXT PRIMARY KEY,
    public_key BLOB NOT NULL,
    label TEXT,
    paired_at INTEGER NOT NULL,
    last_seen INTEGER
);

CREATE TABLE config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

---

## Key Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Sync engine language | Rust | Performance, safety, cross-platform, small binary |
| Desktop framework | Tauri + React | Small binary, Rust core, acceptable UI |
| Mobile framework | Android native/Kotlin | First-class Android APIs, SAF, background services |
| Hash algorithm | BLAKE3 | Fastest cryptographic hash, streaming support |
| Metadata storage | SQLite | Embedded, reliable, well-understood |
| Peer discovery | mDNS (libmdns/zeroconf) | Zero-config, LAN-only, widely supported |
| Transport encryption | Noise Protocol | Modern, audited, minimal dependencies |
| Wire protocol | Simple binary (protobuf or custom) | Compact, versioned, language-agnostic |
| Conflict strategy | Version preservation | Safe default, no data loss |

---

## Error Handling Strategy

```rust
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("network error: {0}")]
    Network(#[from] NetworkError),

    #[error("authentication failed: {0}")]
    Authentication(String),

    #[error("permission denied: {0}")]
    Permission(String),

    #[error("filesystem error: {0}")]
    Filesystem(#[from] std::io::Error),

    #[error("integrity check failed for {path}: expected {expected}, got {actual}")]
    Integrity { path: PathBuf, expected: String, actual: String },

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("{0}")]
    Other(String),
}
```

---

## Performance Targets

| Metric | Target | Method |
|---|---|---|
| Idle CPU | <0.5% | Event-driven watcher, no polling |
| Idle RAM (desktop core) | <30 MB | Minimal allocations, no file cache |
| Metadata DB (10K files) | <5 MB | Compact schema, no content storage |
| Initial index (10K files) | <30 s | Parallel walk + BLAKE3 streaming |
| Change detection latency | <2 s | Native FS events + debounce |
| 1 MB transfer | <500 ms | Direct streaming over LAN |
| 1 GB transfer | <2 min | Chunked streaming, bounded memory |
| Cold start | <2 s | Minimal initialization, lazy loading |
