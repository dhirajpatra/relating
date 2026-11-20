# R1 — Aura Connection and Migration

Base path: `/api/v1`. Migration moves graph data from a project's local Relate
store to Neo4j/Aura over a built-in Bolt v4 client (TLS with verified
certificates for `+s` URIs). Application-code portability arrives with the
Portability Compiler (R3); R1 migrates data, schema objects, and contracts.

## Connections

- `POST /projects/{id}/connections/aura` — `{name, uri, username?, database?, auth_env_var?}`.
  The URI must be `bolt://`, `bolt+s://`, `neo4j://`, or `neo4j+s://` and must
  not embed credentials. Passwords are never stored: `auth_env_var` (default
  `AURA_PASSWORD`) names the environment variable read at runtime.
- `GET /projects/{id}/connections/aura`, `DELETE .../aura/{conn_id}`.
  The list also includes the `.env` connection when `AURA_URI` plus a
  resolvable `AURA_PASSWORD` are set (stable id derived from the URI; see
  `docs/secrets.md`). It cannot be deleted; unset `AURA_URI` to hide it.
- `POST .../aura/{conn_id}/test` — fresh TCP+TLS+Bolt handshake with auth;
  returns reachability, server agent, and Bolt version. Unreachable targets
  yield `502 AURA_UNREACHABLE`, bad credentials `502 AURA_AUTH_FAILED`.
  Accepts stored ids and the `.env` connection id alike.

## One-shot script

`./migrate-aura.sh` starts the Docker stack (`server + postgres + ollama`),
seeds the `aura-fill` demo graph when it is missing (24 employees → 42
nodes, 72 relationships), then runs the whole workflow below and verifies.
It aborts before approval when the dry run reports blockers. Options:
`--project ID`, `--connection CID`, `--fresh` (rebuild the demo graph),
`--no-build`, `--up-only` (start the stack without migrating),
`--down` (stop the stack afterwards).

## Workflow (approval-gated)

1. `POST /projects/{id}/migrations/analyze` — `{connection_id?, aura_tier?}`.
   Stateless: source findings plus target findings (or an honest unreachable
   note). `aura_tier` (preset id, `<N>gb`, `<N>mb`, `custom:<mb>`) enables
   per-tier vector RAM checks; without it, dimensions are still checked and
   RAM fit is reported as needing a tier.
2. `POST /projects/{id}/migrations/plan` — `{connection_id, batch_size?}`.
   Creates a migration (`planned`) with per-label/type batches, generated
   unique-constraint DDL, and an embedded dry-run.
3. `POST /projects/{id}/migrations/{mid}/dry-run` — re-checks the target
   read-only. Always reports `remote_writes: 0`.
4. `POST .../{mid}/approve` — `{approved: true|false}`. Refuses approval while
   dry-run blockers exist. No remote write happens before this step.
5. `POST .../{mid}/execute` — requires `approved` (or resumes `failed`/`paused`).
   Runs idempotent DDL, then `UNWIND`+`MERGE` batches on stable `uid` keys,
   persisting progress after every batch. Endpoints are matched, never
   fabricated; nodes without `uid` are counted and skipped.
6. `POST .../{mid}/verify` — per-label/type counts plus property sampling
   (subset semantics); sets `verified` only when everything matches.
7. `GET .../migrations`, `GET .../{mid}`, `GET .../{mid}/report` (manifest).

## Write mode (disclosed, not implied)

Every plan declares `write_mode: "merge_upsert"`, `destructive: false`, and a
`write_semantics` sentence. Execution runs idempotent `MERGE` upserts on
stable `uid` keys: it appends new nodes/relationships and updates matched
properties, and never deletes target data or wipes the instance. Delta pushes
(migrate, add local documents, migrate again) transfer only new/changed
records. No overwrite/full-refresh mode exists in this release, so an approved
migration cannot clobber live Aura data. The UI shows this banner on every
migration before Approve/Execute.

## Vector index fit (checked before batch writes)

Neo4j vector indexes require 1–4096 dimensions and a single vector per index.
Analysis reports `vector_index_fit` findings per embedding profile:

- Dimensions outside 1–4096 are `UNSUPPORTED` and become dry-run **blockers**,
  so approval is refused instead of failing mid-transfer on Aura.
- With `aura_tier`, estimated float32 vector bytes are checked against a
  conservative 25% of tier RAM: fits are `SUPPORTED`, tight fits need manual
  review, over-budget loads are `UNSUPPORTED` with remediation (larger tier,
  fewer vectors, smaller model).
- Multiple profiles get one `TRANSFORM` note: plan one `CREATE VECTOR INDEX`
  per profile on its own label/property.

Tier RAM figures are documented estimates — confirm against the Aura console
and measure index builds on staging. The Performance Advisor (R6) runs the
same checks headlessly via `vectors` + `aura_tier` (see `advisor.md`).

## Limits (stated, not silent)

- `neo4j://` URIs follow Bolt routing (ROUTE → writer) like official
  drivers, falling back to the direct connection when the target answers
  no routing table. `bolt://` URIs always connect directly.
- `+ssc` (self-signed) URIs are rejected; use trusted certificates.
- Execution runs inside the request in R1; very large graphs should be
  migrated in planned batches and resumed if interrupted.

## Live verification

Protocol conformance is covered by PackStream unit tests and an end-to-end
suite against an in-test fake Bolt server. A real-server check is available
as an ignored test:

```bash
NEO4J_URI=neo4j+s://<host> NEO4J_USER=neo4j NEO4J_PASSWORD=<pw> \
  cargo test -p relating-migrate -- --ignored
```
