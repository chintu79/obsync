# Contributing to Obsync

## Code of Conduct

Be respectful, constructive, and professional. We're building infrastructure, not ego.

## Development Setup

### Prerequisites

- Rust 1.75+ (stable)
- Node.js 20+ (for Tauri desktop UI)
- Android SDK 34+ (for Android builds)
- Tauri CLI (`cargo install tauri-cli`)

### Building the Core

```bash
cargo build --workspace
cargo test --workspace
```

### Building the Desktop App

```bash
cd desktop
npm install
cargo tauri dev
```

### Building for Android

```bash
cd android
./gradlew assembleDebug
```

## Design Principles

- **Correctness over speed.** Data safety is the top priority.
- **YAGNI.** Do not add features "just in case."
- **Small dependencies.** Every dependency must justify its cost.
- **Test all the things.** Especially sync convergence.
- **Simple over clever.** Prefer straightforward code.

## Code Style

- Rust: `cargo fmt`, `cargo clippy` — no warnings
- TypeScript: prettier + eslint
- Kotlin: ktlint
- No unnecessary comments. Code should be self-documenting where possible.
- Use meaningful names. Avoid abbreviations.

## Testing

### Running Tests

```bash
# All tests
cargo test --workspace

# Unit tests only
cargo test --lib

# Integration tests only
cargo test --test '*'

# Randomized convergence tests
cargo test --test convergence -- --include-ignored
```

### What to Test

All new code should include tests for:

- Normal operation paths
- Error conditions
- Edge cases (empty files, very large files, Unicode paths)
- Race conditions (concurrent file operations)
- State convergence (two peers must always end up identical)

## Pull Request Process

1. Open an issue describing the change before working on it (unless trivial).
2. Implement the change with tests.
3. Ensure all tests pass.
4. Run `cargo clippy` and fix all warnings.
5. Submit PR with a clear description of what and why.

## Commit Messages

Follow conventional commits:

```
feat: add BLAKE3 streaming hash for large files
fix: handle edge case when temp file already exists
docs: update pairing protocol documentation
perf: reduce allocations in manifest comparison
test: add convergence test for rename conflict
```

## Architecture Decisions

Significant architecture decisions should be documented in ADRs (Architecture Decision Records) under `docs/adr/`. Each ADR should include:

- **Title:** Brief description
- **Status:** Proposed, Accepted, Deprecated
- **Context:** Why this decision is needed
- **Decision:** What was decided
- **Consequences:** Trade-offs and implications

## Release Process

1. Update version in `core/Cargo.toml`, `desktop/src-tauri/Cargo.toml`, and `android/`
2. Update CHANGELOG.md
3. Create a signed tag: `git tag -s v1.0.0`
4. CI builds release artifacts
5. Publish to GitHub Releases
