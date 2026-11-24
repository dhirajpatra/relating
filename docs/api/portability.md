# R3 — Graph Portability Compiler

Base path: `/api/v1`. The compiler scans an application's repository for
embedded Cypher, drivers, connection strings, transactions, APOC usage, and
inline DDL, then classifies each finding for Aura portability.

## Scan

`POST /projects/{id}/portability/scan` — `{repo_path}` (a directory readable
by the server process). Returns a `PortableReport`:

- `summary`: files/queries scanned plus verdict counts.
- `findings`: `{file, line, rule_id, verdict, message, remediation?}` with
  verdicts `PORTABLE`, `TRANSFORM`, `NEEDS_REWRITE`, `NEEDS_REVIEW`.
- `breaking`: the must-fix (`NEEDS_REWRITE`) subset.
- `tests`: generated target-validation tests (capped at 200, flagged when
  truncated). Each carries `read_only` so the runner can fence writes.

Rule highlights: `R-APOC`/`R-PROC` (procedures must exist on target),
`R-PERIODIC` (rewrite to `UNWIND` batches, corrected candidate included),
`R-LOADCSV` (Aura file access), `R-SHORTEST`/`R-SUBQUERY` (version-dependent),
`R-DDL-OLD` (legacy `ASSERT` → `REQUIRE` rewrite), `R-DRIVER` (Relate imports
are transforms), `R-URI` (plaintext needs review), `R-MULTI` (multi-statement
strings), `R-NOPARAMS` (bind literals).

Scanner limits (stated): Python/TypeScript/JavaScript/Java/Go/Rust sources
plus `.cypher` files; skips VCS, dependency, and build directories; 1 MB per
file; 5000 files and 2000 queries per scan; no symlink escapes.

## Target test runs

`POST /projects/{id}/portability/run-tests` — `{connection_id, tests[]}`
(max 200 tests, 20k chars each). Read tests run as-is; write-shaped tests run
behind `EXPLAIN` so validation never mutates the target. Assertions:
`no_error`, `row_count_eq`, `row_count_gte`. Returns per-test
`{passed, detail, row_count, elapsed_ms}` plus totals. Failures are reported
per test; one bad query never aborts the run.

## SARIF and CI gates

`POST /projects/{id}/portability/scan?format=sarif` returns SARIF 2.1.0
instead of the native report (non-portable findings only, with levels
`note`/`warning`/`error` and remediation fixes). Upload it with
`github/codeql-action/upload-sarif` so findings show up as PR annotations.
Headlessly, `relating portability scan --dir DIR [--emit text|json|sarif]
[--out FILE]` scans any checkout and exits 1 on breaking
(`NEEDS_REWRITE`) findings.
