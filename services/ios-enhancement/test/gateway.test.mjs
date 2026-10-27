import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHandler } from '../worker.mjs';

const ID = '550e8400-e29b-41d4-a716-446655440000';
const TOKEN = 'synthetic-test-token-that-is-at-least-32-characters';
const env = { CLIENT_TOKEN: TOKEN, ANTHROPIC_API_KEY: 'synthetic-provider-secret', ANTHROPIC_MODEL: 'synthetic-model' };
const limiter = async () => ({ allowed: true });
function post(body = { requestId: ID, text: '今天用 Swift 写 3 个函数，不要修改 API。' }, headers = {}) {
  return new Request('https://example.test/v1/enhance', { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${TOKEN}`, ...headers }, body: JSON.stringify(body) });
}
function provider(text, options = {}) {
  return async (_url, init) => {
    options.inspect?.(JSON.parse(init.body), init.headers);
    return Response.json({ stop_reason: 'end_turn', content: [{ type: 'text', text }], ...options.body }, { status: options.status ?? 200 });
  };
}
test('health reveals availability only and is usable without credentials', async () => {
  const h = createHandler({ limiter });
  const r = await h(new Request('https://example.test/health'), {});
  assert.equal(r.status, 200);
  assert.deepEqual(await r.json(), { status: 'ok' });
  assert.equal(r.headers.get('cache-control'), 'no-store');
});
test('auth fails before consuming provider budget', async () => {
  let calls = 0;
  const h = createHandler({ fetchImpl: async () => { calls++; }, limiter });
  assert.equal((await h(post(undefined, { authorization: 'Bearer wrong' }), env)).status, 401);
  assert.equal(calls, 0);
});
test('missing config or global limiter fails closed', async () => {
  const h = createHandler({ fetchImpl: provider('should not call') });
  assert.equal((await h(post(), env)).status, 503);
  assert.equal((await createHandler({ limiter })(post(), { ...env, CLIENT_TOKEN: '' })).status, 503);
  assert.equal((await createHandler({ limiter })(post(), { ...env, ANTHROPIC_API_KEY: '' })).status, 503);
  assert.equal((await createHandler({ limiter })(post(), { ...env, ANTHROPIC_MODEL: '' })).status, 503);
});
test('only strict UUID+text objects are accepted; audio and API overrides rejected', async () => {
  let calls = 0;
  const h = createHandler({ fetchImpl: async () => { calls++; }, limiter });
  for (const body of [null, [], {}, { requestId: ID, text: '' }, { requestId: ID, text: '  ' }, { requestId: 'bad', text: 'hi' }, { requestId: ID, text: 'hi', audio: 'base64' }, { requestId: ID, text: 'hi', model: 'override' }]) {
    assert.equal((await h(post(body), env)).status, 400);
  }
  assert.equal(calls, 0);
});
test('limits Unicode characters and bytes, not UTF16 halves or content-length claims', async () => {
  const h = createHandler({ fetchImpl: provider('hi'), limiter });
  assert.equal((await h(post({ requestId: ID, text: '中'.repeat(8001) }), env)).status, 413);
  const request = new Request('https://example.test/v1/enhance', { method: 'POST', headers: { authorization: `Bearer ${TOKEN}`, 'content-type': 'application/json' }, body: ' '.repeat(65537) });
  assert.equal((await h(request, env)).status, 413);
  const valid = await h(post({ requestId: ID, text: '🙂'.repeat(8000) }), env);
  assert.equal(valid.status, 200);
});
test('rejects invalid JSON and inappropriate media types', async () => {
  const h = createHandler({ limiter });
  assert.equal((await h(new Request('https://example.test/v1/enhance', { method: 'POST', headers: { authorization: `Bearer ${TOKEN}`, 'content-type': 'application/json' }, body: '{' }), env)).status, 400);
  assert.equal((await h(post(undefined, { 'content-type': 'audio/wav' }), env)).status, 415);
});
test('rate limit denies before calling provider', async () => {
  let calls = 0;
  const h = createHandler({ limiter: async () => ({ allowed: false }), fetchImpl: async () => { calls++; } });
  const r = await h(post(), env);
  assert.equal(r.status, 429);
  assert.equal(calls, 0);
  assert.equal(r.headers.get('retry-after'), '60');
});
test('provider sees text but no client token; response matches request id', async () => {
  const text = '今天用 Swift 写 3 个函数。不要修改 API。';
  const h = createHandler({ limiter, fetchImpl: provider(text, { inspect: (body, headers) => {
    assert.equal(body.model, env.ANTHROPIC_MODEL);
    assert.equal(body.messages.length, 1);
    assert.equal(typeof body.system, 'string');
    assert.ok(!JSON.stringify(body).includes(TOKEN));
    assert.ok(!JSON.stringify(body).includes(ID));
    assert.equal(headers['x-api-key'], env.ANTHROPIC_API_KEY);
    assert.equal(headers['anthropic-version'], '2023-06-01');
  } }) });
  const r = await h(post(), env);
  assert.equal(r.status, 200);
  assert.deepEqual(await r.json(), { requestId: ID, text });
});
test('provider errors never expose body, keys or stack and preserve offline text', async () => {
  for (const fetchImpl of [async () => { throw new Error('secret error'); }, provider('provider-secret', { status: 401 })]) {
    const r = await createHandler({ limiter, fetchImpl })(post(), env);
    assert.equal(r.status, 200);
    assert.deepEqual(await r.json(), { requestId: ID, text: '今天用 Swift 写 3 个函数，不要修改 API。' });
  }
});
test('provider has a hard deadline even if fetch ignores AbortSignal', async () => {
  const h = createHandler({ limiter, timeoutMs: 15, fetchImpl: async () => new Promise(() => {}) });
  const start = performance.now();
  const r = await h(post(), env);
  assert.equal(r.status, 200);
  assert.ok(performance.now() - start < 500);
  assert.equal((await r.json()).text, '今天用 Swift 写 3 个函数，不要修改 API。');
});
test('unsafe edits to numbers, negation or English/code tokens are discarded', async () => {
  const raw = '不要将 UserID 从 12 改为 13，URL 是 https://example.com/a。';
  for (const candidate of [raw.replace('12', '14'), raw.replace('不要', '请'), raw.replace('UserID', 'user_id'), raw.replace('/a', '/b'), '好的，以下是整理后的文本：' + raw]) {
    const r = await createHandler({ limiter, fetchImpl: provider(candidate) })(post({ requestId: ID, text: raw }), env);
    assert.equal((await r.json()).text, raw);
  }
});
test('truncated, empty or implausible output is discarded', async () => {
  const raw = '今天我们一起讨论一下这个方案，明天继续。';
  for (const [text, body] of [['好的', {}], ['', {}], ['好'.repeat(1000), {}], [raw, { stop_reason: 'max_tokens' }]]) {
    const r = await createHandler({ limiter, fetchImpl: provider(text, { body }) })(post({ requestId: ID, text: raw }), env);
    assert.equal((await r.json()).text, raw);
  }
});
test('canceled request never starts a paid call', async () => {
  let calls = 0;
  const controller = new AbortController(); controller.abort();
  const request = new Request(post(), { signal: controller.signal });
  const r = await createHandler({ limiter, fetchImpl: async () => { calls++; } })(request, env);
  assert.equal(r.status, 499);
  assert.equal(calls, 0);
});
test('unknown routes and methods do not contact provider', async () => {
  const h = createHandler({ limiter });
  assert.equal((await h(new Request('https://example.test/other'), env)).status, 404);
  assert.equal((await h(new Request('https://example.test/v1/enhance'), env)).status, 405);
});
