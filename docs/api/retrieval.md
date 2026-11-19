# Slice 3 — Retrieval, Q&A, Evaluation API

Base path: `/api/v1`. All bodies are JSON. Error shape is
`{"error": {"code": "...", "message": "..."}}`.

## POST /projects/{id}/embeddings — generate embeddings

Chunks every source in the project and embeds the chunks with the project's
configured AI provider (OpenAI-compatible, Ollama, or mock).

Request (all fields optional):

```json
{ "profile_name": "default", "max_chars": 1200, "overlap_chars": 150 }
```

- CSV/JSON sources produce one chunk per record (`field: value` lines with
  `lineage_ref` provenance).
- Text sources are split into overlapping prose chunks.
- PDF sources are extracted per page (`page_<n>_block_<m>` lineage with a
  `page_number` field) and chunked like prose.
- DOCX sources become paragraph records with heading-derived `section`
  context (`para_<n>` lineage) plus table-row records (`table_<t>_row_<r>`
  lineage with `cells`); both chunk like prose.
- Binary PDF/DOCX uploads travel as `content_base64` (see Sources); legacy
  `.doc` (OLE) is rejected with a convert-to-`.docx` message.
- Generation is idempotent: chunks are unique on `(profile_id, content_hash)`
  and already-indexed chunks are skipped, never re-embedded.
- The embedding profile is created on first use; dimensions are discovered
  from the provider response and enforced on every later call. Vectors from
  different dimensions are never compared (`RETRIEVAL_ERROR`).

Response `201`:

```json
{
  "profile": { "id": "...", "name": "default", "model": "...", "dimensions": 1536 },
  "sources_processed": 1,
  "chunks_created": 3,
  "chunks_skipped": 0,
  "dimensions": 1536,
  "elapsed_ms": 12.4
}
```

## GET /projects/{id}/embeddings — index status

Lists embedding profiles with per-profile chunk counts.

## POST /projects/{id}/retrieval — retrieve passages

```json
{ "query": "Who is Ada?", "strategy": "hybrid", "top_k": 5 }
```

- `strategy`: `vector` | `keyword` | `hybrid` (default, reciprocal rank fusion).
- `top_k`: default 5, clamped to 1–50.
- `400 RETRIEVAL_ERROR` when no embedding index exists yet.

Response is a `RetrievalTrace`: query, strategy, profile/model/dimensions,
ranked passages with scores and filenames, and latency.

## POST /projects/{id}/questions — grounded Q&A

```json
{ "question": "Who works in Platform?", "strategy": "hybrid" }
```

Retrieves evidence, then calls the project's chat model with the versioned
`rag-qa-v1` prompt. Answers carry `grounded`, `citations` (valid `[n]`
markers resolved to passages), the full retrieval trace, model IDs, and
prompt version. When no evidence exists the model is never called and the
answer is an explicit honest `grounded: false` response — never fabricated.

## POST /projects/{id}/evaluations — run an evaluation

```json
{ "questions": ["Who is Ada?", "Who is Bob?"], "strategy": "hybrid" }
```

Runs grounded Q&A per question (1–50 questions), scores citation precision
(cited markers ∩ retrieved passages) and grounded rate, persists the report,
and returns it (`201`). `GET /projects/{id}/evaluations` lists runs;
`GET /projects/{id}/evaluations/{eval_id}` fetches one report.

## Ollama configuration

Create the project with provider `ollama`, base URL `http://localhost:11434/v1`,
a chat model (e.g. `llama3.1`) and an embedding model (e.g. `nomic-embed-text`).
Verify with the project's AI health check before generating embeddings.
A live Ollama check is available as an ignored test:

```bash
OLLAMA_EMBEDDING_MODEL=nomic-embed-text cargo test -p relating-server \
  --test retrieval_integration_tests -- --ignored
```
