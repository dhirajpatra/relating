# R4 — Graph CI/CD

The `relating` CLI runs every validation gate headlessly so pull requests can
block on graph health. Exit codes: `0` pass, `1` gate/test failure, `2`
usage or config error. All commands accept `--data-dir`, `--database-url`
(same defaults as the server), and `--format text|json`.

## Commands

- `relating validate --project ID [--all] [--connection CID] [--eval-questions FILE]`
  Always runs schema conformance, data integrity (uid coverage, duplicate
  merge keys, orphan warnings), and embedding index health. `--all` adds Aura
  compatibility (needs `--connection` or `AURA_URI` + `AURA_PASSWORD` in the
  environment) and a retrieval mini-eval (needs `--eval-questions`, one
  question per line); missing optionals are reported as skipped, never failed.
- `relating aura analyze --project ID [--connection CID]` — source + target
  compatibility; fails on `UNSUPPORTED` verdicts or unreachable targets.
  Without `--connection`, the `.env` Aura connection is used when available.
- `relating aura migrate --project ID --connection CID [--yes] [--report FILE] [--aura-tier T] [--api-url URL]` —
  the full approval-gated migration against a running server (test →
  analyze → plan → dry-run → approve → execute → verify → report).
  Prints the plan summary and requires `--yes` or an interactive yes
  before anything writes; exits 1 on blockers or `passed: false`.
  Sends `RELATING_API_TOKEN` as a bearer token when set.
- `relating portability scan --dir DIR [--emit text|json|sarif] [--out FILE]` —
  scans a repo for Cypher portability risks; exits 1 on breaking
  (`NEEDS_REWRITE`) findings. `--emit sarif` produces SARIF 2.1.0 for
  `github/codeql-action/upload-sarif` PR annotations.
- `relating test graph --project ID [--revision N]` — graph contract tests
  against the latest (or pinned) schema revision.
- `relating schema export --project ID --out DIR` — writes the five
  as-code files for committing to the application repo.
- `relating schema diff --project ID --file schema.json` — diffs a committed
  file against live state; exits 1 on breaking changes.
- `relating eval run --project ID --questions FILE [--strategy S] [--min-grounded F]`
  — runs grounded Q&A per question (default hybrid, min grounded rate 0.5).
- `relating demo seed [--name NAME]` — creates a seeded demo project (mock AI
  provider, two connected cities) and prints its ID. Used by CI when no pinned
  project is configured; handy for local smoke tests too.

## CI wiring

`.github/workflows/relating.yml` runs four jobs on every PR and main push:

- **fmt / clippy / test / web** — `cargo fmt --check`, clippy with warnings
  denied, the full workspace test suite, and the workbench UI build.
- **docker-build** — builds the server image; pushes `latest` + `$sha` to
  GHCR on main pushes.
- **migration flow vs Neo4j** — a `neo4j:5-community` service plus the real
  server binary runs analyze → plan → dry-run (asserts zero remote writes
  and no blockers) → approve → execute → verify (asserts `passed: true`) →
  report on every PR.
- **graph validation** — build the CLI, resolve the project (the
  `RELATING_PROJECT_ID` variable when set, otherwise a freshly seeded demo
  project), then validate, schema-diff (only when
  `relating/graph/schema.json` is committed), contract tests, conditional
  Aura analysis, a conditional eval gate (only when `eval/questions.txt`
  exists), a conditional shadow gate, and a portability SARIF scan whose
  findings upload as code-scanning alerts.

Aura steps gate on variables, never secrets: set `AURA_ENABLED=true` plus
`AURA_CONNECTION_ID`, with `AURA_PASSWORD` as a secret exposed at job level
(a step-level `env:` is invisible to that same step's `if:`, and secrets
cannot appear in `if:` at all). Failing gates fail the workflow and block
the merge; missing optionals skip quietly instead of failing on empty
configuration.

Two more workflows: `aura-nightly.yml` (scheduled; runs read-only Aura
analysis against a real Aura Free instance through the `aura-nightly`
environment — add required reviewers there for a human gate — and uploads a
version-pinned compatibility matrix) and `release.yml` (on `v*` tags;
builds portable linux-x86_64 binaries plus the workbench bundle and attaches
them to the GitHub release).
