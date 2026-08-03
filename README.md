<div align="center">

# Obsync

**Free, local-first P2P sync for your Obsidian vault. No cloud. No account. No subscription.**

Sync your Obsidian vault between your laptop and your phone over your own
network — encrypted, direct, and private. Your notes never touch a third-party
server.

![CI](https://github.com/chintu79/obsync/workflows/CI/badge.svg)
![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)
![Status: alpha](https://img.shields.io/badge/status-alpha-orange)

</div>

> **New:** Obsync is now an **Obsidian plugin** — install it from the
> community catalog and sync your vault between desktop and mobile without any
> separate apps. See the [obsync-plugin](https://github.com/chintu79/obsync-plugin)
> repo. The Rust core, dashboard, and Android app below remain the
> self-hosted/NAS path.

---

## Why Obsync?

Obsidian Sync costs **$4/month** and sends your vault through Obsidian's cloud.
Obsync is an alternative that keeps your notes on your devices:

- **No cloud, no accounts** — devices talk directly over your LAN (TCP, with key fingerprints for pairing).
- **Encrypted P2P transport** — a dedicated sync protocol on your own network.
- **QR pairing** — scan a code on the dashboard from your phone and you're connected.
- **Continuous background sync** — the Android app syncs every 30 s while running.
- **Conflict detection** — files edited on both sides are flagged, never silently clobbered.
- **Local-first** — your vault stays an ordinary folder on disk. No lock-in; leave anytime.

> **Status:** alpha. Works end-to-end for a single laptop ⇄ phone vault. Expect rough edges.

---

## Quick start

### 1. Download the Android app

Grab the latest **`obsync-vX.Y.Z.apk`** from the
[Releases page](https://github.com/chintu79/obsync/releases), then:

1. Copy the APK to your Android phone (or download it directly on the phone).
2. Open it and allow **"Install unknown apps"** for your browser / file manager.
3. Install, then open **Obsync**.
4. Grant the **"All files access"** permission when prompted (required on Android 11+).

### 2. Run the server on your laptop

**Desktop app (recommended, no terminal needed)** — grab the installer for
your OS from the [Releases page](https://github.com/chintu79/obsync/releases)
and double-click it. Obsync starts itself and opens the dashboard.

**Or use the website:** visit the [Obsync home page](https://chintu79.github.io/obsync/)
for one-click download links per platform.

**Headless / advanced users** can run the server binary directly from the
Releases page:

```bash
./obsync-server-linux-x86_64        # Linux
./obsync-server-macos-aarch64       # Apple Silicon Mac
./obsync-server-macos-x86_64        # Intel Mac
./obsync-server-windows-x86_64.exe  # Windows
```

**From source** (requires Rust):

```bash
./run-server.sh
```

Either way, open **http://localhost:42021** in your browser.

### 3. Pair and sync

1. On the dashboard, click **Select vault** and pick the Obsidian vault folder on your laptop.
2. Click **Pair with phone** — a QR code appears.
3. In the Android app, tap **Scan QR** and point it at the code.
4. Approve the device on the dashboard when prompted.
5. The phone pulls the vault — new files on either side now stay in sync.

> Make sure both devices are on the same network (the phone should reach
> `http://<laptop-ip>:42021`).

---

## Features

- **Pairing wizard** on first run — scan a QR code, approve the device, done.
- **Live dashboard** — file counts, sync activity, online/offline device status,
  conflict list with resolution (keep local / remote / both), and snapshot
  restore for every file.
- **Snapshot versioning** — every edit is snapshotted on the server so you can
  roll back.
- **Conflict resolution** — diverged edits are surfaced as conflicts; resolve
  per-file from the dashboard.
- **Headless-friendly server** — the `obsync-httpd` daemon runs anywhere on your
  network (laptop, NAS, Raspberry Pi) and exposes a REST + SSE API for scripts
  and other UIs.

## Project layout

```
core/    Rust sync engine: indexer, store, engine, P2P network, security
httpd/   Web dashboard server (axum) — lib + thin binary; the "desktop" side
         users run on their laptop
cli/     Command-line tool for sync/testing
android/ Android app (Kotlin + Jetpack Compose, embeds the core via JNI)
desktop/ Tauri v2 desktop app — embeds httpd and opens the dashboard in a
         window (the no-terminal release path)
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design and
[CONTRIBUTING.md](CONTRIBUTING.md) for development workflow.

---

## Building from source

### Prerequisites

| Tool        | Version          | Notes                                             |
| ----------- | ---------------- | ------------------------------------------------- |
| Rust        | stable (1.97+)   | [rustup](https://rustup.rs)                        |
| cargo-ndk   | 4.1.2            | `cargo install cargo-ndk --version 4.1.2 --locked` |
| Java (JDK)  | 17+ (25 tested)  | temurin recommended                               |
| Android SDK | API 35           | `ANDROID_HOME` must be set                        |
| NDK         | 26.1.10909125    | `sdkmanager "ndk;26.1.10909125"`                  |

### Server (web dashboard)

```bash
cargo build --release -p obsync-httpd
./target/release/obsync-httpd
# → open http://localhost:42021
```

Or just use the launcher script: `./run-server.sh`.

### Android APK

```bash
cd android
ANDROID_HOME=$HOME/Android/Sdk ./gradlew :app:assembleDebug
# → android/app/build/outputs/apk/debug/app-debug.apk
```

For a release build:

```bash
cp android/app/keystore.properties.example android/app/keystore.properties
# edit with your keystore details (see file for how to generate one)
cd android && ANDROID_HOME=$HOME/Android/Sdk ./gradlew :app:assembleRelease
```

If `keystore.properties` is absent, release builds fall back to the debug
keystore so CI and local builds always succeed.

### CLI

A small CLI is included for scripting and testing:

```bash
cargo run -p obsync-cli -- --help
```

---

## Releases

Tag a release and CI builds everything automatically:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The [release workflow](.github/workflows/release.yml) produces:

- **Android APK** for all supported ABIs (`arm64-v8a`, `armeabi-v7a`, `x86_64`)
- **Desktop app installers** for Windows, macOS, and Linux (double-click to install)
- **Server binaries** for Linux, macOS (Apple Silicon + Intel), and Windows

…and attaches them to a GitHub Release.

> **Note:** Release APKs are signed with the debug keystore unless you configure
> release signing (see below), which is fine for personal use but not for
> public/Play-Store distribution.

### Release signing

1. Generate a keystore:
   ```bash
   keytool -genkey -v -keystore release.keystore -alias obsync \
     -keyalg RSA -keysize 2048 -validity 10000
   ```
2. Set CI secrets (from **Settings → Secrets and variables → Actions**):
   - `OBSYNC_KEYSTORE_BASE64` — `base64 < release.keystore`
   - `OBSYNC_KEYSTORE_PASSWORD`
   - `OBSYNC_KEY_ALIAS` (e.g. `obsync`)
   - `OBSYNC_KEY_PASSWORD`

---

## Roadmap

See [ROADMAP.md](ROADMAP.md) for the implementation roadmap and
[DESIGN.md](DESIGN.md) for design decisions.

## Contributing

We welcome contributions of all kinds — code, docs, bug reports, feature ideas.
See [CONTRIBUTING.md](CONTRIBUTING.md) to get started, and read the
[Code of Conduct](CODE_OF_CONDUCT.md).

Found a security issue? Report it privately — see [SECURITY.md](SECURITY.md).

## Sponsors

Obsync is developed in the open and funded by its users. If it saves you
$4/month, consider sponsoring — every little bit keeps the project alive:

<p align="center">
  <a href="https://github.com/sponsors/chintu79">Sponsor on GitHub</a>
</p>

---

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE](LICENSE))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in Obsync shall be dual-licensed as above, without any additional
terms or conditions.

© 2026 Obsync contributors.
