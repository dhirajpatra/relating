#!/usr/bin/env bash
#
# Relating — start the whole application (API + web workbench) with one command.
#
# Usage:
#   ./start.sh [options]
#
# Options:
#   --port PORT        Listen port (default: 8080, or $RELATING_PORT)
#   --release          Build and run the optimized release binary
#   --rebuild-web      Force a rebuild of the web UI
#   --skip-web-build   Never touch the web UI build
#   -h, --help         Show this help
#
# Environment overrides: RELATING_PORT, RELATING_HOST, RELATING_DATA_DIR,
# RELATING_DATABASE_URL, RELATING_LOG_LEVEL.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PORT="${RELATING_PORT:-8080}"
HOST="${RELATING_HOST:-127.0.0.1}"
DATA_DIR="${RELATING_DATA_DIR:-$ROOT/data}"
DB_URL="${RELATING_DATABASE_URL:-sqlite://$DATA_DIR/relating_metadata.db}"
LOG_LEVEL="${RELATING_LOG_LEVEL:-info}"
RELEASE=0
REBUILD_WEB=0
SKIP_WEB_BUILD=0

usage() {
    cat <<'EOF'
Relating — start the whole application (API + web workbench) with one command.

Usage:
  ./start.sh [options]

Options:
  --port PORT        Listen port (default: 8080, or $RELATING_PORT)
  --release          Build and run the optimized release binary
  --rebuild-web      Force a rebuild of the web UI
  --skip-web-build   Never touch the web UI build
  -h, --help         Show this help

Environment overrides: RELATING_PORT, RELATING_HOST, RELATING_DATA_DIR,
RELATING_DATABASE_URL, RELATING_LOG_LEVEL.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --port) PORT="$2"; shift 2 ;;
        --release) RELEASE=1; shift ;;
        --rebuild-web) REBUILD_WEB=1; shift ;;
        --skip-web-build) SKIP_WEB_BUILD=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown option: $1" >&2; usage >&2; exit 1 ;;
    esac
done

need() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "Missing required tool: $1 ($2)" >&2
        exit 1
    fi
}
need cargo "install Rust via https://rustup.rs"
need node "install Node.js 20+"
need npm "install npm (bundled with Node.js)"

if [[ "$REBUILD_WEB" -eq 1 && "$SKIP_WEB_BUILD" -eq 1 ]]; then
    echo "--rebuild-web and --skip-web-build cannot be combined" >&2
    exit 1
fi

# Fail fast with a clear message when the port is already taken.
if (exec 3<>"/dev/tcp/$HOST/$PORT") 2>/dev/null; then
    exec 3>&- 3<&-
    echo "Port $PORT on $HOST is already in use. Stop that process or pick another port:" >&2
    echo "  ./start.sh --port 8081" >&2
    exit 1
fi

mkdir -p "$DATA_DIR"

if [[ "$SKIP_WEB_BUILD" -eq 0 ]]; then
    if [[ "$REBUILD_WEB" -eq 1 || ! -f "$ROOT/web/dist/index.html" ]]; then
        echo "==> Building web UI..."
        if [[ ! -d "$ROOT/web/node_modules" ]]; then
            echo "==> Installing web dependencies..."
            (cd "$ROOT/web" && npm install)
        fi
        (cd "$ROOT/web" && npm run build)
    else
        echo "==> Web UI build found, skipping (use --rebuild-web to force)."
    fi
fi

# Non-fatal hint about the optional local AI backend.
if command -v curl >/dev/null 2>&1; then
    if ! curl -s -m 2 http://localhost:11434/api/tags >/dev/null 2>&1; then
        echo "==> Note: no Ollama server detected at localhost:11434."
        echo "    The app still works with the built-in mock provider; start Ollama"
        echo "    ('ollama serve') to use local models, then create a project with"
        echo "    provider 'ollama'."
    fi
fi

export RELATING_PORT="$PORT"
export RELATING_HOST="$HOST"
export RELATING_DATA_DIR="$DATA_DIR"
export RELATING_DATABASE_URL="$DB_URL"
export RELATING_LOG_LEVEL="$LOG_LEVEL"

echo "==> Starting Relating at http://$HOST:$PORT"
echo "    Open that URL in your browser to use the workbench."
echo "    API base: http://$HOST:$PORT/api/v1  |  Data: $DATA_DIR"
if [[ "$RELEASE" -eq 1 ]]; then
    echo "==> Building release binary (first run takes a few minutes)..."
    cd "$ROOT" && exec cargo run --quiet --release -p relating-server
else
    cd "$ROOT" && exec cargo run --quiet -p relating-server
fi
