# R5 — Shadow Testing

Base path: `/api/v1`. Shadow testing replays an explicit, safe sample of
**read-only** queries against the local Relate graph and an Aura target, then
compares structure, counts, errors, and latency into a migration confidence
report.

## Run

`POST /projects/{id}/shadow/runs`:

```json
{
  "connection_id": "...",
  "queries": [{"name": "city count", "cypher": "MATCH (n:City) RETURN count(n)", "params": {}}],
  "redact_params": true,
  "include_samples": false
}
```

- At most 100 queries, 20k chars each; every query is gated to read-only
  (anything write-shaped is refused before touching either backend).
- Only the submitted queries run — Relating never scrapes production traffic.
- Parameter values are masked in the report unless `redact_params: false`.
- Target samples are never stored; source samples attach only with
  `include_samples: true`.

## Verdicts and confidence

Per query: `MATCH` (same shape, counts, content hash), `STRUCTURE_DIFF`
(arity/type profile), `COUNT_DIFF` (counts or values), `ERROR_DIFF` (success
differs; identical failure classes still match). Latency reports per side plus
the target/source ratio.

Confidence is explicit, not vibes: `0.7 * match_rate + 0.3 * success_rate`,
mapped to `HIGH (>=0.9)`, `MEDIUM (>=0.6)`, `LOW`, with any error-diff
capping confidence at `MEDIUM`. Reasons list every contributor.

## CLI and CI

`relating shadow run --project ID --connection CID --queries FILE`
(JSON array of `{name, cypher, params?}`) prints the report and exits 1 on
`LOW` confidence. The CI workflow runs it whenever `shadow/queries.json`
exists alongside `AURA_PASSWORD`.
