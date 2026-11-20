# R6 — Performance and Cost Advisor

`POST /projects/{id}/advisor/analyze` — `{queries: [{name, cypher}],
connection_id?, measure?, vectors?, aura_tier?}` (max 50 queries, 20k chars
each). Static review plus live structure notes, sizing estimates, and
optional measured evidence.

## Query rules (static, each explained with a fix)

`ADV-UNBOUNDED` (result scan without LIMIT/aggregation),
`ADV-CARTESIAN` (comma-separated patterns), `ADV-EXPAND` (unbounded `[*]`;
bounded ranges get an info note), `ADV-SCAN` (substring predicates),
`ADV-SHORTEST` (path search cost), `ADV-INDEX` (equality predicates as
*candidates* — the report says to confirm with EXPLAIN, never claims a
missing index), `ADV-PARAMS` (literals vs plan caching).

## Structure (measured counts, labeled heuristics)

uid merge-key coverage (measured count query), per-label scale bands and
relationship skew (explicit heuristic thresholds on measured counts). Every
note carries its basis: measured numbers say `measured: ...`; judgments say
`heuristic ...`.

## Sizing (estimate, never a quote)

Element counts map to a tier hint with stated assumptions (bytes per
node/relationship, excluded embeddings/throughput) and caveats (measure
before committing; confirm current Aura pricing). Storage math is shown, not
hidden.

With `vectors: [{profile_name?, dimensions, vector_count}]`, raw float32
vector bytes are folded into storage and every profile gets a dimension +
tier-RAM fit verdict (same engine as the migration analyzer's
`vector_index_fit` findings): 1–4096 dimensions enforced, RAM checked against
`aura_tier` (preset id, `<N>gb`, `<N>mb`, `custom:<mb>`) when given, and an
explicit caveat when it is not. The response gains a `vectors` section with
per-profile `verdict`, `detail`, and `remediation`; without `vectors` the
sizing shape is unchanged.

## Measurements (observed, optional)

With `connection_id` + `measure: true`, the advisor reads real target counts
and runs `EXPLAIN` per query (planning check + planning latency), reporting
per-query planned/errored honestly. Failures report; they never abort the run.

## CLI

`relating advisor analyze --project ID --queries FILE [--connection CID]
[--measure] [--fail-on-warning]` prints the same report; `--fail-on-warning`
turns it into a CI gate (default exit 0: advisory, not blocking).
