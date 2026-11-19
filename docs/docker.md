# Docker Compose — Runbook

One command starts the whole stack: the workbench server, team Postgres,
and local Ollama AI.

```bash
docker compose up --build
```

Then open http://127.0.0.1:8080 in your browser.

## Services

| Service    | Folder               | What it is | Default port |
|------------|----------------------|------------|--------------|
| `server`   | `docker/server/`     | Axum API + React workbench + MCP HTTP transport; SQLite runtime store at `/app/data` | 8080 |
| `db-init`  | (uses server image)  | One-shot job: `relating db init-postgres` + `db check-postgres` against `postgres`; idempotent | — |
| `postgres` | `docker/postgres/`   | Team Postgres 17 for the shared metadata schema (see `docs/postgres.md`) | 5432 |
| `ollama`   | `docker/ollama/`     | Local AI backend; pre-pulls chat + embedding models on first start | 11434 |
| `neo4j`    | `docker/neo4j/`      | Opt-in only (`--profile neo4j`): local Bolt target for testing the Aura migration flow | 7474 / 7687 |

Each service builds from its own folder with its own Dockerfile. `db-init`
reuses the server image with a different command on purpose — it is a job,
not a separate image.

What is deliberately NOT a service:

- **Graph database** — Relating uses embedded project-scoped Relate stores
  (files under `/app/data`), so no graph container is needed.
- **MCP server** — the HTTP transport (`POST /mcp`) is served by `server`;
  the stdio binary is for agents on your own machine.
- **Neo4j Aura** — the documented migration destination is cloud-hosted;
  the optional `neo4j` profile is only a local stand-in for testing.

## Configuration

All tunables have working defaults in `docker-compose.yml`, so a bare
`docker compose up` works with no `.env`. To change ports, passwords, or
models, copy the template and edit it:

```bash
cp .env.example .env
```

Relevant keys: `RELATING_PORT`, `POSTGRES_USER`, `POSTGRES_PASSWORD`,
`POSTGRES_DB`, `POSTGRES_PORT`, `OLLAMA_PORT`, `OLLAMA_PRELOAD_MODELS`,
`OLLAMA_NUM_PARALLEL`, `NEO4J_PASSWORD`, `NEO4J_HTTP_PORT`,
`NEO4J_BOLT_PORT`, `RELATING_MCP_TOKEN`, `AURA_PASSWORD`.

Notes:

- The server container always runs on SQLite (`RELATING_DATABASE_URL` is
  pinned in the compose file). Postgres holds the provisioned team schema;
  the runtime cutover is roadmap (see `docs/postgres.md`).
- Change `POSTGRES_PASSWORD` / `NEO4J_PASSWORD` before exposing ports
  beyond localhost.

## Using Ollama from a project

AI providers are configured per project in the workbench, never from the
environment. In the project's AI tab set:

- provider: `ollama`
- base URL: `http://ollama:11434/v1` (the in-compose hostname)
- chat model: `llama3.1`
- embedding model: `nomic-embed-text`

then use Test to verify. From your laptop (outside compose) the same
server is `http://127.0.0.1:11434/v1`.

First start downloads several GB of models. To skip preloading (pull
later with `docker compose exec ollama ollama pull <model>`), set
`OLLAMA_PRELOAD_MODELS=` (empty) in `.env`.

For NVIDIA GPU acceleration, install the NVIDIA container toolkit and
uncomment the `deploy.resources` block on the `ollama` service.

## Using the local Neo4j

```bash
docker compose --profile neo4j up --build
```

Register an Aura connection in the workbench pointing at
`bolt://neo4j:7687` (user `neo4j`, password = `NEO4J_PASSWORD`), then run
analyze → plan → dry-run → execute → verify. Browser UI:
http://127.0.0.1:7474.

## CLI inside compose

The server image also ships the `relating` CLI (with the `postgres`
feature), so any headless workflow can run against the stack:

```bash
docker compose exec server relating validate --project <ID>
docker compose run --rm server relating db check-postgres \
  --url postgres://relating:relating@postgres:5432/relating
```

## Data and reset

Named volumes hold all state: `relating-data` (SQLite + Relate stores),
`pgdata`, `ollama-data`, `neo4j-data`, `neo4j-logs`.

```bash
docker compose down            # stop, keep data
docker compose down -v         # stop and DELETE all stack data
docker compose logs -f server  # follow the server log
docker compose ps              # health status of every service
```

## Troubleshooting

| Symptom | Fix |
|---|---|
| `server` unhealthy at first boot | It waits on `postgres` + `ollama` health; check `docker compose ps` and `docker compose logs ollama` (model pulls take a while on first run) |
| `db-init` exits 1 | Postgres never became ready; `docker compose logs postgres db-init`, then `docker compose up db-init` to retry |
| Port already in use | Set `RELATING_PORT` / `POSTGRES_PORT` / `OLLAMA_PORT` in `.env` |
| Ollama model pull fails | Non-fatal by design; the stack keeps working with the mock provider — retry with `docker compose exec ollama ollama pull <model>` |
| Stale UI after `git pull` | `docker compose up --build` |
