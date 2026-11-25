# AGENTS.md — Contributor and Coding-Agent Conventions

Relating is a Rust workspace (16 crates) + React workbench for graph
development, GraphRAG, and Relate→Neo4j Aura migration. `README.md` is the
product doc, `SKILL.md` is the implementation contract — this file is the
day-to-day contributor cheat sheet: exact commands, layout, and gotchas.

## Commands (CI runs these)

```bash
cargo fmt --all -- --check                                  # formatting gate
cargo clippy --workspace --all-targets --locked -- -D warnings  # lint gate
cargo test --workspace --locked                             # full suite
cargo test -p relating-migrate                              # one crate
cargo test -p relating-server --test migration_integration_tests test_rollback  # one test
cd web && npm ci --no-audit --no-fund && npm run build     # workbench UI
```

Run and migrate:

```bash
./start.sh                        # API + UI on http://127.0.0.1:8080 (bash/WSL2/Git Bash)
docker compose up --build         # server alone (SQLite; enough to migrate)
docker compose --profile postgres --profile ollama --profile neo4j up  # full stack
./migrate-aura.sh --yes           # Docker stack + approval-gated Aura migration
cargo run -p relating-cli -- aura migrate --project ID --connection CID --yes --report out.json
cargo run -p relating-cli -- portability scan --dir . --emit sarif --out portability.sarif
```

## Layout

| Path | What lives there |
|---|---|
| `crates/relating-core/` | Domain types, secrets chain, `AURA_*` env config |
| `crates/relating-metadata/` | SQLite runtime store; Postgres provisioning (`--features postgres`) |
| `crates/relating-server/` | Axum API (`src/routes/`), `AppState`, integration tests |
| `crates/relating-graph/` | `GraphBackend` trait, Relate adapter, `relate://local` / `aura://` targets |
| `crates/relating-ai/` | Provider abstraction (mock/openai/ollama/meta) |
| `crates/relating-ingest/` | Rust-native parsers (CSV/JSON/text/PDF/DOCX), ontology, extraction, loading |
| `crates/relating-retrieval/` | Embeddings, hybrid retrieval, Q&A, eval |
| `crates/relating-mcp/` | 12 read-only MCP tools (no write tools yet), stdio + HTTP |
| `crates/relating-migrate/` | Bolt client, analyze/plan/execute/verify/rollback |
| `crates/relating-schema/` | Schema-as-code docs, diff, conformance |
| `crates/relating-portability/` | Repo scanner, rules, remediation, SARIF |
| `crates/relating-cli/` | `relating` binary: validate, aura, portability, eval, shadow, … |
| `crates/relating-shadow/` | Dual-backend replay + comparison |
| `crates/relating-debug/` | GraphRAG traces + diagnosis |
| `crates/relating-synth/` | Synthetic fixture generation |
| `crates/relating-advisor/` | Perf/cost rules, sizing, measurements |
| `web/` | React + TypeScript workbench (12 tabs), served from `web/dist` when built |
| `tests/fixtures/` | Deterministic seed fixtures (`aura-fill-ontology.json`) |
| `docs/api/` | Per-area API contracts — update with every endpoint change |
| `docker/` | One folder + Dockerfile per compose service |

## Rust conventions

- Workspace dependencies in root `Cargo.toml`; per-crate `thiserror` error
  types mapped to `{error: {code, message}}` bodies in `relating-server/src/error.rs`.
- Stored structs (`MigrationPlan`, progress, reports) must stay
  backward-compatible: `#[serde(default)]` on every new field, plus a
  legacy-JSON deserialization test (see `plan.rs`).
- `MigrationStatus` serializes as `snake_case` over JSON and the DB
  (`rolled_back` included); match sites must handle every variant.
- Never interpolate values into Cypher. Reuse the backtick escapers
  (`escape_identifier`, `escape_cypher_label`); `SKIP`/`LIMIT` may only use
  internally computed integers.
- Migration writes are `MERGE` on stable keys, never `CREATE`: nodes on
  `uid`, relationships on their own `relationship_uid`. Rows without keys
  fail fast — never merge unpredictably.
- Created records are stamped `ON CREATE` with
  `MIGRATION_TAG_PROPERTY`; user properties of the same name are filtered
  out before writing. Rollback deletes by that tag only.
- API auth reads the token from `AppState.api_token` (populated from
  `RELATING_API_TOKEN` at startup), never from env per-request, so tests
  stay hermetic. `/api/v1/health` is always open; everything else under
  `/api/v1/` needs `Authorization: Bearer <token>` when set.

## Testing conventions

- Unit tests live in the module (`#[cfg(test)]`). Server behavior is
  covered by integration tests (`crates/relating-server/tests/`) using
  `oneshot` requests, `tempdir` data dirs, and in-memory SQLite.
- Bolt-level tests use the fake server in `tests/common/mod.rs` (real TCP,
  MERGE semantics). It supports uid listing, tags, deletes, and fault
  injection via `fail_write_at` — use it to prove resume/rollback.
- Process env is global: tests that mutate it serialize via an `ENV_LOCK`
  (`std::sync::Mutex` for sync tests, `tokio::sync::Mutex` for async
  tests — never hold a std guard across `.await`).
- CLI tests spawn the real binary: run the child with `cwd` inside the
  fixture dir and strip `LLM_BACKEND`/`META_API_KEY`/`MODEL_API_KEY`, or
  dotenv's upward search finds the developer's real `.env` and reroutes
  the mock provider to the network.
- The golden path (seed → analyze → dry-run → approve → execute → verify
  → report) must stay covered end to end; label anything outside it
  experimental.

## Secrets and safety

- Secrets are never stored, logged, or returned: `resolve_secret` reads
  `$NAME`, then `$NAME_FILE`, then the OS keychain (feature-gated).
  Errors name the variable, never the value.
- `.env.example` holds placeholders only. Never commit `.env`, `.secrets/`,
  `data/`, or `*-report.json` (all ignored).
- Compose binds every published port to `127.0.0.1`; the server runs as
  non-root; base images are minor-pinned (bump deliberately, then rebuild).
- `db-init` builds the Postgres URL from parts with percent-encoding, so
  any characters are safe in `POSTGRES_PASSWORD`. Prefer an explicit
  pre-encoded `RELATING_POSTGRES_URL` only when you must.

## Docs to update with code

- New/changed endpoint → `docs/api/<area>.md` (see `migration.md`,
  `portability.md`, `ci.md` for the pattern).
- New crate, tab, script, or workflow → `README.md` layout/status sections.
- New product guarantee or rule → `SKILL.md` (contract) and this file.
- Deterministic seeds go through `POST .../ontology/import` plus a fixture
  under `tests/fixtures/` — never through the LLM propose path.

## Gotchas

- `dotenvy` walks up from the working directory: binaries run from the repo
  root pick up the real `.env`; tests must isolate (see above).
- `web/node_modules` is historically tracked — do not touch it; `npm ci`
  output must not leak into commits.
- `web/dist` is optional at runtime (API-only mode when absent) but is
  baked into the server image at build time.
- Aura Free has hard caps (~200k nodes / ~400k relationships, estimates);
  always plan with `--aura-tier` for capacity pre-flight.
- `relate://local` vs `aura://<connection-id>` is the entire backend flip:
  same statement, same bound parameters, different target.
