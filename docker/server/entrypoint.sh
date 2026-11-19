#!/bin/sh
# Relating server entrypoint.
#
# Modes:
#   relating-server (default)  run the API + workbench UI
#   db-init                    provision the shared Postgres schema with the
#                              sanctioned `relating db init-postgres` path,
#                              then validate it with `db check-postgres`
#   anything else              exec'd as-is (e.g. `relating validate ...`)
set -eu

MODE="${1:-relating-server}"

# The SQLite runtime store and the per-project Relate graph stores live here.
DATA_DIR="${RELATING_DATA_DIR:-/app/data}"
mkdir -p "$DATA_DIR"

if [ "$MODE" = "db-init" ]; then
    PG_URL="${RELATING_POSTGRES_URL:-}"
    if [ -z "$PG_URL" ]; then
        echo "db-init: RELATING_POSTGRES_URL is not set" >&2
        exit 2
    fi
    # init-postgres is idempotent, so retrying until Postgres accepts
    # connections is safe. check-postgres is read-only validation.
    ATTEMPTS="${DB_INIT_ATTEMPTS:-30}"
    i=1
    while [ "$i" -le "$ATTEMPTS" ]; do
        if relating db init-postgres --url "$PG_URL"; then
            echo "db-init: schema ready, validating..."
            exec relating db check-postgres --url "$PG_URL"
        fi
        echo "db-init: Postgres not ready (attempt $i/$ATTEMPTS), retrying..." >&2
        i=$((i + 1))
        sleep 2
    done
    echo "db-init: Postgres never became ready" >&2
    exit 1
fi

if [ "$MODE" = "relating-server" ]; then
    exec relating-server
fi

exec "$@"
