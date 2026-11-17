# R6 — Performance and Cost Advisor

`POST /projects/{id}/advisor/analyze` — `{queries: [{name, cypher}],
connection_id?, measure?}` (max 50 queries, 20k chars each). Static review
plus live structure notes, sizing estimates, and optional measured evidence.

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

## Measurements (observed, optional)

With `connection_id` + `measure: true`, the advisor reads real target counts
and runs `EXPLAIN` per query (planning check + planning latency), reporting
per-query planned/errored honestly. Failures report; they never abort the run.

## CLI

`relating advisor analyze --project ID --queries FILE [--connection CID]
[--measure] [--fail-on-warning]` prints the same report; `--fail-on-warning`
turns it into a CI gate (default exit 0: advisory, not blocking).
