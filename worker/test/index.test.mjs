import { test } from 'node:test';
import assert from 'node:assert/strict';
import worker from '../build/index.js';

const uploaded = new Date('2026-11-18T03:00:00Z');

// The R2 binding's behavior that the Worker relies on: a missing key is null,
// and a failed precondition returns the object without a body.
function bucket(objects) {
  const reads = [];
  return {
    reads,
    async get(key, { onlyIf } = {}) {
      reads.push(key);
      if (!(key in objects)) return null;
      const bytes = new TextEncoder().encode(objects[key]);
      const etag = `e-${key.length}`;
      const object = { key, size: bytes.length, etag, httpEtag: `"${etag}"`, uploaded };
      // Every validator is evaluated, so the Worker must pass only the ones it means.
      const header = (name) => onlyIf?.get(name) ?? null;
      const tags = (value) => value.split(',').map((tag) => tag.trim().replace(/^W\//, ''));
      const matches = (value) => tags(value).some((tag) => tag === object.httpEtag || tag === '*');
      const seconds = Math.floor(uploaded / 1000);
      const failed = (header('if-none-match') !== null && matches(header('if-none-match')))
        || (header('if-modified-since') !== null && seconds <= Date.parse(header('if-modified-since')) / 1000)
        || (header('if-match') !== null && !matches(header('if-match')))
        || (header('if-unmodified-since') !== null && seconds > Date.parse(header('if-unmodified-since')) / 1000);
      if (failed) return object;
      const body = new Blob([bytes]).stream();
      return { ...object, body };
    },
  };
}

function limiter(allowed = Infinity) {
  const keys = [];
  return { keys, async limit({ key }) { keys.push(key); return { success: keys.length <= allowed }; } };
}

const files = {
  'feed/manifest.json': '{"format":"1.0"}',
  'feed/manifest.json.sig': '{"key_id":"k","signature":"s"}',
  'feed/segments/run-1-5-0123456789abcdef.jsonl.zst': 'records',
  'feed/segments/run-1-5-fedcba9876543210.vec': 'vectors',
  'feed/snapshots/40-0123456789abcdef.jsonl.zst': 'snapshot',
  'test-feed/manifest.json': 'test',
  'state/20261118T000000Z.sqlite3.zst': 'state',
};

function env(overrides = {}) {
  return { FEED: bucket(files), LIMITER: limiter(), PREFIX: 'feed/', ...overrides };
}

function get(path, init = {}, e = env()) {
  return worker.fetch(new Request(`https://rhythm-feed.example${path}`, init), e);
}

test('serves each feed file with its type and caching', async () => {
  const cases = [
    ['/manifest.json', 'application/json', 'no-cache', '{"format":"1.0"}'],
    ['/manifest.json.sig', 'application/json', 'no-cache', '{"key_id":"k","signature":"s"}'],
    ['/segments/run-1-5-0123456789abcdef.jsonl.zst', 'application/zstd', 'public, max-age=31536000, immutable', 'records'],
    ['/segments/run-1-5-fedcba9876543210.vec', 'application/octet-stream', 'public, max-age=31536000, immutable', 'vectors'],
    ['/snapshots/40-0123456789abcdef.jsonl.zst', 'application/zstd', 'public, max-age=31536000, immutable', 'snapshot'],
  ];
  for (const [path, type, cache, body] of cases) {
    const response = await get(path);
    assert.equal(response.status, 200, path);
    assert.equal(response.headers.get('content-type'), type, path);
    assert.equal(response.headers.get('cache-control'), cache, path);
    assert.match(response.headers.get('etag'), /^"e-\d+"$/, path);
    assert.equal(response.headers.get('last-modified'), 'Wed, 18 Nov 2026 03:00:00 GMT', path);
    assert.equal(response.headers.get('access-control-allow-origin'), null, path);
    assert.equal(await response.text(), body, path);
  }
});

test('answers only GET and HEAD', async () => {
  for (const method of ['POST', 'PUT', 'DELETE', 'PATCH', 'OPTIONS']) {
    const e = env();
    const response = await get('/manifest.json', { method }, e);
    assert.equal(response.status, 405, method);
    assert.equal(response.headers.get('allow'), 'GET, HEAD');
    assert.deepEqual(e.FEED.reads, [], method);
  }
});

test('HEAD returns the headers and length without a body', async () => {
  const response = await get('/manifest.json', { method: 'HEAD' });
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('content-length'), '16');
  assert.equal(response.headers.get('content-type'), 'application/json');
  assert.equal(response.body, null);
});

test('refuses every path outside the feed files', async () => {
  const paths = [
    '/', '/manifest', '/manifest.json/', '/MANIFEST.json', '/segments/', '/segments',
    '/segments/run.txt', '/segments/a/b.vec', '/segments//run.vec', '//manifest.json',
    '/segments/..%2Fmanifest.json', '/segments/%2E%2E.vec', '/segments/run%2Fx.vec',
    '/segments/.vec', '/segments/..vec', '/other/run.vec', '/state/20261118T000000Z.sqlite3.zst',
    '/test-feed/manifest.json', '/feed/manifest.json',
  ];
  for (const path of paths) {
    const e = env();
    const response = await get(path, {}, e);
    assert.equal(response.status, 404, path);
    assert.deepEqual(e.FEED.reads, [], path);
  }
  // The URL parser resolves dot segments, so these stay inside the feed.
  const e = env();
  assert.equal((await get('/segments/../manifest.json', {}, e)).status, 200);
  assert.deepEqual(e.FEED.reads, ['feed/manifest.json']);
});

test('reads only under its prefix', async () => {
  const e = env({ PREFIX: 'test-feed/' });
  const response = await get('/manifest.json', {}, e);
  assert.equal(await response.text(), 'test');
  assert.deepEqual(e.FEED.reads, ['test-feed/manifest.json']);
});

test('a missing file is 404', async () => {
  assert.equal((await get('/segments/run-6-9-0000000000000000.jsonl.zst')).status, 404);
});

test('revalidation returns 304 without a body', async () => {
  const first = await get('/manifest.json');
  const etag = first.headers.get('etag');
  for (const headers of [
    { 'if-none-match': etag },
    { 'if-none-match': `W/${etag}` },
    { 'if-none-match': `"other", ${etag}` },
    { 'if-modified-since': 'Wed, 18 Nov 2026 03:00:00 GMT' },
  ]) {
    for (const method of ['GET', 'HEAD']) {
      const response = await get('/manifest.json', { method, headers });
      assert.equal(response.status, 304, JSON.stringify(headers));
      assert.equal(response.body, null);
      assert.equal(response.headers.get('etag'), etag);
      assert.equal(response.headers.get('cache-control'), 'no-cache');
    }
  }
  // A changed file, or an If-None-Match that wins over If-Modified-Since.
  for (const headers of [
    { 'if-none-match': '"other"' },
    { 'if-modified-since': 'Wed, 18 Nov 2026 02:59:59 GMT' },
    { 'if-none-match': '"other"', 'if-modified-since': 'Wed, 18 Nov 2026 03:00:00 GMT' },
  ]) {
    const response = await get('/manifest.json', { headers });
    assert.equal(response.status, 200, JSON.stringify(headers));
    assert.equal(await response.text(), '{"format":"1.0"}');
  }
});

test('ignores preconditions other than revalidation', async () => {
  const response = await get('/manifest.json', { headers: { 'if-match': '"other"', 'if-unmodified-since': 'Thu, 01 Jan 2026 00:00:00 GMT' } });
  assert.equal(response.status, 200);
});

test('limits requests per client address before reading', async () => {
  const e = env({ LIMITER: limiter(2) });
  const headers = { 'cf-connecting-ip': '203.0.113.7' };
  assert.equal((await get('/manifest.json', { headers }, e)).status, 200);
  assert.equal((await get('/manifest.json.sig', { headers }, e)).status, 200);
  const limited = await get('/manifest.json', { headers }, e);
  assert.equal(limited.status, 429);
  assert.equal(limited.headers.get('retry-after'), '60');
  assert.equal(limited.body, null);
  assert.deepEqual(e.LIMITER.keys, ['203.0.113.7', '203.0.113.7', '203.0.113.7']);
  assert.equal(e.FEED.reads.length, 2);
});
