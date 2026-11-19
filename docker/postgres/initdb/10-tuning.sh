#!/bin/sh
# Runs once via /docker-entrypoint-initdb.d on first volume init.
# Appends small-team staging tuning to the fresh postgresql.conf.
set -eu

cat >> "$PGDATA/postgresql.conf" <<'EOF'

# --- Relating staging tuning (small team, 3-5 engineers) ---
shared_buffers = 256MB
max_connections = 100
log_min_duration_statement = 1000
log_connections = on
log_disconnections = on
EOF

echo "Relating Postgres tuning applied to $PGDATA/postgresql.conf"
