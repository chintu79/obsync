# Obsync Architecture

## Overview

Obsync uses a shared Rust sync core embedded in a Tauri+React desktop app and an Android native client. All synchronization logic lives in the core library. Platform clients provide only UI and system integration.

```
                    ┌─────────────────────────────┐
                    │        Sync Core (Rust)      │
                    │                               │
                    │  ┌─────────┐  ┌───────────┐  │
                    │  │ Indexer │  │   Engine   │  │
                    │  └────┬────┘  └─────┬─────┘  │
                    │       │              │        │
                    │  ┌────▼────┐  ┌──────▼──────┐ │
                    │  │ Store   │  │  Network    │ │
                    │  └─────────┘  └──────┬──────┘ │
                    │                      │        │
                    │  ┌─────────┐  ┌──────▼──────┐ │
                    │  │Conflict │  │  Security   │ │
                    │  └─────────┘  └─────────────┘ │
                    └───────────────┬───────────────┘
                                    │
                    ┌───────────────┴───────────────┐
                    │                               │
          ┌─────────▼─────────┐           ┌─────────▼─────────┐
          │  Desktop Client   │           │  Android Client   │
          │  (Tauri + React)  │           │  (Kotlin/Jetpack) │
          │                    │           │                    │
          │  ┌──────────────┐ │           │  ┌──────────────┐ │
          │  │  Dashboard   │ │           │  │  Dashboard   │ │
          │  │  Devices      │ │           │  │  QR Scanner  │ │
          │  │  Conflicts   │ │           │  │  Settings    │ │
          │  │  Settings    │ │           │  │  Conflicts   │ │
          │  └──────────────┘ │           │  └──────────────┘ │
          └───────────────────┘           └───────────────────┘
```

---

## Layered Architecture

### Layer 1: Core Library (`core/`)

Platform-independent Rust library. No UI, no platform-specific dependencies (except where abstracted via traits).

**Responsibilities:**
- File indexing and hashing (BLAKE3)
- State tracking in SQLite
- Sync engine with state machine
- Conflict detection and recording
- Change queue management
- P2P transport (TCP + Noise Protocol)
- mDNS peer discovery
- Device identity and pairing
- Cryptographic operations

### Layer 2: Platform Bridge

Thin FFI or IPC layer exposing core functionality to the platform UI.

**Desktop (Tauri):**
- Rust core compiled as a Tauri command handler
- Tauri IPC bridge: React ↔ Rust core
- Filesystem watcher integration (notify crate)
- System tray integration

**Android (Kotlin):**
- Rust core compiled via JNI (jni crate)
- Kotlin wrapper exposing suspend functions
- FileSystemWatcherService using Android's FileObserver or DocumentFile
- Foreground service for background sync

### Layer 3: Platform UI

Thin presentation layer. No sync logic.

**Desktop (React + Tailwind):**
- SyncDashboard
- DeviceList
- PairingView (QR display)
- ConflictList
- SettingsPanel

**Android (Jetpack Compose):**
- SyncDashboard
- DeviceList
- QRScannerView
- ConflictList
- SettingsPanel

---

## Module Communication

```
┌─────────────────────────────────────────────────────────┐
│                      Application                         │
├─────────────────────────────────────────────────────────┤
│                                                          │
│  ┌──────────┐    ┌───────────┐    ┌───────────────────┐ │
│  │ Watcher  │───→│  Engine   │←───│  Network Service  │ │
│  └──────────┘    └─────┬─────┘    └───────────────────┘ │
│                        │                                 │
│              ┌─────────┼─────────┐                       │
│              │         │         │                       │
│         ┌────▼───┐ ┌──▼───┐ ┌───▼────┐                  │
│         │ Index  │ │Queue │ │Conflict│                  │
│         └────┬───┘ └──────┘ └───┬────┘                  │
│              │                  │                        │
│         ┌────▼────────────┐     │                        │
│         │  SQLite Store   │◄────┘                        │
│         └─────────────────┘                              │
│                                                          │
└─────────────────────────────────────────────────────────┘
```

---

## Threading Model

```
Main Thread (UI)
    │
    ├── Tauri/Android event loop
    │
Async Runtime (tokio)
    │
    ├── Network I/O (transport, discovery)
    ├── File I/O (streaming hash, read/write)
    ├── Sync engine (state machine)
    │
Blocking Pool
    │
    ├── SQLite operations
    ├── CPU-bound hashing for large files
    └── Filesystem walks
```

---

## Platform-Specific Abstractions

```rust
/// Filesystem watcher abstraction
#[cfg(target_os = "linux")]
type Watcher = inotify::InotifyWatcher;
#[cfg(target_os = "macos")]
type Watcher = fsevent::FsEventWatcher;
#[cfg(target_os = "windows")]
type Watcher = readdir::ReadDirectoryChangesWatcher;

trait FileWatcher {
    fn watch(&mut self, path: &Path) -> Result<()>;
    fn event_stream(&mut self) -> Box<dyn Stream<Item = WatchEvent>>;
}
```

---

## Deployment Architecture

### Desktop
- Single binary (Tauri app)
- Embedded SQLite database in app data directory
- Rust core statically linked
- No external runtime dependencies

### Android
- APK with bundled native .so for Rust core
- SQLite via rusqlite (or Android SQLite via JNI bridge)
- Foreground service with low-priority notification
- SAF-compatible file access

---

## Network Topology (V1)

```
Desktop (server + client)
    │
    │  mDNS: _obsync._tcp.local
    │  TCP: Noise-encrypted stream
    │
Android (client + server)
    │
    └── Peer table: [desktop_id → (address, port, public_key)]
```

### Discovery Protocol
1. Desktop advertises `_obsync._tcp.local` via mDNS
2. Android discovers service, resolves address
3. If previously paired → connect directly
4. If not paired → show in device list for pairing

### Transport Protocol
- TCP with Noise Protocol (NX handshake)
- Protocol version in every message
- Request-response with correlation IDs
- Streaming for large file transfers
