# R5 — AI Graph Debugger

`POST /projects/{id}/debug/trace` — `{question, strategy?, top_k?,
ai_hypothesis?}` — traces one question through the whole GraphRAG path and
diagnoses likely failure causes. Stored retrieval defaults apply when the
request omits them.

## Trace stages

1. Retrieval candidates via the shared retrieval service (same behavior as
   Q&A): passages, profile, model, dimensions.
2. Ranking decisions: per-passage fused score plus the underlying vector and
   keyword ranks.
3. Graph evidence: entities carrying each chunk's provenance (`_source_file` /
   `_lineage_ref`), with sampled relationship types. Prose chunks without
   record lineage say so explicitly.
4. Ontology mapping: which linked labels map to approved/latest ontology
   entities and their relationships.
5. Final context: evidence size plus a truncated preview of exactly what the
   model received, with the prompt version.
6. Answer: response text, grounded flag, resolved citations, and the count of
   hallucinated markers (citations pointing at passages never retrieved).

## Diagnosis

Deterministic rules always run: `DBG-E001` (no evidence), `DBG-W001`
(vocabulary mismatch), `DBG-W002`/`W003` (missing/hallucinated citations),
`DBG-W004`/`W005` (extraction coverage), `DBG-W006` (duplicate entities),
`DBG-W007` (no approved ontology), plus info rules. `likely_causes` ranks the
implicated areas. An optional AI second opinion (`ai_hypothesis`, on by
default) adds a natural-language hypothesis labeled with its model; provider
failures degrade to rules-only with an explicit note, never silence.

## CLI

`relating debug trace --project ID --question "..." [--strategy S] [--no-ai]`
prints the trace summary, causes, and rules (or full JSON with `--format json`).
