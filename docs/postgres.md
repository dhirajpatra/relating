# PostgreSQL Metadata — Team Runbook

SQLite remains the default local metadata store and is fully supported.
PostgreSQL is for small teams (3–5 engineers) sharing one staging instance
instead of passing SQLite files around.

## Current scope (read this first)

In this release PostgreSQL support is **provisioning + validation**:

- `relating db init-postgres --url <pg-url>` creates the schema idempotently
  (safe to re-run; existing tables are left untouched).
- `relating db check-postgres --url <pg-url>` validates reachability and
  reports which Relating tables exist (read-only).
- The API server still runs on SQLite: pointing `RELATING_DATABASE_URL` at
  Postgres fails fast with a message explaining the cutover status instead
  of a cryptic SQLite error.

Both commands need `--features postgres`:

```bash
cargo run -p relating-cli --features postgres -- \
  db init-postgres --url postgres://relating@db.internal:5432/relating
cargo run -p relating-cli --features postgres -- \
  db check-postgres --url postgres://relating@db.internal:5432/relating
```

Exit codes: 0 when the schema is ready, 1 when tables are missing (check) or a
gate fails, 2 for usage/config errors. `--format json` prints the full report.

## Schema

`crates/relating-metadata/migrations_pg/schema.sql` ports SQLite migrations
`0001`–`0005` 1:1 (type adjustments only: `BLOB` → `BYTEA`,
`REAL` → `DOUBLE PRECISION`). Tables: `projects`, `jobs`, `sources`,
`ontologies`, `embedding_profiles`, `chunks`, `evaluations`,
`aura_connections`, `migrations`, `schema_revisions`, `retrieval_configs`.

## Suggested team setup

1. Provision one Postgres database per shared environment (staging, prod).
2. Run `db init-postgres` once per environment; add `db check-postgres` to
   deploy health checks.
3. Keep developer laptops on SQLite; share staging through the server.
4. Store the Postgres URL and Aura passwords per `docs/secrets.md`
   (`*_FILE` sidecars or the OS keychain — never in Git).

## Roadmap to runtime cutover

Flipping the server's repository layer to Postgres touches every query and is
deliberately **not** in this change (it must not affect the running
application). The cutover is a separate slice: introduce a backend-agnostic
repository trait, port placeholders per dialect, dual-run SQLite vs Postgres
in CI, then switch `RELATING_DATABASE_URL` handling. The provisioned schema
here is that slice's starting point.
