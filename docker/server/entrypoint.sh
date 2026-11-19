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
    # Authentication failures are NOT retried: they mean the password in
    # use differs from the one the pgdata volume was initialized with
    # (Postgres ignores POSTGRES_PASSWORD on existing volumes).
    ATTEMPTS="${DB_INIT_ATTEMPTS:-30}"
    i=1
    while [ "$i" -le "$ATTEMPTS" ]; do
        if out=$(relating db init-postgres --url "$PG_URL" 2>&1); then
            echo "$out"
            echo "db-init: schema ready, validating..."
            exec relating db check-postgres --url "$PG_URL"
        fi
        echo "$out" >&2
        case "$out" in
            *"authentication failed"*)
                echo "db-init: password rejected; fix with:" >&2
                echo "  docker compose exec postgres psql -U postgres -c \"ALTER USER ${POSTGRES_USER:-relating} PASSWORD '<password-from-.env>';\"" >&2
                echo "or reset the volume (deletes all stack data): docker compose down -v && docker compose up" >&2
                exit 1
                ;;
        esac
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
