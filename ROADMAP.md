# Obsync — Implementation Roadmap

> Status: all milestones implemented and shipped. The dependency audit later
> simplified the design — the filesystem watcher, change queue, mDNS discovery,
> Noise protocol, and chunked-transfer modules were removed in favor of a
> simpler hand-rolled TCP sync protocol with vault re-scanning. Where a task
> below names a removed module, it is kept for historical reference and marked
> **(superseded)**.

## Milestone 1: Filesystem Indexing (Week 1-2) — ✅ Complete

**Goal:** A Rust library that walks a directory, computes BLAKE3 hashes, and stores FileState in SQLite.

**Tasks:**
- [x] Initialize Cargo workspace with core library
- [x] Implement `filesystem/io.rs` — streaming BLAKE3 hashing
- [x] Implement `index/state.rs` — FileState struct
- [x] Implement `index/store.rs` — SQLite schema + CRUD
- [x] Implement `index/scanner.rs` — recursive directory walk with parallel hashing
- [x] Implement `storage/db.rs` — connection management
- [x] Write unit tests for hashing, path normalization, store CRUD
- [x] Benchmark: index 1K / 10K / 100K files

**Deliverable:** `cargo test` passes. Library can index a directory and query file states.

---

## Milestone 2: Change Discovery & State Updates (Week 3-4) — ✅ Complete (superseded)

**Goal:** React to filesystem changes, update index.

**Tasks:**
- [x] Implement `filesystem/watcher.rs` — **(superseded, removed)** the audit dropped watchers; changes are found by re-scanning at sync time
- [x] Implement `filesystem/ignore.rs` — filter editor temp files
- [x] Implement `sync/queue.rs` — **(superseded, removed)** no persistent change queue; sessions do full reconciliation
- [x] Test: create/modify/delete/rename files, verify state updates

**Deliverable:** Index updates on change (via re-scan on sync).

---

## Milestone 3: Local Sync Simulation (Week 5-6) — ✅ Complete

**Goal:** Two instances of the sync engine synchronize directories on the same machine.

**Tasks:**
- [x] Implement `index/compare.rs` — manifest diff between two states
- [x] Implement `sync/delta.rs` — generate operations from diff
- [x] Implement `sync/engine.rs` — basic sync state machine
- [x] Implement `filesystem/atomic.rs` — safe atomic writes
- [x] Implement `conflict/detector.rs` — divergent-edit detection (`synced_hash`)
- [x] Implement `conflict/record.rs` — conflict metadata storage
- [x] Build test harness: two directories, simulate operations, verify convergence
- [x] Build randomized test: random file ops, verify State(A) == State(B)
- [x] Test: create/modify/delete/rename/move bidirectionally
- [x] Test: conflict detection and version preservation

**Deliverable:** `cargo test` includes integration tests for bidirectional sync between two directories.

---

## Milestone 4: Protocol & LAN Networking (Week 7-8) — ✅ Complete

**Goal:** Two machines on the same network can exchange sync data.

**Tasks:**
- [x] Design wire protocol (message types, serialization, bincode over TCP)
- [x] Implement `network/protocol.rs` — message framing, versioning
- [x] Implement `network/discovery.rs` — **(superseded, removed)** the phone dials the desktop's QR-provided address directly
- [x] Implement `network/transport.rs` — **(superseded, removed)** encryption lives in `security/crypto.rs` (X25519 + AES-GCM)
- [x] Implement `network/peer.rs` — hand-rolled TCP sync server/client (port 42042)
- [x] Wire sync engine to network peer
- [x] Test: two machines, pair, sync directory
- [x] Test: disconnect/reconnect
- [x] Test: protocol version mismatch rejection

**Deliverable:** Desktop and Android on the same network can pair and synchronize a directory.

---

## Milestone 5: Identity & Pairing (Week 9-10) — ✅ Complete

**Goal:** Secure identity generation, key storage, and QR-code pairing.

**Tasks:**
- [x] Implement `security/identity.rs` — X25519 keypair generation + hand-rolled UUID v4 device id
- [x] Implement `security/crypto.rs` — encrypt/decrypt (AES-GCM)
- [x] Implement `security/pairing.rs` — **(superseded, removed)** pairing is done via QR (`/api/pairing-qr`) + approve/reject endpoints; approvals persist in `~/.obsync-approved.json`
- [x] Wire pairing into httpd (dashboard wizard, approve/reject UI)
- [x] Test: pairing flow, encrypted transport

**Deliverable:** Two devices can pair via QR code and establish encrypted communication.

---

## Milestone 6: Desktop App (Week 11-13) — ✅ Complete

**Goal:** Desktop application with full sync functionality.

**Tasks:**
- [x] Initialize Tauri v2 project
- [x] Dashboard served by httpd's `webui.html` (webview at `http://127.0.0.1:42021`)
- [x] Implement `VaultSelector` — directory picker (`/api/browse-vault`, manual path)
- [x] Implement `PairingView` — QR code display
- [x] Implement `DevicesList` — paired device management
- [x] Implement `SyncDashboard` — status, stats, recent activity (SSE + polling)
- [x] Implement `ConflictList` — conflict resolution UI
- [x] Implement `SettingsPanel` — vault path, diagnostics
- [x] Dark/light theme, accessibility review
- [x] Test: full desktop workflow

**Deliverable:** Desktop application builds and runs. Can select vault, pair, and sync.

---

## Milestone 7: Android Client (Week 14-17) — ✅ Complete

**Goal:** Android application with sync functionality.

**Tasks:**
- [x] Initialize Android project (Kotlin)
- [x] Set up JNI bridge to Rust core (`core/src/android.rs`, `buildRust` Gradle task)
- [x] Implement SAF vault directory picker
- [x] Implement QR scanner using CameraX
- [x] Implement dashboard with sync status
- [x] Implement device list management
- [x] Implement foreground service for background sync
- [x] Implement conflict resolution UI
- [x] Handle Android lifecycle (sleep/wake, storage permissions)
- [x] Test: full Android workflow

**Deliverable:** Android APK that pairs and syncs with desktop.

---

## Milestone 8: Offline & Conflict Polish (Week 18-19) — ✅ Complete

**Goal:** Robust offline operation, conflict workflow.

**Tasks:**
- [x] Full offline behavior across app restarts (reconciliation on reconnect)
- [x] Tombstone GC logic
- [x] Conflict resolution improvements (keep one, discard the other)
- [x] Error recovery: corrupted transfer, partial write
- [x] Graceful handling of permission changes, storage removal
- [x] Edge case: sync during sleep/wake cycle
- [x] Edge case: very large vault (100K+ files)
- [x] Edge case: file names with special characters, Unicode

**Deliverable:** Robust sync under adverse conditions.

---

## Milestone 9: Performance Profiling (Week 20) — ✅ Complete

**Tasks:**
- [x] Profile cold startup time
- [x] Profile idle RAM and CPU (both platforms)
- [x] Profile initial index of 1K / 10K / 100K files
- [x] Profile 1 MB and 1 GB transfer
- [x] Profile reconnection and state reconciliation
- [x] Profile metadata database size
- [x] Document performance characteristics

**Deliverable:** Performance report matching or exceeding targets.

---

## Milestone 10: Security Review & Packaging (Week 21-22) — ✅ Complete

**Tasks:**
- [x] Security review of pairing and transport encryption
- [x] Test: adversarial network conditions
- [x] Test: large-scale randomized convergence
- [x] Build Tauri installer (AppImage/dmg/msi) via `release.yml`
- [x] Build Android APK
- [x] Write user-facing README
- [x] Write troubleshooting guide
- [x] Tag v1.0.0

**Deliverable:** Release artifacts for desktop and Android.
