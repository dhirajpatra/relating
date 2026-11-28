import {
  RelatingClient,
  RelatingApiError,
  parseTarget,
  defaultTarget,
  LOCAL_TARGET,
} from '../src/index.js';

// Minimal harness: no node:test dependency, runnable with plain `node`.
type Case = { name: string; run: () => void | Promise<void> };
const cases: Case[] = [];
function test(name: string, run: () => void | Promise<void>): void {
  cases.push({ name, run });
}
function ok(value: unknown, message?: string): void {
  if (!value) throw new Error(message ?? 'expected truthy value');
}
function equal<T>(actual: T, expected: T, message?: string): void {
  const left = JSON.stringify(actual);
  const right = JSON.stringify(expected);
  if (left !== right) {
    throw new Error(message ?? `expected ${right}, got ${left}`);
  }
}
function env(): Record<string, string | undefined> {
  const holder = globalThis as unknown as Record<string, { env: Record<string, string | undefined> }>;
  return holder['process'].env;
}

function stubFetch(handler: (url: string, init?: RequestInit) => unknown): typeof fetch {
  return ((url: unknown, init?: RequestInit) => {
    const result = handler(String(url), init);
    return Promise.resolve({
      ok: true,
      status: 200,
      json: () => Promise.resolve(result),
    });
  }) as unknown as typeof fetch;
}

function failingFetch(status: number, payload: unknown): typeof fetch {
  return (() =>
    Promise.resolve({
      ok: false,
      status,
      json: () => Promise.resolve(payload),
    })) as unknown as typeof fetch;
}

test('parseTarget maps local/ aura and rejects junk', () => {
  equal(parseTarget('relate://local'), undefined);
  equal(parseTarget('aura://cid-1'), 'cid-1');
  for (const bad of ['', 'bolt://x', 'aura://']) {
    let threw = false;
    try {
      parseTarget(bad);
    } catch {
      threw = true;
    }
    ok(threw, `expected ${bad} to throw`);
  }
});

test('defaultTarget reads RELATING_TARGET', () => {
  const saved = env().RELATING_TARGET;
  env().RELATING_TARGET = 'aura://env-c';
  equal(defaultTarget(), 'aura://env-c');
  if (saved === undefined) delete env().RELATING_TARGET;
  else env().RELATING_TARGET = saved;
  equal(defaultTarget(), LOCAL_TARGET);
});

test('client sends bearer auth and JSON bodies', async () => {
  let seen = { url: '', init: undefined as RequestInit | undefined };
  const client = new RelatingClient({
    baseUrl: 'http://127.0.0.1:9/api/v1',
    token: 'test-token',
    fetchImpl: stubFetch((url, init) => {
      seen = { url, init };
      return { ok: true };
    }),
  });
  await client.createProject({ name: 'demo' });
  equal(seen.url, 'http://127.0.0.1:9/api/v1/projects');
  const headers = seen.init?.headers as Record<string, string>;
  equal(headers['Authorization'], 'Bearer test-token');
  equal(seen.init?.method, 'POST');
  equal(JSON.parse(seen.init?.body as string), { name: 'demo' });
});

test('client flips targets into connection_id', async () => {
  const bodies: unknown[] = [];
  const client = new RelatingClient({
    baseUrl: 'http://127.0.0.1:9/api/v1',
    fetchImpl: stubFetch((_url, init) => {
      bodies.push(JSON.parse((init?.body as string) ?? '{}'));
      return { rows: [] };
    }),
  });
  await client.runCypherRead('pid', 'MATCH (n) RETURN n', { a: 1 }, undefined, 'aura://c1');
  await client.runCypherRead('pid', 'MATCH (n) RETURN n', undefined, undefined, 'relate://local');
  equal((bodies[0] as Record<string, unknown>).connection_id, 'c1');
  equal((bodies[1] as Record<string, unknown>).connection_id, undefined);
});

test('client raises typed errors preserving code and status', async () => {
  const client = new RelatingClient({
    baseUrl: 'http://127.0.0.1:9/api/v1',
    fetchImpl: failingFetch(404, { error: { code: 'NOT_FOUND', message: 'gone' } }),
  });
  try {
    await client.listProjects();
    throw new Error('expected listProjects to throw');
  } catch (error) {
    ok(error instanceof RelatingApiError, 'expected RelatingApiError');
    const typed = error as RelatingApiError;
    equal(typed.status, 404);
    equal(typed.code, 'NOT_FOUND');
    ok(typed.message.includes('gone'), typed.message);
  }
});

test('client builds migration sync and changes URLs', async () => {
  const urls: string[] = [];
  const client = new RelatingClient({
    baseUrl: 'http://127.0.0.1:9/api/v1',
    fetchImpl: stubFetch((url) => {
      urls.push(url);
      return { ok: true };
    }),
  });
  await client.syncMigration('p', 'm', true);
  await client.changes('p', '2026-01-01T00:00:00Z', 10);
  ok(urls[0].endsWith('/projects/p/migrations/m/sync?propagate_deletes=true'), urls[0]);
  ok(urls[1].includes('/projects/p/changes?'), urls[1]);
  ok(urls[1].includes('limit=10'), urls[1]);
});

const failures: string[] = [];
for (const { name, run } of cases) {
  try {
    await run();
    console.log(`ok - ${name}`);
  } catch (error) {
    failures.push(name);
    console.error(`FAIL - ${name}: ${(error as Error).message}`);
  }
}
if (failures.length > 0) {
  console.error(`${failures.length} failing test(s)`);
  const holder = globalThis as unknown as Record<string, { exitCode: number }>;
  holder['process'].exitCode = 1;
}
