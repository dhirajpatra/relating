# Backend Flip — Query Relate and Aura Without Rewriting Queries

Application code (Python/Node/Rust) talks to one shaped Cypher API. Switching
from the local Relate store to Aura after migration is a **target flip**, not
a query rewrite: the same statement text and bound parameters run on either
backend and return the identical result shape
(`{columns, rows, elapsed_ms, row_count}`).

## Connection strings

- Local Relate store: `relate://local` (the project's own store).
- Aura connection: `aura://<connection-id>` (a connection registered on the
  project via `POST /projects/{id}/connections/aura`).

`GET /projects/{id}/query-targets` lists the available targets for a project:

```json
{
  "project_id": "<uuid>",
  "local": {"backend": "relate", "connection_string": "relate://local"},
  "aura": [
    {"backend": "aura", "connection_string": "aura://<cid>",
     "connection_id": "<cid>", "name": "staging"}
  ],
  "usage": "..."
}
```

## Proxy mode (REST)

`POST /projects/{id}/cypher/read` accepts an optional `connection_id`:

```bash
# Before migration: runs on the local Relate store.
curl -X POST /api/v1/projects/$PID/cypher/read \
  -d '{"query": "MATCH (n:Person) RETURN n.name AS name", "params": {}}'

# After migration: the IDENTICAL statement, proxied to Aura.
curl -X POST /api/v1/projects/$PID/cypher/read \
  -d '{"query": "MATCH (n:Person) RETURN n.name AS name", "params": {},
       "connection_id": "<aura-connection-id>"}'
```

Reads keep the same default bound (100 rows unless the statement already has
`LIMIT`). Values stay bound parameters on both backends — nothing is
interpolated into the statement.

`POST /projects/{id}/cypher/write` accepts `connection_id` too, but remote
writes are safe-by-default: they require `"confirm_remote_write": true`, and
cross-project connection ids are rejected with 404 (no scope leak).

## Proxy mode (MCP)

`read_cypher` accepts an optional `connection_id` argument with the same
semantics: same statement, same shape, Aura instead of local. The connection
must belong to the MCP session's project scope.

## Client pattern

Keep the statement in one place and inject the target from configuration:

```python
TARGET = os.environ.get("RELATING_TARGET", "relate://local")  # or "aura://<cid>"
result = relating.cypher_read(project_id, QUERY, params, target=TARGET)
```

Only `RELATING_TARGET` changes at cutover. The Cypher Console (tab 4) has the
same flip as a target dropdown so the behavior can be tried by hand first.

## Limits (stated, not silent)

- Proxy mode routes statements; it does not translate dialects. Statements
  that use engine-specific functions should be checked with the Portability
  Compiler (R3) before cutover.
- Result columns come from the Aura `RUN` response field names, matching the
  local column names for the same statement.
