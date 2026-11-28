# @relating/sdk — TypeScript client for the Relating graph API

Ergonomic `fetch`-based client over `/api/v1`, extracted from the workbench
client and extended with auth, grants, audit, connectors, and continuous
sync. Works in browsers (relative `/api/v1`) and Node 18+ (pass a full
`baseUrl`). Zero runtime dependencies.

## Install

```bash
npm install ../..  # from this directory, or copy it into your project
```

No install step is needed to build or test: the repo's workbench
TypeScript compiles it (`../../web/node_modules/.bin/tsc -p tsconfig.json`).

## Quickstart

```ts
import { RelatingClient } from '@relating/sdk';

const client = new RelatingClient({ baseUrl: 'http://127.0.0.1:8080/api/v1' });
const project = await client.createProject({ name: 'demo' });

// Backend flip: only RELATING_TARGET changes at cutover.
const result = await client.runCypherRead(
  project.id,
  'MATCH (n:Person) RETURN n LIMIT 5',
  {},
  undefined,
  process.env.RELATING_TARGET ?? 'relate://local' // or 'aura://<cid>'
);
console.log(result.row_count);
```

Token resolution: explicit `token` option, else `RELATING_API_TOKEN` in Node.

## Errors

Failures throw `RelatingApiError` with `.status`, `.code`, and `.message`
straight from the server's `{error: {code, message}}` envelope.

```ts
import { RelatingApiError } from '@relating/sdk';

try {
  await client.getProject('missing');
} catch (error) {
  if (error instanceof RelatingApiError && error.status === 404) {
    console.log(error.code, error.message);
  }
}
```

## Build and test

```bash
npm run build  # tsc (uses the workbench compiler; no extra install)
npm test       # build + dependency-free test harness on node
```
