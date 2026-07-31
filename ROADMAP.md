# Obsync — Implementation Roadmap

## Milestone 1: Filesystem Indexing (Week 1-2)

**Goal:** A Rust library that walks a directory, computes BLAKE3 hashes, and stores FileState in SQLite.

**Tasks:**
- [ ] Initialize Cargo workspace with core library
- [ ] Implement `filesystem/io.rs` — streaming BLAKE3 hashing
- [ ] Implement `index/state.rs` — FileState struct
- [ ] Implement `index/store.rs` — SQLite schema + CRUD
- [ ] Implement `index/scanner.rs` — recursive directory walk with parallel hashing
- [ ] Implement `storage/db.rs` — connection management, migrations
- [ ] Write unit tests for hashing, path normalization, store CRUD
- [ ] Benchmark: index 1K / 10K / 100K files

**Deliverable:** `cargo test` passes. Library can index a directory and query file states.

---

## Milestone 2: File Watching & State Updates (Week 3-4)

**Goal:** React to filesystem changes, update index incrementally.

**Tasks:**
- [ ] Implement `filesystem/watcher.rs` — abstracted file watcher using `notify` crate
- [ ] Implement debounce logic for event bursts
- [ ] Implement `filesystem/ignore.rs` — filter editor temp files
- [ ] Wire watcher events → index update → hash (if needed)
- [ ] Implement `sync/queue.rs` — persistent change queue
- [ ] Test: create/modify/delete/rename files, verify state updates
- [ ] Test: rapid file changes, debounce correctness

**Deliverable:** Watcher detects changes, updates SQLite state, queues operations.

---

## Milestone 3: Local Sync Simulation (Week 5-6)

**Goal:** Two instances of the sync engine synchronize directories on the same machine.

**Tasks:**
- [ ] Implement `index/compare.rs` — manifest diff between two states
- [ ] Implement `sync/delta.rs` — generate operations from diff
- [ ] Implement `sync/engine.rs` — basic sync state machine
- [ ] Implement `filesystem/atomic.rs` — safe atomic writes
- [ ] Implement `conflict/detector.rs` — concurrent edit detection
- [ ] Implement `conflict/record.rs` — conflict metadata storage
- [ ] Build test harness: two directories, simulate operations, verify convergence
- [ ] Build randomized test: random file ops, verify State(A) == State(B)
- [ ] Test: create/modify/delete/rename/move bidirectionally
- [ ] Test: conflict detection and version preservation

**Deliverable:** `cargo test` includes integration tests for bidirectional sync between two directories.

---

## Milestone 4: Protocol & LAN Networking (Week 7-8)

**Goal:** Two machines on the same network can discover each other and exchange sync data.

**Tasks:**
- [ ] Design wire protocol (message types, serialization)
- [ ] Implement `network/protocol.rs` — message framing, versioning
- [ ] Implement `network/discovery.rs` — mDNS advertisement + discovery
- [ ] Implement `network/transport.rs` — TCP + encryption layer
- [ ] Implement `network/peer.rs` — connection lifecycle
- [ ] Wire sync engine to network transport
- [ ] Integrate discovery into sync engine state machine
- [ ] Test: two machines, pair, sync directory
- [ ] Test: disconnect/reconnect, queue + replay
- [ ] Test: protocol version mismatch rejection

**Deliverable:** Two machines on LAN can discover each other and synchronize a directory.

---

## Milestone 5: Identity & Pairing (Week 9-10)

**Goal:** Secure identity generation, key storage, and QR-code pairing.

**Tasks:**
- [ ] Implement `security/identity.rs` — X25519 keypair generation
- [ ] Implement `security/crypto.rs` — Noise Protocol encrypt/decrypt
- [ ] Implement `security/pairing.rs` — QR code generation, pairing flow
- [ ] Implement secure key storage (OS keychain on desktop, Android Keystore)
- [ ] Wire pairing into discovery and transport
- [ ] Test: pairing flow, key exchange, encrypted transport
- [ ] Test: replay attack prevention
- [ ] Test: peer revocation

**Deliverable:** Two devices can pair via QR code and establish encrypted communication.

---

## Milestone 6: Desktop UI (Week 11-13)

**Goal:** Tauri desktop application with full sync functionality.

**Tasks:**
- [ ] Initialize Tauri project
- [ ] Implement `VaultSelector` — directory picker
- [ ] Implement `PairingView` — QR code display
- [ ] Implement `DevicesList` — paired device management
- [ ] Implement `SyncDashboard` — status, stats, recent activity
- [ ] Implement `ConflictList` — conflict resolution UI
- [ ] Implement `SettingsPanel` — vault path, pause/resume, diagnostics
- [ ] Wire Tauri commands to Rust core
- [ ] Implement system tray with sync status
- [ ] Dark/light theme
- [ ] Accessibility review
- [ ] Test: full desktop workflow

**Deliverable:** Desktop application builds and runs. Can select vault, pair, and sync.

---

## Milestone 7: Android Client (Week 14-17)

**Goal:** Android application with sync functionality.

**Tasks:**
- [ ] Initialize Android project (Jetpack Compose)
- [ ] Set up JNI bridge to Rust core
- [ ] Implement SAF vault directory picker
- [ ] Implement QR scanner using CameraX
- [ ] Implement dashboard with sync status
- [ ] Implement device list management
- [ ] Implement file watcher (FileObserver)
- [ ] Implement foreground service for background sync
- [ ] Implement conflict resolution UI
- [ ] Handle Android lifecycle (sleep/wake, storage permissions)
- [ ] Test: full Android workflow

**Deliverable:** Android APK that pairs and syncs with desktop.

---

## Milestone 8: Offline & Conflict Polish (Week 18-19)

**Goal:** Robust offline operation, queue persistence, conflict workflow.

**Tasks:**
- [ ] Full offline queue testing across app restarts
- [ ] Implement tombstone GC logic
- [ ] Conflict resolution improvements (keep both, select version)
- [ ] Transfer resumption for large files
- [ ] Error recovery: corrupted transfer, partial write
- [ ] Graceful handling of permission changes, storage removal
- [ ] Edge case: sync during sleep/wake cycle
- [ ] Edge case: very large vault (100K+ files)
- [ ] Edge case: file names with special characters, Unicode

**Deliverable:** Robust sync under adverse conditions.

---

## Milestone 9: Performance Profiling (Week 20)

**Goal:** Measure and optimize against targets.

**Tasks:**
- [ ] Profile cold startup time
- [ ] Profile idle RAM and CPU (both platforms)
- [ ] Profile initial index of 1K / 10K / 100K files
- [ ] Profile single-file change latency
- [ ] Profile 1 MB and 1 GB transfer
- [ ] Profile reconnection and state reconciliation
- [ ] Profile metadata database size
- [ ] Optimize bottlenecks found
- [ ] Document performance characteristics

**Deliverable:** Performance report matching or exceeding targets.

---

## Milestone 10: Security Review & Packaging (Week 21-22)

**Goal:** Security audit, final testing, build artifacts.

**Tasks:**
- [ ] Security review of pairing protocol
- [ ] Security review of transport encryption
- [ ] Security review of key storage
- [ ] Test: adversarial network conditions
- [ ] Test: large-scale randomized convergence
- [ ] Build Tauri installer (AppImage/dmg/msi)
- [ ] Build Android APK/AAB
- [ ] Write user-facing README
- [ ] Write troubleshooting guide
- [ ] Tag v1.0.0

**Deliverable:** Release artifacts for desktop and Android.
