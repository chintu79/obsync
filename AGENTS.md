# Agent notes for Obsync

## Environment (developer machine)

- Fedora Linux, Java 25 → use Gradle wrapper 9.5.0, AGP 8.10.0, Kotlin 2.1.20, compileSdk 35.
- **Always** `export ANDROID_HOME=$HOME/Android/Sdk` before any Gradle invocation.
- `cargo-ndk v4.1.2` + Rust Android targets (`aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android`) are installed.
- NDK versions available: `26.1.10909125` (used by CI), `28.2.13676358`, `30.0.14904198`.
- `core/src/android.rs` is `#[cfg(target_os = "android")]` — host `cargo check`/`cargo test` do NOT compile or validate it. The Android Rust library only builds via `cargo ndk` (the Gradle `buildRust` task).
- Rust workspace members: `core`, `cli`, `httpd`. The `desktop/` Tauri app has its own Cargo.toml and is NOT a workspace member.

## Phone (testing)

- NARZO 70 Pro 5G, `adb` serial `IVHEDMV4EEQGY5JN`, Android 16 (SDK 36), app targetSdk 34.
- Aggressive background-app killing; floating WhatsApp/Instagram windows steal focus and hijack `uiautomator dump`. Run `adb shell am force-stop com.whatsapp` before UI tests.
- The phone's hotspot is the network bridge to the laptop: laptop LAN IP `10.174.223.140`, phone reaches it on the same `10.174.223.x/24`. Sync port `42042`, web dashboard `42021`.
- Vault on phone: `/storage/emulated/0/Documents/Obsidian`. Vault on laptop (dev): `/tmp/opencode/vaultA`.
- If `adb` drops (USB unplugged), the phone still syncs over its hotspot — check `httpd` logs for handshakes rather than assuming the device is offline.

## Server behavior

- `obsync-httpd` serves the dashboard (port 42021, embedded `webui.html`) and the P2P sync TCP server (port 42042). It keeps the selected vault and approved devices **in memory** — restarting the daemon loses the vault selection (re-select via `POST /api/select-vault`) but approvals persist in `~/.obsync-approved.json`.
- The sync server calls `engine.refresh_index(true)` before every session so files created/edited directly on the laptop disk reach the phone. Do NOT revert to a DB-only manifest.
- `refresh_index(detect_deletions)` gates auto-tombstoning: the laptop server is authoritative and passes `true`; the phone client passes `false` (additive-only) because its disk may be an incomplete replica — a phantom tombstone from the phone can delete files on the authoritative vault (see `core/src/android.rs` `syncOnce`).
- Conflict model: revisions are per-engine local counters, so "revision > 0 on both sides" must NEVER be treated as "both changed since last sync" (any hash difference would be a permanent false conflict). The correct signal is `file_states.synced_hash` (the content hash the last sync agreed on, v2 schema column): conflict only when BOTH sides changed since their agreement (`core/src/conflict/detector.rs` `resolve_divergence`); rows with NULL `synced_hash` (pre-v2) fall back to newer-mtime-wins. `record_remote_file`, `mark_synced`, and `apply_update` set `synced_hash`; `refresh_index` preserves it across local edits. The client diff (`sync/peer.rs` step 3) is the single decision point — the server accepts pushes unless it has unsynced local edits.
- `run-server.sh` builds + starts the server and opens the dashboard (release binary unless `--debug`).
- CI: `.github/workflows/ci.yml` (push/PR checks), `.github/workflows/release.yml` (tag `v*` → APK + Linux/macOS/Windows server binaries + GitHub Release).
- KNOWN FLAKY: `core/src/sync/transfer.rs` `test_send_receive_small_file` failed intermittently (~50%) at `--test-threads=32` with `dest.md` empty on disk; never reproduced in 27+ runs after debug instrumentation was added then removed (Heisenbug, unresolved). If it resurfaces in CI, debug recipe: `eprintln!` of `received`/`size_on_disk` in `receive_file` after `commit()`, and the "temp already exists" print in `filesystem/atomic.rs` `AtomicWriter::new`.
