# Data Connectors — SQL and REST Sources

Base path: `/api/v1`. Connectors fetch rows on demand, map them to
normalized records, and run the standard extract-and-load pipeline against
the project's approved ontology. Each connector owns one managed source
(`connector://<name>`) refreshed in place, plus watermark/hash state for
incremental syncs.

## Registry

- `POST /projects/{id}/connectors` — `{name, kind, config, mapping?,
  poll_interval_secs?}`. `kind` is `sql` or `rest`; configs validate
  before persisting (bad configs fail here, never at 2am sync). Duplicate
  names are rejected per project.
- `GET .../connectors`, `GET .../connectors/{cid}` (cross-project ids
  404), `DELETE .../connectors/{cid}` (204).
- `GET .../connectors/due` — connectors whose `poll_interval_secs`
  elapsed (or never synced). Cron-style schedulers poll this list, then
  `POST .../sync` for each id. There is no background daemon.

## SQL sources

```json
{
  "driver": "sqlite",
  "path": "/data/warehouse.db",
  "query": "SELECT uid, name, updated FROM people",
  "watermark_column": "updated",
  "limit_per_sync": 5000
}
```

Networked drivers add `host`, `port`, `database`, `username`, and
`password_env` (resolved via the secrets chain at sync time; the name
follows the same allowlist as Aura connections). Postgres takes
`ssl_mode: disable|prefer`.

Queries are validated read-only (`SELECT`/`WITH`, single statement —
stacked queries rejected). With a `watermark_column` and a stored mark,
syncs wrap the query (`WHERE "col" > mark ORDER BY "col" LIMIT n`);
watermark values outside a strict timestamp/numeric charset fall back to
a full snapshot rather than risking injection. Empty deltas short-circuit
as `unchanged` without touching the graph; `?full=true` re-transfers.

## REST sources

```json
{
  "base_url": "https://crm.example/items",
  "method": "GET",
  "headers": {"X-Tenant": "acme"},
  "bearer_env": "MY_CRM_TOKEN",
  "record_path": "data.items",
  "pagination": {"mode": "offset", "page_size": 100, "max_pages": 20},
  "rate_limit_per_sec": 5,
  "max_attempts": 3,
  "timeout_secs": 15
}
```

Raw credential headers (`authorization`, `x-api-key`, `cookie`, …) are
rejected at creation — secrets ride `bearer_env` / `basic_password_env`
references. Pagination modes are `none`, `offset` (`?offset&limit`), and
`page` (`?page&limit`); empty pages stop the walk. Transport errors,
429s, and 5xx retry with capped exponential backoff honoring
`Retry-After`; bodies truncate to 300 chars in errors. Loopback sources
always go direct; remote hosts honor the proxy environment.

`record_path` is a dot path to the record array (empty = root array).
Non-object items are skipped and counted.

## Sync

- `POST .../connectors/{cid}/test` — read-only probe: one row through the
  real fetch path (viewer+). Registers nothing, updates nothing.
- `POST .../connectors/{cid}/sync[?full=true]` — fetch, map
  (`uid_field` default `uid`→`id`, plus renames/drops; keyless rows skip),
  extract, and load. Requires an approved ontology. Reports
  `{rows_fetched, records_mapped, rows_skipped, unchanged, nodes_created,
  relationships_created, watermark}`.

File ingestion stays incremental too: `POST .../ingest[?full=true]`
skips sources whose bytes match `last_ingested_hash` and reports
`sources_skipped`; connector-managed sources sync through their own
endpoints. Byte parsers and row mappers are intentionally separate —
connectors bypass `parse_source_content` by design.

## Client SDKs

- Python (`sdks/python`, zero-dependency): `RelatingClient` over every
  endpoint above plus the full product surface, `RELATING_TARGET`
  backend flip, and typed errors (`RelatingError` + subclasses).
  Tests: `PYTHONPATH=src python3 -m unittest discover -s tests`.
- TypeScript (`sdks/typescript`, zero runtime dependencies):
  `RelatingClient` class with the workbench types plus connector/auth/
  audit/sync coverage, `baseUrl`/`token` options, Node + browser fetch.
  Build/test with the workbench compiler, no extra install:
  `npm run build`, `npm test`.

## Limits (stated, not silent)

- SQL value decoding covers the Any driver's nine primitive kinds;
  temporal/uuid/numeric columns must cast to text in the query
  (`updated_at::text`), or the fetch fails naming the type.
- Postgres `sslmode=require` is rejected (no TLS stack compiled in);
  `disable`/`prefer` work on private networks.
- REST auth is bearer/basic via secret references; OAuth flows and
  cursor pagination are follow-ups.
- Syncs run inside the request like migration executes; very large
  snapshots should page via watermark columns and `limit_per_sync`.
