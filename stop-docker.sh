#!/usr/bin/env bash
#
# Relating — stop the container stack started by ./start-with-docker.sh.
#
# Usage:
#   ./stop-docker.sh [--volumes] [-h|--help]
#
# By default this runs `docker compose down`, which stops and removes the
# containers but KEEPS all data volumes (your projects are safe).
# Pass --volumes to also delete the volumes (project data, Postgres data,
# Ollama models, Neo4j data). That cannot be undone.
#
# Platform: bash-only (Linux/macOS). On Windows, run it from WSL2 or
# Git Bash; native PowerShell is not supported.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

VOLUMES=0

usage() {
    cat <<'EOF'
Relating — stop the container stack started by ./start-with-docker.sh.

Usage:
  ./stop-docker.sh [--volumes] [-h|--help]

Without flags this runs `docker compose down`: containers stop and are
removed, data volumes (your projects) are kept. --volumes also deletes
the volumes, which cannot be undone.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --volumes) VOLUMES=1; shift ;;
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

if [[ "$VOLUMES" -eq 1 ]]; then
    # Fail closed when stdin is not a terminal (piped CI input refuses).
    if [[ ! -t 0 ]]; then
        echo "Refusing --volumes without an interactive terminal (stdin is not a TTY)." >&2
        exit 1
    fi
    echo "This deletes ALL compose volumes: project data, Postgres data, Ollama models, Neo4j data." >&2
    read -r -p "Delete all Relating volumes? [y/N] " answer
    if [[ "${answer,,}" != "y" && "${answer,,}" != "yes" ]]; then
        echo "Aborted: volumes kept. Run without --volumes to stop while keeping data."
        exit 1
    fi
    echo "==> Stopping containers and deleting volumes..."
    exec docker compose down --volumes
fi

echo "==> Stopping containers (volumes kept)..."
exec docker compose down
