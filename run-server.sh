#!/usr/bin/env bash
#
# Obsync — one-command server launcher.
#
# Builds the sync server (if needed) and starts it, then opens the pairing
# dashboard in your browser. The server exposes:
#   • http://localhost:42021 — the web dashboard (pair devices, pick a vault)
#   • 0.0.0.0:42042          — the P2P sync port your phone connects to
#
# Usage:
#   ./run-server.sh            # build + run (release binary)
#   ./run-server.sh --debug    # build + run the debug binary (faster to build)
#   ./run-server.sh --no-open  # don't open the browser automatically
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$ROOT/target/release/obsync-httpd"
DEBUG_BIN="$ROOT/target/debug/obsync-httpd"
OPEN_BROWSER=1
PROFILE=release

for arg in "$@"; do
    case "$arg" in
        --debug) PROFILE=debug ;;
        --no-open) OPEN_BROWSER=0 ;;
        -h|--help)
            awk '{ if ($0 ~ /^#!/) next; if ($0 ~ /^#/) { sub(/^# ?/, ""); print } else exit }' "$0"
            exit 0
            ;;
        *)
            echo "Unknown option: $arg" >&2
            exit 1
            ;;
    esac
done

cd "$ROOT"

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: Rust toolchain not found. Install it from https://rustup.rs" >&2
    exit 1
fi

# Rebuild when the binary is missing OR any source is newer than it —
# otherwise a stale binary silently serves the old dashboard/sync code.
needs_rebuild() {
    local bin="$1"
    [ ! -x "$bin" ] && return 0
    find core cli httpd Cargo.toml Cargo.lock -type f -newer "$bin" -print -quit 2>/dev/null | grep -q .
}

if [ "$PROFILE" = "release" ]; then
    if needs_rebuild "$BIN"; then
        echo "→ Building obsync-httpd (release)…"
        cargo build --release -p obsync-httpd
    fi
    SERVER="$BIN"
else
    if needs_rebuild "$DEBUG_BIN"; then
        echo "→ Building obsync-httpd (debug)…"
        cargo build -p obsync-httpd
    fi
    SERVER="$DEBUG_BIN"
fi

echo "→ Starting Obsync server…"
"$SERVER" &

SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null' EXIT INT TERM

# Wait for the dashboard to come up
for _ in $(seq 1 20); do
    if curl -fsS "http://localhost:42021/api/status" >/dev/null 2>&1; then
        break
    fi
    sleep 0.5
done

echo
echo "  Dashboard:  http://localhost:42021"
echo "  Sync port:  0.0.0.0:42042 (use this in the phone app)"
echo "  Press Ctrl+C to stop."
echo

if [ "$OPEN_BROWSER" = "1" ]; then
    command -v xdg-open >/dev/null 2>&1 && xdg-open "http://localhost:42021" >/dev/null 2>&1 &
fi

wait "$SERVER_PID"
