// The one manifest read: cache, abort budget, status and size gates, and the
// promise that it never throws.
//
// `.ts` by filename only: Node's type stripping runs it, `tsc` never sees it.
import test from 'node:test';
import assert from 'node:assert/strict';

import { MAX_MANIFEST_BYTES, RELEASE_MANIFEST_URL } from '../src/update/config.ts';
import { fetchLatestRelease } from '../src/update/source.ts';

function headers(map = {}) {
  const values = new Map(Object.entries(map).map(([key, value]) => [key.toLowerCase(), String(value)]));
  return { get(name) { return values.get(String(name).toLowerCase()) ?? null; } };
}

function response({ status = 200, body = '', headerMap = {} } = {}) {
  let textCalls = 0;
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: headers(headerMap),
    async text() { textCalls += 1; return body; },
    get textCalls() { return textCalls; },
  };
}

test('a 2xx manifest is read with no-store and a signal', async () => {
  const calls = [];
  const answer = response({ body: '{"release":"v0.1.6","assets":{}}' });
  const result = await fetchLatestRelease({
    timeout: 100,
    fetch: async (url, init) => { calls.push([url, init]); return answer; },
  });

  assert.deepEqual(result, { ok: true, release: 'v0.1.6' });
  assert.equal(calls.length, 1);
  const [url, init] = calls[0];
  assert.equal(url, RELEASE_MANIFEST_URL);
  assert.equal(init.cache, 'no-store');
  assert.ok(init.signal instanceof AbortSignal);
});

test('a non-2xx answer is a reason, and the body is not parsed', async () => {
  const answer = response({ status: 503, body: '{"release":"v9.9.9"}' });
  const result = await fetchLatestRelease({ timeout: 100, fetch: async () => answer });
  assert.equal(result.ok, false);
  assert.match(result.reason, /^update\.http_status: 503/);
  assert.equal(answer.textCalls, 0, 'no version is read out of an error body');
});

test('a declared Content-Length over the cap is refused before reading', async () => {
  const answer = response({
    headerMap: { 'Content-Length': MAX_MANIFEST_BYTES + 1 },
    body: '{"release":"v0.1.6"}',
  });
  const result = await fetchLatestRelease({ timeout: 100, fetch: async () => answer });
  assert.equal(result.ok, false);
  assert.match(result.reason, /^update\.manifest_too_large/);
  assert.equal(answer.textCalls, 0);
});

test('a decoded body over the cap is refused even without Content-Length', async () => {
  const body = JSON.stringify({ release: 'v0.1.6', pad: 'x'.repeat(MAX_MANIFEST_BYTES) });
  const result = await fetchLatestRelease({ timeout: 100, fetch: async () => response({ body }) });
  assert.equal(result.ok, false);
  assert.match(result.reason, /^update\.manifest_too_large/);
});

test('an unusable Content-Length is ignored instead of skipping a good body', async () => {
  const answer = response({
    headerMap: { 'content-length': 'not-a-number' },
    body: '{"release":"0.1.6"}',
  });
  assert.deepEqual(
    await fetchLatestRelease({ timeout: 100, fetch: async () => answer }),
    { ok: true, release: '0.1.6' },
  );
});

test('a hanging fetch is aborted at the timeout and reported as a timeout', async () => {
  const fetch = (url, init) => new Promise((resolve, reject) => {
    init.signal.addEventListener('abort', () => reject(new Error('aborted by the client')));
  });
  const started = Date.now();
  const result = await fetchLatestRelease({ fetch, timeout: 10 });
  assert.equal(result.ok, false);
  assert.match(result.reason, /^update\.timeout/);
  assert.ok(Date.now() - started < 2_000, 'no unbounded wait');
});

test('a transport throw, an invalid response and a parse failure stay reasons', async () => {
  assert.match(
    (await fetchLatestRelease({ fetch: () => { throw new Error('offline'); }, timeout: 100 })).reason,
    /^update\.fetch_failed: offline/,
  );
  assert.match(
    (await fetchLatestRelease({ fetch: async () => null, timeout: 100 })).reason,
    /^update\.invalid_response/,
  );
  assert.match(
    (await fetchLatestRelease({ fetch: async () => response({ body: '<html>nope</html>' }), timeout: 100 })).reason,
    /^manifest\.invalid_json/,
  );
  assert.match(
    (await fetchLatestRelease({ fetch: async () => response({ body: '' }), timeout: 100 })).reason,
    /^manifest\.invalid_json/,
  );
});

test('a rejecting text() is a fetch failure, not an unhandled rejection', async () => {
  const answer = {
    ok: true,
    status: 200,
    headers: headers(),
    text: async () => { throw new Error('stream reset'); },
  };
  const result = await fetchLatestRelease({ fetch: async () => answer, timeout: 100 });
  assert.equal(result.ok, false);
  assert.match(result.reason, /^update\.fetch_failed: stream reset/);
});
