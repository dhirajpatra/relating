# R2 — Schema-as-Code and Versioning

Base path: `/api/v1`. The schema document (`relating-schema-v1`) is the
version-controlled contract for a project: ontology, unique constraints,
indexes, embedding profiles, and retrieval defaults.

File exports are JSON (`.json`, `.cypher`) rather than the YAML sketches in
early design notes: identical information, no new dependencies.

## Export

`GET /projects/{id}/schema/export` builds the live contract and returns five
committable files plus the parsed document:

- `graph/schema.json` — the full document (source of truth).
- `graph/ontology.json` — entities and relationships.
- `graph/constraints.cypher` — generated unique-constraint DDL (Neo4j 5).
- `graph/indexes.cypher` — generated index DDL.
- `tests/graph_contracts.json` — machine-readable contract checks (feeds R4 CI).

## Revisions

- `POST /projects/{id}/schema/revisions` — snapshot live state, or store a
  supplied `{document}`. Idempotent: identical content returns the existing
  revision with `created: false` instead of a new version.
- `GET .../revisions`, `GET .../revisions/{version}`.

## Diff and proposals

`POST /projects/{id}/schema/diff` — `{from_version?, to_version?, to_document?}`.
`from` defaults to the latest revision; `to` defaults to the live state.
Returns classified changes plus a migration proposal:

- Breaking: removed labels/relationships/properties, type changes,
  primary-key flips, removed constraints, embedding model/dimension changes.
- Safe: pure additions, index changes, retrieval tuning.
- The proposal carries a suggested `VNNN__...cypher` filename, additive
  idempotent DDL, and ordered manual steps for every breaking change.

## Apply

`POST /projects/{id}/schema/apply` — `{document}`. Validates, diffs against
the previous revision, conformance-checks the live graph (sampled property
presence/types plus uniqueness spot-checks), and records the revision.
Recording never hides non-conformance: the report carries `passed` plus
per-check details. Blocking on failure is R4 CI's job.

## Retrieval defaults

`GET` / `PUT /projects/{id}/retrieval-config` — `{default_strategy,
default_top_k}`. Retrieval, Q&A, and evaluation endpoints use these when the
request omits them; explicit per-request values always win.
