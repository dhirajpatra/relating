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
# RELATING_DATABASE_URL, RELATING_LOG_LEVEL, RELATING_MCP_TOKEN,
# RELATING_API_TOKEN, RELATING_SECRET_BACKEND, AURA_URI, AURA_PASSWORD,
# LLM_BACKEND, META_API_KEY.
# Any of these can be set in a `.env` file in this directory (see
# .env.example) instead of exporting them.
#
# Platform: bash-only (Linux/macOS). On Windows, run it from WSL2 or
# Git Bash with Rust, Node.js 20+, and cargo on PATH; native PowerShell
# is not supported (see README "Windows" notes).
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Load $ROOT/.env when present. Variables already exported in the environment
# win; the file only fills in what is unset. Parsed safely without executing
# anything: blank lines, `#` comments, `export KEY=value`, and single/double
# quotes are supported.
load_dotenv() {
    local file="$ROOT/.env" line key value
    [[ -f "$file" ]] || return 0
    while IFS= read -r line || [[ -n "$line" ]]; do
        line="${line%$'\r'}"
        line="${line#"${line%%[![:space:]]*}"}"
        line="${line%"${line##*[![:space:]]}"}"
        [[ -z "$line" || "$line" == \#* ]] && continue
        [[ "$line" == export\ * ]] && line="${line#export }"
        if [[ "$line" != *=* ]]; then
            echo "Ignoring bad line in .env: $line" >&2
            continue
        fi
        key="${line%%=*}"
        value="${line#*=}"
        key="${key%"${key##*[![:space:]]}"}"
        if [[ "$value" == \"* ]]; then
            value="${value#\"}"
            value="${value%%\"*}"
        elif [[ "$value" == \'* ]]; then
            value="${value#\'}"
            value="${value%%\'*}"
        else
            value="${value%%#*}"
            value="${value%"${value##*[![:space:]]}"}"
        fi
        if [[ ! "$key" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
            echo "Ignoring bad variable name in .env: $key" >&2
            continue
        fi
        if [[ -z "${!key+x}" ]]; then
            printf -v "$key" '%s' "$value"
            export "$key"
        fi
    done < "$file"
    echo "==> Loaded environment from $file (already-exported variables take precedence)."
}
load_dotenv

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
RELATING_DATABASE_URL, RELATING_LOG_LEVEL, RELATING_MCP_TOKEN,
RELATING_API_TOKEN, RELATING_SECRET_BACKEND, AURA_URI, AURA_PASSWORD,
LLM_BACKEND, META_API_KEY.

A `.env` file next to this script (see .env.example) is loaded automatically;
already-exported variables take precedence over it.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --port)
            if [[ $# -lt 2 || -z "${2:-}" ]]; then
                echo "--port needs a value" >&2
                usage >&2
                exit 1
            fi
            PORT="$2"; shift 2 ;;
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

node_major="$(node -p "process.versions.node.split('.')[0]" 2>/dev/null || echo 0)"
if [[ "$node_major" -lt 20 ]]; then
    echo "Warning: Node.js 20+ is required, found $(node --version 2>/dev/null || echo unknown). The web build may fail." >&2
fi

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
if [[ -n "${RELATING_MCP_TOKEN:-}" ]]; then
    echo "    MCP: stdio + HTTP transport enabled"
else
    echo "    MCP: stdio only (set RELATING_MCP_TOKEN in .env to enable the HTTP transport)"
fi
if [[ "$RELEASE" -eq 1 ]]; then
    echo "==> Building release binary (first run takes a few minutes)..."
    cd "$ROOT" && exec cargo run --quiet --release -p relating-server
else
    cd "$ROOT" && exec cargo run --quiet -p relating-server
fi
