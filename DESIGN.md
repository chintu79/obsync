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
├── filesystem/       # File I/O, atomic writes, versioning
│   ├── io.rs         # Streaming read/write, hashing
│   ├── atomic.rs     # Safe atomic write operations
│   ├── ignore.rs     # Editor temp file filtering
│   └── versioning.rs # Snapshot/version preservation
│
├── index/            # Vault state tracking
│   ├── state.rs      # FileState record, serialization
│   ├── scanner.rs    # Full vault walk + hash
│   ├── store.rs      # SQLite persistence for metadata
│   └── compare.rs    # Manifest diff (two device states)
│
├── sync/             # Core sync orchestration
│   ├── engine.rs     # Main sync state machine
│   ├── peer.rs       # Client/server sync-session logic
│   └── delta.rs      # Operations from state comparison
│
├── conflict/         # Conflict detection & resolution
│   ├── detector.rs   # Concurrent edit detection
│   ├── record.rs     # Conflict metadata
│   └── resolution.rs # Version preservation logic
│
├── network/          # P2P networking
│   ├── peer.rs       # Hand-rolled TCP sync protocol
│   └── protocol.rs   # Message types, serialization
│
├── security/         # Cryptography & identity
│   ├── identity.rs   # Device keypair generation & storage
│   └── crypto.rs     # Encrypt/decrypt session cipher
│
└── storage/          # Persistent storage
    ├── config.rs     # Application configuration
    └── db.rs         # SQLite connection management
```

### Data Flow: Full Sync on Connect

```
Peer Connected
      ↓
Hello / HelloAck handshake (X25519 session key)
      ↓
Exchange Manifests
      ↓
Compare States (manifest diff)
      ↓
Generate Operations (create/update/delete)
      ↓
Request + stream each changed file (chunked)
      ↓
Verify Integrity (BLAKE3)
      ↓
Apply + Index (atomic write)
```

### Change Discovery

File changes are NOT detected by a filesystem watcher. The server refreshes its
index (`engine.refresh_index(true)`) at the start of every sync session, walking
the vault and re-hashing changed files, so edits made directly on the laptop
disk always reach the phone. The authoritative server passes
`detect_deletions = true`; the phone client passes `false` (additive-only)
because its disk may be an incomplete replica.

---

## Data Model

### FileState

```rust
struct FileState {
    relative_path: PathBuf,
    content_hash: Blake3Hash,    // 32 bytes
    size: u64,
    modified_at: i64,            // unix ms
    revision: RevisionId,        // monotonic per-device counter
    sync_state: SyncState,
    synced_hash: Option<Blake3Hash>, // hash last sync agreed on (None = pre-migration)
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
    payload: Vec<u8>,        // bincode-serialized
}
```

Message types (see `network/protocol.rs`): `Hello`/`HelloAck` (handshake),
`Manifest`, `FileRequest`, `FileChunk` (sync), `SyncOperation`/`OperationAck`
(ops), `Ping`/`Disconnect` (control). Files are transferred as chunked
`FileRequest`/`FileChunk` exchanges, each chunk verified against the target
BLAKE3 hash as it is applied.

---

## State Machine

```
                 ┌──────────┐
                 │   IDLE    │
                 └────┬─────┘
                      │ connect to known peer
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

## Conflict Detection Algorithm

```
1. For each file present on both sides with different content:
   a. Was either side unchanged since the last sync agreement?

2. The agreement signal is synced_hash (the content hash the last sync agreed
   on). Revisions are per-device local counters and are NOT a reliable base:
   a file edited on both devices ever has revision > 0 on both sides.

3. Cases:
   - local unchanged since agreement (local.synced_hash == local hash)
     → remote is simply newer → pull remote
   - remote unchanged since agreement → local is simply newer → push local
   - BOTH changed since agreement → genuine conflict
   - No agreement recorded (pre-migration, synced_hash is NULL)
     → newer mtime wins

4. On conflict:
   - Leave the working copy untouched; both versions preserved
   - Write remote → filename.conflict-{device_id}.md
   - Record ConflictRecord in DB
   - Surface in UI (the user keeps one and discards the other)
```

> Revisions are per-engine local counters, so "revision > 0 on both sides"
> must NEVER be treated as "both changed since last sync" — any hash
> difference would otherwise be a permanent false conflict. The only correct
> signal is `file_states.synced_hash`. See `conflict/detector.rs`.

---

## Encryption Design

### Identity

```
DeviceIdentity:
  - device_id: UUID v4 (hand-rolled from OS RNG)
  - keypair: X25519 (static)
  - created_at: unix ms
  - label: human-readable name
```

Persisted via `ConfigStore` (hex-encoded) in the vault's obsync config.

### Pairing Flow

```
Desktop                          Phone
   │                               │
   │── QR code (host, port, id) ──→│ (phone scans)
   │←── Hello (device id, fp) ────│
   │── approve/reject (HTTP) ────→│
   │                               │
   Approved device fingerprints persist in ~/.obsync-approved.json
```

### Transport Security

- X25519 key agreement for a per-session AES-256-GCM key
- Every sync session is encrypted with that key
- Hello payloads carry the peer's public-key fingerprint; the server only
  accepts connections from approved devices

---

## Storage Schema (SQLite)

```sql
CREATE TABLE file_states (
    relative_path TEXT PRIMARY KEY,
    content_hash BLOB NOT NULL,       -- BLAKE3 (32 bytes)
    size INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,     -- unix ms
    revision INTEGER NOT NULL,
    sync_state INTEGER NOT NULL DEFAULT 0,
    synced_hash BLOB                  -- v2: hash last sync agreed on (NULL = none)
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

Schema is created in `core/src/index/store.rs::migrate()`; the `synced_hash`
column is added via `ALTER TABLE` when upgrading a pre-v2 database.

---

## Key Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Sync engine language | Rust | Performance, safety, cross-platform, small binary |
| Desktop framework | Tauri v2 (webview over httpd) | Small binary, Rust core, no build step for the UI |
| Mobile framework | Android native/Kotlin | First-class Android APIs, SAF, background services |
| Hash algorithm | BLAKE3 | Fastest cryptographic hash, streaming support |
| Metadata storage | SQLite | Embedded, reliable, well-understood |
| Peer discovery | None (static host/port from QR) | The phone scans the desktop's QR; no LAN discovery needed |
| Transport encryption | X25519 + AES-256-GCM | Minimal, audited primitives; hand-rolled sync protocol |
| Wire protocol | bincode over TCP | Compact, versioned, language-agnostic |
| Conflict strategy | Version preservation + synced_hash | Safe default, no data loss, no false conflicts |
| File-change detection | Vault re-scan on sync | No watcher dependency; simple and correct |

---

## Error Handling Strategy

```rust
// NetworkError is hand-rolled (no thiserror dependency):
//   - Connection, Protocol, Encryption, Timeout, Io
//   - impl std::error::Error + Display, with From<std::io::Error>

#[derive(Debug, Clone)]
pub enum SyncError {
    Network(NetworkError),
    Authentication(String),
    Permission(String),
    Filesystem(std::io::Error),
    Integrity { path: PathBuf, expected: String, actual: String },
    Conflict(String),
    Protocol(String),
    Other(String),
}
```

---

## Performance Targets

| Metric | Target | Method |
|---|---|---|
| Idle CPU | <0.5% | No watcher; only a slow status ticker and SSE broadcast |
| Idle RAM (desktop core) | <30 MB | Minimal allocations, no file cache |
| Metadata DB (10K files) | <5 MB | Compact schema, no content storage |
| Initial index (10K files) | <30 s | Parallel walk + BLAKE3 streaming |
| Change detection latency | On next sync | Vault re-scan at session start |
| 1 MB transfer | <500 ms | Direct streaming over LAN |
| 1 GB transfer | <2 min | Chunked streaming, bounded memory |
| Cold start | <2 s | Minimal initialization, lazy loading |
