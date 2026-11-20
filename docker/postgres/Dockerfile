# Relating Postgres image: shared team metadata database.
#
# Build context: this folder.
#   docker build -f docker/postgres/Dockerfile -t relating-postgres docker/postgres
#
# The Relating table schema is NOT baked in here on purpose: it is provisioned
# idempotently at compose time by the `db-init` job, which runs the sanctioned
# `relating db init-postgres` path (see docs/postgres.md). That keeps one
# source of truth for the schema (crates/relating-metadata/migrations_pg/).
#
# What this image adds over stock postgres:17:
#   - small-team staging tuning applied on first init (initdb/10-tuning.sh)
#   - documented defaults (POSTGRES_USER/DB); the password always comes from
#     the environment, never from this file
FROM postgres:17-bookworm

# Apply tuning exactly once, when the data volume is first initialized.
COPY initdb/10-tuning.sh /docker-entrypoint-initdb.d/10-tuning.sh

ENV POSTGRES_USER=relating
ENV POSTGRES_DB=relating

EXPOSE 5432
