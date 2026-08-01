# Contributing to Obsync

Thanks for your interest! Obsync is a local-first, peer-to-peer sync tool for
Obsidian vaults. Everything runs on your own network — no cloud, no accounts.

## Code of Conduct

This project is governed by the [Contributor Covenant](CODE_OF_CONDUCT.md).
By participating you agree to abide by its terms.

## Development Setup

### Prerequisites

| Tool        | Version          | Notes                                             |
| ----------- | ---------------- | ------------------------------------------------- |
| Rust        | stable (1.97+)   | [rustup](https://rustup.rs)                        |
| cargo-ndk   | 4.1.2            | `cargo install cargo-ndk --version 4.1.2 --locked` |
| Java (JDK)  | 17+ (25 tested)  | temurin recommended                               |
| Android SDK | API 35           | `ANDROID_HOME` must be set                        |
| NDK         | 26.1.10909125    | `sdkmanager "ndk;26.1.10909125"`                  |

The `desktop/` Tauri app is **not** part of the workspace and is
work-in-progress; the supported desktop path is the `httpd` web dashboard.

### Building the workspace

```bash
cargo build --workspace
cargo test --workspace
```

### Building the Android app

```bash
cd android
ANDROID_HOME=$HOME/Android/Sdk ./gradlew :app:assembleDebug
```

The Android app embeds the Rust core via JNI (`core/src/android.rs`). Host
`cargo check`/`cargo test` do **not** compile the Android-specific code — it
only builds through the Gradle `buildRust` task using `cargo ndk`.

### Running the server locally

```bash
./run-server.sh              # release build + opens the dashboard
./run-server.sh --debug      # debug build
# → http://localhost:42021
```

## Design Principles

- **Correctness over speed.** Data safety is the top priority. This tool moves
  people's notes — never risk their data for a faster path.
- **YAGNI.** Do not add features "just in case."
- **Small dependencies.** Every dependency must justify its cost.
- **Test all the things.** Especially sync convergence.
- **Simple over clever.** Prefer straightforward code.

## Code Style

- Rust: `cargo fmt`, `cargo clippy` — no warnings
- Kotlin: match existing style (ktlint where configured)
- No unnecessary comments. Code should be self-documenting where possible.
- Use meaningful names. Avoid abbreviations.

## Testing

### Running tests

```bash
# All tests
cargo test --workspace

# Randomized convergence tests
cargo test --test convergence -- --include-ignored
```

### What to test

All new code should include tests for:

- Normal operation paths
- Error conditions
- Edge cases (empty files, very large files, Unicode paths)
- Race conditions (concurrent file operations)
- State convergence (two peers must always end up identical)

> Known flaky test (historical): `core/src/sync/transfer.rs::test_send_receive_small_file`
> failed intermittently at `--test-threads=32` (never reproduced after
> instrumentation). The transfer module was removed in the dependency audit, so
> this only matters if a chunked-transfer path is ever reintroduced.

## Pull Request Process

1. Open an issue describing the change before working on it (unless trivial).
2. Implement the change with tests.
3. Ensure all tests pass and `cargo clippy` is clean.
4. Submit a PR with a clear description of what and why.

## Commit Messages

Follow conventional commits:

```
feat: add BLAKE3 streaming hash for large files
fix: handle edge case when temp file already exists
docs: update pairing protocol documentation
perf: reduce allocations in manifest comparison
test: add convergence test for rename conflict
```

## Release Process

1. Bump the workspace version in `Cargo.toml` and the Android app version.
2. Tag a release: `git tag vX.Y.Z` and push — CI builds the APK and server
   binaries automatically (see `.github/workflows/release.yml`).
3. Attach release notes on the GitHub Release.
