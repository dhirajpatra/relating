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
- `POST .../aura/{conn_id}/test` — fresh TCP+TLS+Bolt handshake with auth;
  returns reachability, server agent, and Bolt version. Unreachable targets
  yield `502 AURA_UNREACHABLE`, bad credentials `502 AURA_AUTH_FAILED`.

## Workflow (approval-gated)

1. `POST /projects/{id}/migrations/analyze` — `{connection_id?}`. Stateless:
   source findings plus target findings (or an honest unreachable note).
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

## Limits (stated, not silent)

- `neo4j://` URIs connect directly; cluster routing is not implemented (works
  for single-instance Aura; use the direct address for clusters).
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
