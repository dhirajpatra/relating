# R6 — Synthetic Test Data

Base path: `/api/v1`. Generates deterministic, production-like graph fixtures
from a live project's profiled shapes — never by copying customer values.

## Profile

`POST /projects/{id}/synth/profile` returns the source profile: per-label
counts with property types/coverage, per-relationship counts with sampled
endpoint labels and property shapes, and totals. Sampling caps at 500 per
group; totals are exact counts.

## Generate

`POST /projects/{id}/synth/generate`:

```json
{
  "scale": 1.0,
  "seed": 7,
  "edge_cases": ["duplicates", "orphans", "cycle", "invalid_values"],
  "max_nodes": 50000,
  "materialize": false,
  "target_name": "fixture",
  "include_dataset": false
}
```

- Same seed + same profile always yields the identical dataset.
- Counts scale proportionally (minimum 1 per non-empty group); over-budget
  requests scale down with an explicit note. Scalar property types are
  synthesized; array/object shapes are skipped with a note, never faked.
- Every relationship endpoint references a generated uid (referential
  integrity by construction).
- Edge cases: `duplicates` (cloned entity, fresh uid), `orphans` (3
  unconnected nodes), `cycle` (3-node loop; skipped with a note when no
  self-referential type was sampled), `invalid_values` (wrong-typed value for
  negative testing; skipped when no numeric property was sampled).
- Every generated node/relationship carries `_synthetic: true`.
- `materialize: true` creates an isolated sibling project and loads the
  dataset there; the source project is never modified.

## CLI

`relating synth generate --project ID --out FILE [--scale F] [--seed N]
[--edge-cases a,b] [--materialize]` writes the dataset JSON (and optionally
materializes) for CI fixtures.
