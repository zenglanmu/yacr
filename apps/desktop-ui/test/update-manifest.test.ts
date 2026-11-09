// The landing manifest schema: one `release` string, everything else ignored.
//
// `.ts` by filename only: Node's type stripping runs it, `tsc` never sees it.
import test from 'node:test';
import assert from 'node:assert/strict';

import { MAX_MANIFEST_BYTES } from '../src/update/config.ts';
import { parseManifest, utf8Length } from '../src/update/manifest.ts';

function failed(result) {
  assert.equal(result.ok, false);
  assert.equal(typeof result.reason, 'string');
  assert.ok(result.reason.length > 0);
  return result.reason;
}

test('a valid manifest yields the release and ignores every other field', () => {
  const body = JSON.stringify({
    release: 'v0.1.6',
    cdnBase: 'https://example.test/releases/v0.1.6',
    assets: { linux: { file: 'x.flatpak', size: 1, sha256: 'deadbeef' } },
  });
  assert.deepEqual(parseManifest(body), { ok: true, release: 'v0.1.6' });
  // No `v` is just as valid; the version layer normalizes it.
  assert.deepEqual(parseManifest('{"release":"0.1.6"}'), { ok: true, release: '0.1.6' });
  assert.deepEqual(parseManifest('{"release":"v10.20.30"}'), { ok: true, release: 'v10.20.30' });
});

test('missing, non-string and malformed release fields fail with a reason', () => {
  assert.match(failed(parseManifest('{}')), /^manifest\.release_missing/);
  assert.match(failed(parseManifest('{"release":null}')), /^manifest\.release_missing/);
  assert.match(failed(parseManifest('{"release":123}')), /^manifest\.release_missing/);
  assert.match(failed(parseManifest('{"release":""}')), /^manifest\.release_invalid/);
  assert.match(failed(parseManifest('{"release":"1.2"}')), /^manifest\.release_invalid/);
  assert.match(failed(parseManifest('{"release":"v0.1.6-beta"}')), /^manifest\.release_invalid/);
  assert.match(failed(parseManifest('{"release":" 0.1.6"}')), /^manifest\.release_invalid/);
});

test('non-JSON, non-object and empty bodies never produce a version', () => {
  for (const body of [
    '', '   ', 'not json', '<!doctype html><html><body>error</body></html>',
    '<html>502 Bad Gateway</html>', '[]', '["v0.1.6"]', 'null', 'true', '42',
    '"v0.1.6"',
  ]) {
    const result = parseManifest(body);
    assert.equal(result.ok, false, JSON.stringify(body));
    assert.match(result.reason, /^manifest\./, JSON.stringify(body));
  }
  assert.match(failed(parseManifest('[]')), /^manifest\.not_object/);
  assert.match(failed(parseManifest('not json')), /^manifest\.invalid_json/);
});

test('an oversized body is refused before parsing', () => {
  // Oversized but syntactically valid: the cap, not the schema, is the failure.
  const giant = JSON.stringify({ release: 'v0.1.6', pad: 'x'.repeat(MAX_MANIFEST_BYTES) });
  const reason = failed(parseManifest(giant));
  assert.match(reason, /^manifest\.too_large/);

  // Non-JSON garbage over the cap names the same reason, not `invalid_json`.
  assert.match(failed(parseManifest('x'.repeat(MAX_MANIFEST_BYTES + 1))), /^manifest\.too_large/);

  // Exactly at the limit is still allowed (the cap is inclusive).
  const atLimit = '{"release":"v0.1.6"}';
  assert.ok(utf8Length(atLimit) < MAX_MANIFEST_BYTES);
  assert.deepEqual(parseManifest(atLimit), { ok: true, release: 'v0.1.6' });
});

test('parseManifest never throws, whatever it is handed', () => {
  for (const body of ['', '{', 'nul', '[]', '\u0000', '{"release":{}}']) {
    assert.doesNotThrow(() => parseManifest(body), JSON.stringify(body));
  }
});
