# Obsync → Obsidian Plugin: Migration Plan

Status: proposal · Target: replace the Tauri desktop app + Android APK with a
single Obsidian plugin that runs on both desktop and mobile.

## 1. Goal and scope

Ship Obsync as an Obsidian plugin so users install it from the community
plugins catalog instead of downloading a desktop app + an APK. One plugin, two
roles, matching the existing architecture:

| Host | Role | Existing equivalent |
| --- | --- | --- |
| Obsidian **desktop** (Electron + Node) | sync server + dashboard UI | `obsync-httpd` (:42042 + :42021) + Tauri app |
| Obsidian **mobile** (Capacitor) | sync client | `android/` app (JNI → `core/src/android.rs`) |

The Rust `core/` engine and `obsync-httpd` stay in the repo as the reference
implementation and the headless/NAS/self-host path.

## 2. Key architectural change: transport

The current wire protocol is raw TCP + bincode frames
(`core/src/network/peer.rs`, `core/src/network/protocol.rs`). **Mobile Obsidian
plugins cannot open sockets** — Capacitor only exposes `requestUrl()` (HTTP).
Desktop plugins *can* open servers (`require('http')`; precedent:
`obsidian-local-rest-api`).

⇒ Replace the TCP/bincode transport with **HTTP + JSON** in the plugin path.
The message set stays identical (manifest exchange, file request, file chunk,
sync ops) — only framing changes. The Rust server keeps its TCP protocol for
the NAS path, or we add an HTTP bridge later; the plugin never needs it.

## 3. Module-by-module port mapping

Rust (`core/src/...`) → TypeScript (`plugin/src/core/...`). Pure-logic modules
port 1:1 and must mirror their `cargo test` suite in vitest.

### Ported as-is (logic unchanged)

| Rust module | TS module | Notes |
| --- | --- | --- |
| `index/state.rs` | `core/state.ts` | `FileState`, `Manifest`, `Tombstone`, `SyncState` — same fields |
| `sync/delta.rs` | `core/delta.ts` | `SyncOperation` enum |
| `index/compare.rs` | `core/compare.ts` | `compare_manifests`, `ManifestDiff` — pure |
| `conflict/detector.rs` | `core/conflict.ts` | `resolve_divergence`, `SideOutcome` — the `synced_hash` decision point |
| `conflict/record.rs`, `conflict/resolution.rs` | `core/conflict.ts` | `ConflictEntry`, `Resolution`, conflict-copy naming |
| `filesystem/ignore.rs` | `core/ignore.ts` | copy rules; **add `.obsync/` to the `.obsidian`-style exception** |
| `filesystem/versioning.rs` | `core/versioning.ts` | `snapshot_before_overwrite` → copy previous content to `.obsync/versions/` |

### Ported with a changed backend

| Rust module | TS module | Change |
| --- | --- | --- |
| `storage/db.rs` + `index/store.rs` | `core/store.ts` | rusqlite → a JSON index file (`.obsync/index.json`) read/written through the vault adapter. Same API: `upsert_file_state`, `get_file_state`, `get_all_file_states`, tombstones, conflicts, config, revision counter. SQLite is not available to mobile plugins; a Map + JSON file is faithful for a note-sized vault |
| `filesystem/io.rs` (hash/stat/read) | `core/hash.ts` | `blake3::hash` → `@noble/hashes/blake3`; file reads through `app.vault.adapter` |
| `filesystem/atomic.rs` | `core/atomic.ts` | `AtomicWriter` (temp + rename) → `vault.modify`/`create` on the adapter (desktop) or Node `fs` write-tmp-rename (when running in a Node context) |
| `index/scanner.rs` | `core/scanner.ts` | walking disk → iterate `vault.getFiles()`; incremental re-hash only files whose `stat.mtime`/size changed |
| `security/crypto.rs` | `core/crypto.ts` | `aes-gcm` → `@noble/ciphers/aes` (AES-256-GCM, same nonce layout) |
| `security/identity.rs` | `core/identity.ts` | `x25519-dalek` → `@noble/curves/x25519`; `sha2` → `@noble/hashes/sha256`; v4 UUID via `crypto.randomUUID`; hex identity persistence |
| `network/protocol.rs` | `core/protocol.ts` | bincode → JSON message bodies over HTTP routes |
| `network/peer.rs` (TCP handshake) | `core/transport.ts` | client = `requestUrl`, server = Node `http`; handshake = POST `/pair` |

### Ported with the same decision logic (this is the "reuse the engine" core)

| Rust module | TS module | Change |
| --- | --- | --- |
| `sync/engine.rs` | `core/engine.ts` | `SyncEngine`: `record_remote_file`, `mark_synced`, `plan_conflict_copy`, `conflict_copy_path`, `resolve_conflict`, `refresh_index(detect_deletions)`, `build_manifest`, `reconcile`, `apply_operation` — all port verbatim. `now_millis()` → `Date.now()`. Filesystem ops via the vault adapter |
| `sync/peer.rs` (client/server sessions) | `core/session.ts` | `run_client_session` / `run_server_session` split into request handlers. Same pull/push/conflict/delete ordering (steps 3–7) |

### Dropped

| Artifact | Why |
| --- | --- |
| `android.rs` (JNI bridge) | mobile plugin replaces the app |
| `desktop/` (Tauri) | desktop plugin replaces it; remove from release matrix once stable |
| `android/` APK | replaced by the mobile plugin |
| bincode | JSON is self-describing; both ends are JS in the plugin path |

## 4. Where each piece runs

```
DESKTOP OBSIDIAN (Electron, Node)          MOBILE OBSIDIAN (Capacitor)
┌──────────────────────────────┐          ┌──────────────────────────────┐
│ plugin main.ts               │          │ plugin main.ts               │
│  ├─ core/engine.ts           │          │  ├─ core/engine.ts           │
│  ├─ core/store.ts (JSON)     │          │  ├─ core/store.ts (JSON)     │
│  ├─ core/session.ts (server) │◄─ HTTP ─►│  └─ core/session.ts (client) │
│  └─ ui/ (settings tab:       │  :42042  │     transport = requestUrl   │
│      status, approve,        │          │                              │
│      conflicts, restore)     │          │                              │
└──────────────────────────────┘          └──────────────────────────────┘
```

- **Desktop is authoritative**: `refresh_index(true)` (deletions tombstoned),
  same as the current laptop server.
- **Mobile is additive**: `refresh_index(false)`, same as the current phone.
- **Conflict model unchanged**: `resolve_divergence` on `synced_hash`
  (`core/src/conflict/detector.rs`) is ported untouched — this is the single
  decision point and must not drift.

## 5. Storage details

- Index file: `.obsync/index.json` inside the vault (hidden; the ignore list
  must allow `.obsync/` the way it allows `.obsidian/`).
- Plugin settings: Obsidian `loadData`/`saveData` for pairing state and
  approved devices.
- Snapshots: `.obsync/versions/` — same layout as the Rust `versioning.rs`.
- Migration: on first run, if a Rust `.obsync/state.db` exists, import it
  (same table → same JSON keys) so current users don't lose pairing/history.

## 6. Crypto note (gap to close)

`security/crypto.rs` defines AES-256-GCM encrypt/decrypt, but the current TCP
session path never calls it — the transport carries the public-key fingerprint
(`HelloPayload`) but exchanges no session key. The plugin migration is the
opportunity to actually wire it: X25519 key agreement during pairing +
AES-256-GCM per request, using the modules that already exist.

## 7. Plugin project layout

```
plugin/
  manifest.json, main.ts, styles.css
  src/
    core/          # engine, store, compare, conflict, session, crypto, identity, hash, ignore, versioning, transport
    ui/            # settings tab (pair, status, conflicts, restore), QR render
    vault-adapter.ts  # thin Obsidian API wrapper so core/ is testable in Node
  tests/           # vitest, mirrored from cargo test
  esbuild.config.mjs
```

`core/` takes a `VaultAdapter` interface (getFiles, read, write, stat, mkdir,
remove) — that single seam is what makes the engine unit-testable outside
Obsidian and lets desktop use Node `fs` while mobile uses the Capacitor adapter.

## 8. Dependency choices

Keep the dependency-minimalism rule. New JS deps, all pure TS / audited,
matching the Rust set 1:1:

- `@noble/hashes` (blake3, sha256) → replaces `blake3`, `sha2`, `hex`
- `@noble/curves` (x25519) → replaces `x25519-dalek`
- `@noble/ciphers` (aes256gcm) → replaces `aes-gcm`
- vitest (dev) → mirrors `cargo test`
- esbuild (build) → standard Obsidian plugin bundler
- QR rendering: reuse the inlined `qr-code-styling` approach from
  `httpd/src/webui.html`

No SQLite, no tokio, no serde analogues. No framework — the plugin UI is
settings-tab DOM, matching the existing no-build webui style.

## 9. Phased plan with gates

| Phase | Work | Gate |
| --- | --- | --- |
| **0. Spike** | Minimal plugin: desktop opens an HTTP server, mobile `requestUrl`s it over the hotspot. Confirm `require('http')` server access + mobile reachability | laptop plugin serves, phone plugin fetches a test payload |
| **1. Port pure logic** | `state/delta/compare/conflict/ignore/versioning` → TS + vitest | vitest mirrors `cargo test` for those modules |
| **2. Store + scanner** | `store.ts` on the vault adapter, `scanner.ts` over `vault.getFiles()`, `hash.ts` (blake3) | indexing a sample vault matches Rust's manifest hashes |
| **3. Crypto + transport** | `crypto/identity/protocol/transport` + wire AES-GCM session | TS client↔server in-process round-trip test; crypto vectors match Rust |
| **4. Engine + desktop UI** | `engine.ts`, `session.ts` server side, settings tab (pair, approve, conflicts, restore) | desktop-only sync test: create/edit/delete propagate both ways |
| **5. Mobile client** | same `core/`, `requestUrl` transport; real phone | live laptop⇄phone sync + forced-conflict test |
| **6. Replace artifacts** | drop Tauri + APK from `release.yml`, update landing page/README, submit to `obsidian-releases` | fresh user pairs and syncs from the plugin catalog |

## 10. Conformance strategy

Keep `core/` as the single source of truth. Add a shared fixture set (sample
vaults + expected manifests/hashes) consumed by both `cargo test` and vitest,
so the TS port is always provably equivalent. Never diverge the
`resolve_divergence` logic.

## 11. Risks

- **Mobile server limitation** (addressed by HTTP transport) — the phone side
  can never be authoritative; must always be the additive client. Matches today.
- **`requestUrl` reachability** over hotspot — validated in Phase 0 spike.
- **Obsidian's settings/API surface changes** — plugin pins a min Obsidian
  version in `manifest.json`.
- **Vault adapter perf** — hashing every file on each scan is the same cost as
  today's `refresh_index`; incremental scan by mtime/size avoids re-reads.
- **Electron server port conflicts** — desktop plugin binds :42042; if taken,
  pick a free port and encode it in the pairing payload (today's config lives
  in `~/.obsync-approved.json`).

## 12. What stays in Rust (unchanged)

- `core/` — reference engine + test oracle.
- `obsync-httpd` — headless/NAS/Raspberry Pi path, real self-hosting.
- `cli/` — scripting/testing tool.
- `.github/workflows/` — CI still gates `cargo fmt/clippy/test`; release matrix
  drops Tauri + APK only after Phase 6.
