# Relating — Step-by-Step Demo Guide

This guide takes you from a fresh checkout to a running demo: one command
starts the API and the web workbench, and the walkthrough below exercises the
full local loop (project → ingest → graph → GraphRAG) plus the optional Aura
and agent pieces.

## 0. What `start.sh` does

`./start.sh` is the only start command you need. It:

1. Loads `./.env` if present (already-exported variables take precedence).
2. Refuses to start with a clear message if the port is taken.
3. Builds the React UI once (skipped on later runs unless you pass `--rebuild-web`).
4. Exports the `RELATING_*` settings and runs the Axum server, which also
   serves the UI — so a single URL gives you everything.

Keep the terminal open while you demo; stop with `Ctrl+C`.

## 1. Prerequisites

| Tool | Required | Check command | Expected |
|---|---|---|---|
| Rust stable | Yes | `cargo --version` | `cargo 1.8x+` (install via [rustup](https://rustup.rs)) |
| Node.js 20+ with npm | Yes | `node --version` | `v20+` (`start.sh` warns below 20) |
| Ollama | No (mock AI works offline) | `ollama --version` | only for real local inference |
| Neo4j Aura account | No | — | only for the migration demo |
| Docker | No | `docker --version` | only for the container path (step 8) |

```bash
cargo --version
node --version
```

No `relate` binary is needed: the graph engine is a Rust dependency and is
compiled in automatically.

## 2. Configure `.env` (demo attributes)

```bash
cp .env.example .env
```

For a basic demo **every variable is optional** — defaults work out of the
box. Set only what your demo needs:

| Attribute | Needed for demo? | Default | What it does |
|---|---|---|---|
| `RELATING_PORT` | No | `8080` | HTTP port (or `./start.sh --port 8081`) |
| `RELATING_HOST` | No | `127.0.0.1` | Bind address |
| `RELATING_DATA_DIR` | No | `./data` | Projects, graph stores, uploads |
| `RELATING_DATABASE_URL` | No | `sqlite://./data/relating_metadata.db` | Metadata store (SQLite for local) |
| `RELATING_LOG_LEVEL` | No | `info` | Log verbosity (`debug` for troubleshooting) |
| `RELATING_MCP_TOKEN` | Only to demo agent HTTP access | unset (HTTP off, stdio works) | Bearer token for `POST /mcp` |
| `RELATING_SECRET_BACKEND` | No | `auto` | `env` → `*_FILE` → `keychain`; see [docs/secrets.md](docs/secrets.md) |
| `AURA_PASSWORD` | Only for the Aura migration demo | unset | Aura password (never stored); or `AURA_PASSWORD_FILE` |

A typical demo `.env` is therefore tiny:

```bash
# Minimal demo: everything else uses defaults.
RELATING_MCP_TOKEN=demo-token-change-me
AURA_PASSWORD=<paste-only-if-demoing-migration>
```

> AI keys do **not** go in `.env` — the server never reads `OPENAI_*` style
> variables. Each project carries its own provider config, entered in the
> workbench (step 5.1). New projects default to the built-in **mock**
> provider, so the full loop demos with no keys and no network.

`.env` is gitignored — secrets you paste there are never committed.

## 3. Start the application

```bash
./start.sh
```

First run takes a few minutes (Rust compile + `npm install` + UI build);
later runs start in seconds. Expected output ends with:

```text
==> Starting Relating at http://127.0.0.1:8080
    Open that URL in your browser to use the workbench.
    API base: http://127.0.0.1:8080/api/v1  |  Data: /path/to/data
    MCP: stdio only (set RELATING_MCP_TOKEN in .env to enable the HTTP transport)
```

Useful variants:

```bash
./start.sh --port 8081        # avoid a port clash
./start.sh --rebuild-web      # force a UI rebuild after pulling changes
./start.sh --release          # optimized binary (slower build, faster app)
./start.sh --skip-web-build   # never touch web/dist
./start.sh --help             # all options
```

Sanity check from another terminal:

```bash
curl -s http://127.0.0.1:8080/api/v1/health
# {"status":"ok","product":"Relating",...}
```

## 4. Open the workbench

Open **http://127.0.0.1:8080** in your browser. If you used `--port 8081`,
open that port instead. Leave the server terminal running.

## 5. Demo walkthrough (about 15 minutes)

### 5.1 Create a project (AI Service)

1. Click **New Project**, name it (e.g. `Demo`), keep the default **mock**
   provider, create it.
2. Open the **AI Service** tab and run **Test** — a healthy mock report
   proves the AI plumbing end to end with no keys.
3. For *real* inference instead: start Ollama (`ollama serve`, plus
   `ollama pull llama3.1` and `ollama pull nomic-embed-text`), then set the
   project's provider to **ollama** with base URL
   `http://localhost:11434/v1`, chat model `llama3.1`, embedding model
   `nomic-embed-text`, and test again. (OpenAI works the same way with
   `https://api.openai.com/v1` plus an API key.)

### 5.2 Ingest data (Sources)

1. Open **Sources** and click **+ Add Sample CSV & JSON** — instant dataset,
   no files needed. Each source shows byte size, record count, and SHA-256.
2. Optionally drag in a real file: CSV, JSON, text, **PDF**, or **DOCX**
   (PDF pages and DOCX sections keep their lineage).

### 5.3 Ontology gate (Ontology)

1. Click **Propose ontology** — the AI suggests entities/relationships.
2. Review and **Approve**. Nothing is written to the graph until approval;
   this gate is the product's safety story.

### 5.4 Extract into the graph (Ingest)

Run **Ingest**. The response reports sources processed plus nodes,
relationships, and properties created — all carrying source provenance.

### 5.5 Explore (Graph Explorer + Cypher Console)

1. **Graph Explorer**: visual Cytoscape graph; click nodes for properties.
2. **Cypher Console**: run `MATCH (n) RETURN n LIMIT 25`. Reads are capped
   for safety; writes need the explicit write button. The **Target** dropdown
   runs the identical query against Aura after migration (no rewrite).

### 5.6 GraphRAG (Retrieval & Q&A)

1. Click **Generate embeddings** (mock or Ollama, per project config).
2. Run a **retrieval** (vector / keyword / hybrid) and inspect the trace.
3. **Ask a question** — the answer arrives with citations into sources.
4. Run an **evaluation** to score groundedness over sample questions.

### 5.7 Agents (MCP)

Open **Agents** for copy-paste configs (Claude Code, Codex, Cursor, generic).
`read_cypher` and friends are project-scoped and read-only by default; the
HTTP transport needs the `RELATING_MCP_TOKEN` from step 2.

### 5.8 Aura migration (needs Aura + `AURA_PASSWORD`)

1. **Aura Migration** → **+ New Connection** (`neo4j+s://…`, user `neo4j`,
   env var `AURA_PASSWORD`, which you exported or put in `.env`).
2. **Test**, then **Analyse** (pick the target tier for vector-RAM checks).
3. **Plan Migration** → **Dry-run** (always `0` remote writes) → **Approve**
   → **Execute** → **Verify**. The plan banner states the write mode
   explicitly: MERGE upsert by stable keys — appends/updates only, the target
   is never wiped.

### 5.9 Power tabs (1 minute each)

- **Schema & Contracts**: export schema-as-code, revisions, diff.
- **Portability**: scan an app repo for queries needing rewrites for Aura.
- **Shadow & Debug**: replay queries Relate-vs-Aura; trace GraphRAG answers.
- **Test Data & Advisor**: synthetic fixtures; query/sizing advice including
  vector-fit checks per Aura tier.

## 6. CLI alternative (headless)

Same backend, no browser (uses `./data` by default):

```bash
cargo run -q -p relating-cli -- demo seed --name cli-demo
# prints a project_id; use it below as $PID
cargo run -q -p relating-cli -- validate --project $PID
cargo run -q -p relating-cli -- schema export --project $PID --out /tmp/schema
cargo run -q -p relating-cli -- advisor analyze --project $PID --queries queries.json
```

## 7. Stop and reset

```bash
# Stop: Ctrl+C in the server terminal.
# Full reset (deletes demo data):
rm -rf ./data
./start.sh   # recreates everything on boot
```

## 8. Docker alternative

```bash
docker compose up --build
# workbench at http://127.0.0.1:8080, data in the relating-data volume
```

Note: the compose file pins its own environment values, so `.env` is not
picked up there — edit `docker-compose.yml` to change ports or secrets.

## 9. Troubleshooting

| Symptom | Fix |
|---|---|
| `Port 8080 ... is already in use` | `./start.sh --port 8081` |
| UI looks stale after `git pull` | `./start.sh --rebuild-web` |
| `cargo`/`node`/`npm` missing | Install Rust via rustup; Node 20+ via nodejs.org |
| Node `< 20` warning | Upgrade Node, then `--rebuild-web` |
| AI calls fail on a real provider | Recheck base URL / key / model in the project's AI tab, then Test |
| Aura auth fails | `export AURA_PASSWORD='…'` (or `.env`) and restart the server |
| `RELATING_DATABASE_URL` points at Postgres | Server refuses by design; SQLite is the runtime store — see [docs/postgres.md](docs/postgres.md) |
| Need verbose logs | `RELATING_LOG_LEVEL=debug ./start.sh` |

## 10. Command cheat sheet

```bash
cp .env.example .env   # once: create local config (optional, defaults work)
./start.sh              # start API + UI at http://127.0.0.1:8080
curl -s http://127.0.0.1:8080/api/v1/health   # health check
# Ctrl+C               # stop
```
