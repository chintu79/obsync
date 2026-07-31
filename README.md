# Obsync

**Obsync** is a local-first, peer-to-peer file sync tool for keeping your Obsidian
vault in sync between your laptop and your phone — no cloud, no accounts, no data
leaving your devices. Everything runs on your own network over an encrypted direct
connection.

The sync engine is written in **Rust** (with BLAKE3 hashing, SQLite state tracking,
and per-file conflict detection) and is embedded in an **Android** client
(Kotlin/Jetpack Compose via JNI) and a small **web dashboard** that runs on your
laptop.

> **Status:** alpha. Works end-to-end for a single laptop ⇄ phone vault. Expect
> rough edges.

---

## Features

- **No cloud** — devices talk directly over your LAN (TCP, with key fingerprints for pairing).
- **QR pairing** — scan a QR code on your phone to connect to your laptop's dashboard.
- **Continuous background sync** — the Android app syncs every 30 s while running.
- **Conflict detection** — files edited on both sides are flagged, not clobbered.
- **P2P transport** — encrypted sync protocol on a dedicated TCP port.

---

## Quick start (end users)

### 1. Download the Android app

Grab the latest **`obsync-vX.Y.Z.apk`** from the
[Releases page](https://github.com/YOUR_USERNAME/Obsync/releases), then:

1. Copy the APK to your Android phone (or download it directly on the phone).
2. Open it and allow **"Install unknown apps"** for your browser / file manager.
3. Install, then open **Obsync**.
4. Grant the **"All files access"** permission when prompted (required on Android 11+).

### 2. Run the server on your laptop

**Pre-built binary** (from the same Releases page) — run it and it prints the
dashboard URL:

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

## Project layout

```
core/    Rust sync engine: indexer, store, engine, P2P network, security
httpd/   Web dashboard server (axum) — the "desktop" side users run on their laptop
cli/     Command-line tool for sync/testing
android/ Android app (Kotlin + Jetpack Compose, embeds the core via JNI)
desktop/ Tauri desktop app (React) — work-in-progress alternative to httpd
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design and
[CONTRIBUTING.md](CONTRIBUTING.md) for development workflow.

---

## Releases

Tag a release and CI builds everything automatically:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The [release workflow](.github/workflows/release.yml) produces:

- **Android APK** for all supported ABIs (`arm64-v8a`, `armeabi-v7a`, `x86_64`)
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

## License

[MIT](LICENSE) © 2026 Obsync contributors.
