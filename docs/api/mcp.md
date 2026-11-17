# R1 — MCP Server

Relating exposes a read-only MCP server (protocol `2024-11-05`) with 12 tools.
Every connection is bound to one project; tools outside that scope are rejected.

## Transports

- **stdio** (local agents): `relating-mcp --stdio --project <PROJECT_ID>`
  (respects `RELATING_DATA_DIR` / `RELATING_DATABASE_URL`). Newline-delimited
  JSON-RPC on stdout; logs go to stderr.
- **Streamable HTTP** (self-hosted): `POST /mcp` with a JSON-RPC body, served
  by `relating-server`. Requires `Authorization: Bearer <RELATING_MCP_TOKEN>`
  and an `x-relating-project: <UUID>` header. The transport stays disabled
  (403) until `RELATING_MCP_TOKEN` is set. `GET /mcp/health` is public and
  reports server/mode/tool count only.

## Tools (all read-only)

`list_projects`, `get_project_summary`, `get_graph_schema`, `search_sources`,
`search_graph`, `read_cypher` (engine-enforced read-only store open, default
limit 100), `retrieve_context`, `explain_retrieval`, `get_ingestion_status`,
`get_evaluation_report`, `analyze_aura_compatibility`, `preview_migration`.

Tool failures return MCP `isError` content, never stack traces or secrets.
Calls time out after 30s. Write tools (`ingest_source`, `write_cypher`,
`create_ontology_revision`, `execute_approved_migration`) are intentionally
absent in R1; the server answers them as unknown methods.

## Client setup

See the “Connect an agent” screen (project tab 7) for copyable Claude Code,
Codex, Cursor, and generic configurations. Integrations are documented as
configuration patterns; per-client behavior should be verified against the
client version in use.
