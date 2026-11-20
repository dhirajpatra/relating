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

See `.github/workflows/relating.yml`: checkout, stable Rust, build the CLI,
resolve the project (the `RELATING_PROJECT_ID` variable when set, otherwise a
freshly seeded demo project), then validate, schema-diff (only when
`relating/graph/schema.json` is committed), contract tests, conditional Aura
analysis (only when `AURA_PASSWORD` plus `AURA_CONNECTION_ID` are set), a
conditional eval gate (only when `eval/questions.txt` exists), and a
conditional shadow gate (only when `shadow/queries.json` exists alongside Aura
credentials). Failing gates fail the workflow and block the merge; missing
optionals skip quietly instead of failing on empty configuration.
