# Obsync Architecture

## Overview

Obsync uses a shared Rust sync core embedded in three consumers: the `obsync-httpd`
server (which is also embedded in a Tauri v2 desktop app) and an Android native
client. All synchronization logic lives in the core library. Platform clients
provide only UI and system integration.

```
                  ┌────────────────────────────────────────┐
                  │             obsync-httpd               │
                  │                                        │
                  │  ┌────────────┐   ┌────────────────┐   │
                  │  │ Dashboard  │   │  Sync TCP      │   │
                  │  │  :42021    │   │  server :42042 │   │
                  │  │ webui.html │   └───────┬────────┘   │
                  │  └─────┬──────┘           │            │
                  │        │ activity/SSE     │            │
                  │        ▼                  │            │
                  │  ┌────────────┐   ┌───────▼────────┐   │
                  │  │  AppState  │   │     Engine      │   │
                  │  └────────────┘   └───────┬────────┘   │
                  └───────────────────────────┼────────────┘
                                              │
                  ┌───────────────────────────┼────────────┐
                  │                    Sync Core (core/)    │
                  │                       │                 │
                  │   ┌─────────┐  ┌───────▼───────┐        │
                  │   │ Indexer │  │ Sync Engine   │        │
                  │   └────┬────┘  └───────┬───────┘        │
                  │        │               │                │
                  │   ┌────▼────┐  ┌───────▼───────┐        │
                  │   │ Store   │  │ Network peer  │        │
                  │   │(SQLite) │  │  (protocol)   │        │
                  │   └─────────┘  └───────┬───────┘        │
                  │        │               │                │
                  │   ┌────▼────┐  ┌───────▼───────┐        │
                  │   │Conflict │  │  Security     │        │
                  │   │(detect/ │  │  (crypto +    │        │
                  │   │ resolve)│  │   identity)   │        │
                  │   └─────────┘  └───────────────┘        │
                  └─────────────────────────────────────────┘
                                              │
                    ┌─────────────────────────┼─────────────┐
                    │                         │             │
          ┌─────────▼─────────┐   ┌───────────▼──────────┐
          │  Desktop (Tauri)  │   │  Android (JNI)       │
          │  embeds httpd     │   │  syncOnce → peer      │
          │  run_server()     │   │  client               │
          └───────────────────┘   └──────────────────────┘
```

---

## Layered Architecture

### Layer 1: Core Library (`core/`)

Platform-independent Rust library. No UI, no platform-specific dependencies
(except the Android JNI bridge in `android.rs`, gated by `#[cfg(target_os = "android")]`).

**Modules:**

- `index/` — file scanner, hashing (BLAKE3), state tracking in SQLite (`scanner`,
  `compare`, `store`, `state`).
- `sync/` — `engine` (refresh/apply/tombstone orchestration), `delta` (change
  computation), `peer` (client/server sync-session logic).
- `network/` — `peer` (hand-rolled TCP sync protocol, port 42042) and `protocol`
  (message framing). No QUIC/Noise/mDNS.
- `security/` — `crypto` (X25519 + AES-GCM session encryption) and `identity`
  (device IDs + key persistence).
- `conflict/` — `detector` (synced_hash-based divergence), `record`, `resolution`.
- `filesystem/` — `io` (streaming hash, read/write), `atomic` (atomic writes),
  `ignore`, `versioning` (snapshots), plus `now_millis()` helper.
- `storage/` — `config` and `db` (SQLite schema).
- `android.rs` — JNI surface for the Android app (`#[cfg(target_os = "android")]`).

**Responsibilities:**
- File indexing and hashing (BLAKE3)
- State tracking in SQLite
- Sync engine (refresh → diff → push/pull, authoritative vs additive)
- Conflict detection and recording
- P2P sync over a hand-rolled TCP protocol
- Device identity and pairing
- Cryptographic operations

### Layer 2: HTTP Server (`httpd/`)

`obsync-httpd` is a lib + thin binary. It owns:

- The web dashboard (single-file `webui.html`, served via `include_str!` on
  port 42021) — first-run pairing wizard, approve/reject, files/conflicts/
  versions/activity views.
- The P2P sync TCP server (port 42042), calling `engine.refresh_index(true)`
  before every session.
- A REST API (`/api/status`, `/api/select-vault`, `/api/sync-now`,
  `/api/files`, `/api/devices`, `/api/conflicts`, `/api/versions`,
  `/api/restore`, `/api/identity`, `/api/pairing-qr`, `/api/pending`,
  `/api/approve/:id`, `/api/reject/:id`, etc.).
- Live dashboard updates via Server-Sent Events (`/api/events`, tokio broadcast
  fed by `record_activity`), with a fallback polling loop in the UI.

The desktop Tauri app embeds `obsync_httpd::run_server()` directly and opens a
webview at `http://127.0.0.1:42021` — no separate IPC layer needed.

### Layer 3: Platform Clients

- **Desktop (Tauri v2):** thin wrapper around `obsync-httpd`. `frontendDist` is
  a static placeholder; the real UI is httpd's `webui.html` in the webview.
- **Android (Kotlin + JNI):** calls the Rust core via `jni`; syncs by dialing
  the desktop's `:42042` sync port (additive-only, `refresh_index(false)`).

---

## Module Communication

```
┌─────────────────────────────────────────────────────────┐
│                        httpd                             │
│                                                          │
│  ┌──────────────┐    ┌────────────┐   ┌───────────────┐  │
│  │  REST + SSE  │    │  AppState  │   │  Sync server  │  │
│  └──────┬───────┘    └─────┬──────┘   └───────┬───────┘  │
│         │ activity         │                  │          │
│         ▼                  ▼                  ▼          │
│  ┌────────────────────────────────────────────────────┐  │
│  │                    Engine                           │  │
│  └────────────────────────┬───────────────────────────┘  │
│                           │                              │
│                   ┌───────┼─────────────┐                │
│                   │       │             │                │
│              ┌────▼───┐ ┌─▼────┐ ┌──────▼─────┐         │
│              │ Index  │ │Store │ │   peer     │         │
│              │ scanner│ │(SQL) │ │ (protocol) │         │
│              └────────┘ └──────┘ └────────────┘         │
│                                                          │
└──────────────────────────────────────────────────────────┘
```

Sync sessions flow through `sync/peer.rs`: the client sends its manifest, the
server diffs (`index/compare.rs`), and both sides apply deltas (`delta.rs`) using
the conflict rules in `conflict/detector.rs`.

---

## Threading Model

```
Main Thread (UI)
    │
    ├── Tauri/Android event loop
    │
Async Runtime (tokio)
    │
    ├── Network I/O (sync sessions, SSE broadcast)
    ├── File I/O (streaming hash, read/write)
    ├── Sync engine (refresh/diff/apply)
    │
Blocking Pool
    │
    ├── SQLite operations (spawn_blocking)
    ├── CPU-bound hashing for large files
    └── Filesystem walks (refresh_index)
```

---

## Deployment Architecture

### Desktop
- Single binary (Tauri app embedding `obsync-httpd`)
- Embedded SQLite database in the vault's obsync data directory
- Rust core statically linked
- No external runtime dependencies

### Android
- APK with bundled native .so for Rust core (built via `cargo ndk` + Gradle
  `buildRust` task)
- SQLite via rusqlite (bundled)
- Syncs over the phone's hotspot / LAN to the desktop's `:42042` port

---

## Network Topology

```
Desktop (authoritative server, port 42042)
    │
    │  TCP, hand-rolled sync protocol (X25519/AES-GCM session)
    │
Android (client, additive-only)
    │
    └── Peer table: [device_id → (address, port, public_key)]
```

### Pairing
1. Desktop selects a vault and shows a QR code (`/api/pairing-qr`) containing
   host, port, and device identity.
2. Android scans the QR, connects to the sync port, and presents its own
   identity for approval.
3. The desktop approves or rejects (`/api/approve/:id`, `/api/reject/:id`);
   approvals persist in `~/.obsync-approved.json`.

### Sync Protocol
- TCP with X25519 + AES-GCM encrypted sessions
- Protocol version and message framing in `network/protocol.rs`
- Server refreshes its index (`refresh_index(true)`) before each session so
  direct disk edits reach the phone
- Conflict resolution keyed on `file_states.synced_hash` — see AGENTS.md
