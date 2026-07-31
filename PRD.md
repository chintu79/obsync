# Obsync — Product Requirements Document

## Product Overview

Obsync is a local-first, peer-to-peer synchronization tool for Obsidian/Markdown vaults. It synchronizes a selected directory between a desktop computer and an Android device over the local network, with no cloud dependency, no accounts, and no manual configuration beyond QR-code pairing.

The existing filesystem is the source of truth. Obsync never copies vault contents into a proprietary database. It maintains only compact synchronization metadata.

---

## Goals

- Synchronize a selected directory bidirectionally between desktop and Android
- Detect and transfer only changed files
- Work with arbitrary files: `.md`, images, PDFs, attachments, nested directories, `.obsidian/` config
- Require zero configuration beyond QR pairing
- Operate fully offline (no internet required)
- Encrypt all traffic; assume the local network is untrusted
- Consume negligible resources while idle
- Never lose data

---

## Non-Goals (V1)

- Cloud storage or relay servers
- User accounts or authentication providers
- Web application
- Real-time collaborative editing
- CRDTs
- AI features
- Version history browser
- Internet relay / TURN/STUN
- Public sharing links
- Team collaboration
- Plugins or extension system
- Analytics or telemetry
- Git integration

---

## User Personas

### Primary: Obsidian Power User
Maintains a substantial Markdown vault (hundreds to thousands of files) across a desktop and phone. Wants seamless, wireless, encrypted sync without thinking about infrastructure.

### Secondary: Privacy-Conscious User
Wants full control over data. Will not use cloud services. Needs LAN-only sync with no phone-home capability.

---

## User Stories

| ID | Story |
|---|---|
| US-01 | As a user, I install Obsync on my desktop and Android phone without creating an account. |
| US-02 | As a user, I select my vault directory on desktop. |
| US-03 | As a user, I scan a QR code on my phone to pair with my desktop. |
| US-04 | As a user, my vault stays synchronized without me thinking about it. |
| US-05 | As a user, changes I make on my phone appear on my desktop and vice versa. |
| US-06 | As a user, synchronization works over my local Wi-Fi with no internet. |
| US-07 | As a user, all data is encrypted during transfer. |
| US-08 | As a user, I can see sync status at a glance. |
| US-09 | As a user, conflicts are surfaced clearly and I can resolve them. |
| US-10 | As a user, adding a large file doesn't block small file sync. |
| US-11 | As a user, the app uses negligible resources when idle. |
| US-12 | As a user, interrupted transfers don't corrupt my files. |
| US-13 | As a user, I can pause and resume sync. |

---

## Functional Requirements

### FR-01: Vault Selection
- Desktop: directory chooser, stores path in config
- Android: SAF directory picker for scoped storage

### FR-02: Filesystem Monitoring
- Native filesystem events (inotify on Linux, FSEvents on macOS, ReadDirectoryChanges on Windows)
- Debounce bursts (configurable, default ~500ms)
- Ignore common temp/editor files
- Prevent sync feedback loops

### FR-03: Indexing
- Walk vault on first run and after unclean shutdown
- Store: relative_path, content_hash (BLAKE3), size, mtime, revision, sync_state
- Compact SQLite storage
- Never store file contents in metadata DB

### FR-04: Pairing
- QR code displayed on desktop
- Scanned by Android camera
- Contains: protocol version, device ID, public key fingerprint, ephemeral pairing token
- Single pairing ceremony; persisted trust

### FR-05: LAN Discovery
- mDNS-based zero-config peer discovery
- No manual IP entry
- Background discovery, negligible idle resource use

### FR-06: Encrypted Transport
- Authenticated encryption for all traffic
- Persistent device identity keypair
- Key exchange during pairing
- Replay-resistant

### FR-07: Bidirectional Sync
- Full state reconciliation on connect
- Incremental changes streamed in real-time
- Create / modify / delete / rename / move support
- RENAME detected via content hash match

### FR-08: Conflict Handling
- Detect concurrent edits to same base version
- Preserve both versions on both devices
- File.conflict-{device}.md naming
- Record conflict metadata (device, timestamps, revision)
- Conflict resolution UI

### FR-09: Safe Writes
- Write to `.sync-temp` first
- Verify hash/size on completion
- Atomic rename to final path
- Clean up stale temps on startup

### FR-10: Offline Queue
- Persist pending changes across restarts
- References to file revisions, not copies of content
- Auto-sync when peer reconnects

### FR-11: Deletion Synchronization
- Tombstones (compact: path + revision + timestamp)
- Prevent offline-peer resurrection
- GC tombstones after safe period

### FR-12: Conflict UI
- List conflicted files
- Show: file name, conflicting devices, timestamps
- Actions: keep local, keep remote, keep both, open file

### FR-13: Settings
- Vault directory path
- Pause/resume sync
- View paired devices
- Unpair device
- Export diagnostics

---

## Non-Functional Requirements

| ID | Requirement | Target |
|---|---|---|
| NFR-01 | Idle CPU usage | <0.5% on modern hardware |
| NFR-02 | Idle RAM (desktop core) | <30 MB |
| NFR-03 | Idle RAM (Android service) | <20 MB |
| NFR-04 | Cold start (desktop) | <2 seconds |
| NFR-05 | Cold start (Android) | <1 second |
| NFR-06 | Initial index (10K files) | <30 seconds |
| NFR-07 | Change detection latency | <2 seconds typical |
| NFR-08 | Metadata DB size (10K files) | <5 MB |
| NFR-09 | Transfer encryption overhead | <10% throughput penalty |
| NFR-10 | Mean time to sync single .md | <1 second (LAN) |

---

## Out of Scope (V1 — Listed for Clarity)

| Feature | Rationale |
|---|---|
| Cloud relay | Adds complexity, dependency, attack surface |
| TURN/STUN | Not needed for LAN V1 |
| Multi-device (3+) | Increases complexity; V2 consideration |
| Selective sync per folder | Nice-to-have; adds UI/config complexity |
| Git-style version history | Filesystem is truth; V2+ consideration |
| Binary diff (bsdiff/xdelta) | Not justified without measurement |
| Service workers / PWA | Not needed; Tauri + Android native |
| iOS support | Not in scope for V1 |
