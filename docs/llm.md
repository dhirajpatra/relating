# LLM Backends — Meta API with Ollama Fallback

Relating picks an AI provider per project, with `.env` acting as the
default when a project did not choose one.

## Selection order

1. A project with an explicit non-`mock` provider (`openai`, `ollama`,
   `meta_api`) always uses it.
2. Otherwise (`mock`, the default for new projects), when `LLM_BACKEND`
   is `meta_api` **and** an API key resolves, the Meta API is used.
3. Otherwise the built-in mock provider answers offline and
   deterministically.

Picking provider `meta_api` explicitly in the workbench (or API) forces
Meta for that project; blank base URL / key fields fall back to the
`.env` values so workstations share one server-side key.

## Environment variables

| Variable | Default | Meaning |
|---|---|---|
| `LLM_BACKEND` | (unset) | `meta_api` enables the Meta backend |
| `META_API_KEY` (`MODEL_API_KEY` alias) | — | Bearer token; resolves via `docs/secrets.md` |
| `META_API_URL` | `https://api.meta.ai/v1/responses` | Full Responses endpoint |
| `META_MODEL` | `muse-spark-1.3-contributor` | Chat model |
| `MAX_OUTPUT_TOKENS` | `1500` | Default output budget (floor: 512) |
| `FALLBACK_TIMEOUT` | `25` | Seconds to wait for Meta before Ollama |
| `OLLAMA_BASE_URL` | `http://localhost:${OLLAMA_PORT:-11434}/v1` | Fallback/embeddings endpoint |
| `OLLAMA_CHAT_MODEL` | `llama3.1` | Chat fallback model |
| `OLLAMA_EMBED_MODEL` | `nomic-embed-text` | Embeddings model |

## Verified API behavior

- Chat posts the Responses shape: system messages become `instructions`,
  user turns use `input_text` items, assistant turns use `output_text`
  items (the API rejects `input_text` on assistant messages), with
  `reasoning: {effort: "minimal"}`. Default reasoning effort burns the
  whole token budget before emitting text; `minimal` answers reliably.
- Requests carry a 512-token floor: below that, reasoning alone exhausts
  the budget and the API returns `incomplete` with no message. Larger
  caller budgets are always honored.
- Meta exposes no embeddings endpoint (`/v1/embeddings` is 404), so
  `embed` always delegates to the Ollama model above and fails loudly
  when Ollama is down — inventing vectors would poison embedding spaces.
- Chat falls back to Ollama once on any Meta error/timeout; when Ollama
  fails too, the original Meta error is returned, never masked.
- Health (`GET /projects/{id}/ai/test`) reports Meta reachability plus
  whether the Ollama fallback is live.

## Verifying

```bash
# Server log at startup names the live integrations (never values):
./start.sh
# -> LLM: Meta API selected via LLM_BACKEND=meta_api (Ollama fallback on error)

# Health for a project (mock projects route to Meta when configured):
curl -s localhost:8080/api/v1/projects/<id>/ai/test | head -c 400
```
