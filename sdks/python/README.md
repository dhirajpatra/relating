# relating — Python SDK for the Relating graph API

Zero-dependency client (stdlib `urllib` only) over `/api/v1`: projects,
sources, ontologies, ingestion, GraphRAG, Aura migration, and data
connectors. Authentication reads `RELATING_API_TOKEN`; query targets read
`RELATING_TARGET`.

## Install

```bash
pip install ./sdks/python
# or, without installing:
export PYTHONPATH="$PWD/sdks/python/src"
```

## Quickstart

```python
import os
from relating import RelatingClient

client = RelatingClient()  # token from RELATING_API_TOKEN
project = client.create_project("demo")
print(project["id"])

# Backend flip: only RELATING_TARGET changes at cutover.
TARGET = os.environ.get("RELATING_TARGET", "relate://local")  # or "aura://<cid>"
result = client.cypher_read(project["id"], "MATCH (n:Person) RETURN n LIMIT 5", target=TARGET)
print(result["row_count"])
```

## Errors

Failures raise `RelatingError` (with `.status`, `.code`, `.message`),
narrowed to `AuthenticationError` (401/403), `NotFoundError` (404),
`ValidationError` (400), `UpstreamError` (502), and `RateLimitedError`
(429).

```python
from relating import NotFoundError

try:
    client.get_project("missing")
except NotFoundError as exc:
    print(exc.code, exc.message)
```

## Full migration flow

```python
analysis = client.migration_analyze(pid, connection_id=cid, aura_tier="free")
plan = client.migration_plan(pid, connection_id=cid)
mid = plan["migration"]["id"]
client.migration_dry_run(pid, mid)
client.migration_approve(pid, mid, True)
client.migration_execute(pid, mid)
report = client.migration_verify(pid, mid)
assert report["passed"]
```

## Tests

```bash
cd sdks/python && python3 -m unittest discover -s tests -v
```
