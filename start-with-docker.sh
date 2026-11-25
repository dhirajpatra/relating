#!/usr/bin/env bash
#
# Relating — start the container stack (API + web workbench) with one command.
#
# Usage:
#   ./start-with-docker.sh [options]
#
# Options:
#   --profile NAME   Enable a compose profile (repeatable):
#                    postgres, ollama, neo4j
#   --no-build       Skip `docker compose build` (reuse existing images)
#   --detach         Run in the background instead of the foreground
#   -h, --help       Show this help
#
# This is the container alternative to ./start.sh: same workbench, but
# project data lives in the `relating-data` volume instead of ./data.
# Stop it with ./stop-docker.sh (or Ctrl+C when running in the foreground).
#
# Platform: bash-only (Linux/macOS). On Windows, run it from WSL2 or
# Git Bash with Docker Desktop (compose v2) on PATH; native PowerShell
# is not supported.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

PROFILES=()
BUILD=1
DETACH=0

usage() {
    cat <<'EOF'
Relating — start the container stack (API + web workbench) with one command.

Usage:
  ./start-with-docker.sh [options]

Options:
  --profile NAME   Enable a compose profile (repeatable):
                   postgres, ollama, neo4j
  --no-build       Skip `docker compose build` (reuse existing images)
  --detach         Run in the background instead of the foreground
  -h, --help       Show this help

Project data lives in the `relating-data` volume (not ./data).
Stop with ./stop-docker.sh (or Ctrl+C in the foreground).
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --profile)
            if [[ $# -lt 2 || -z "${2:-}" ]]; then
                echo "--profile needs a value" >&2
                usage >&2
                exit 1
            fi
            case "$2" in
                postgres|ollama|neo4j) PROFILES+=("$2"); shift 2 ;;
                *)
                    echo "Unknown profile: $2 (expected postgres, ollama, or neo4j)" >&2
                    exit 1
                    ;;
            esac
            ;;
        --no-build) BUILD=0; shift ;;
        --detach) DETACH=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown option: $1" >&2; usage >&2; exit 1 ;;
    esac
done

if ! command -v docker >/dev/null 2>&1; then
    echo "Missing required tool: docker (install Docker Desktop or the Docker Engine)" >&2
    exit 1
fi
if ! docker compose version >/dev/null 2>&1; then
    echo "Missing required tool: docker compose v2 (update Docker Desktop or install the compose plugin)" >&2
    exit 1
fi

ARGS=(up)
if [[ "$BUILD" -eq 1 ]]; then
    ARGS+=(--build)
fi
if [[ "$DETACH" -eq 1 ]]; then
    ARGS+=(-d)
fi
if [[ "${#PROFILES[@]}" -gt 0 ]]; then
    for profile in "${PROFILES[@]}"; do
        ARGS+=(--profile "$profile")
    done
fi

PORT="${RELATING_PORT:-8080}"
echo "==> Starting Relating containers at http://127.0.0.1:${PORT}"
echo "    Open that URL in your browser to use the workbench."
echo "    Project data lives in the 'relating-data' volume (not ./data)."
echo "    Stop with ./stop-docker.sh."
exec docker compose "${ARGS[@]}"
