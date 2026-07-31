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
- The sync server calls `engine.refresh_index()` before every session so files created/edited directly on the laptop disk reach the phone. Do NOT revert to a DB-only manifest.
- `run-server.sh` builds + starts the server and opens the dashboard (release binary unless `--debug`).
- CI: `.github/workflows/ci.yml` (push/PR checks), `.github/workflows/release.yml` (tag `v*` → APK + Linux/macOS/Windows server binaries + GitHub Release).
