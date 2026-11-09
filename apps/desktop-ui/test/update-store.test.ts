// The update store: one check per session, every failure modelled, and only a
// genuinely newer release becomes a notice.
//
// `.ts` by filename only: Node's type stripping runs it, `tsc` never sees it.
import test, { beforeEach } from 'node:test';
import assert from 'node:assert/strict';

import {
  dismissUpdate,
  getUpdateStatus,
  openReleasePage,
  resetUpdateStoreForTests,
  skipUpdateVersion,
  startUpdateCheck,
  subscribe,
  useUpdateStatus,
} from '../src/update/store.ts';
import {
  LAST_CHECK_KEY,
  SKIPPED_VERSION_KEY,
  lastCheckAt,
} from '../src/update/policy.ts';

const NOW = 1_700_000_000_000;

function memoryStorage(initial = {}) {
  const map = new Map(Object.entries(initial).map(([key, value]) => [key, String(value)]));
  return {
    map,
    getItem(key) { return map.has(key) ? map.get(key) : null; },
    setItem(key, value) { map.set(key, String(value)); },
    removeItem(key) { map.delete(key); },
  };
}

/** Let the fire-and-forget check drain its microtasks and one macrotask. */
async function settle() {
  await new Promise(resolve => setTimeout(resolve, 0));
}

function manifestResponse(release) {
  return {
    ok: true,
    status: 200,
    headers: { get: () => null },
    text: async () => JSON.stringify({ release }),
  };
}

function failureResponse(status) {
  return {
    ok: false,
    status,
    headers: { get: () => null },
    text: async () => '',
  };
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

beforeEach(() => {
  resetUpdateStoreForTests();
});

test('the hook exists and the store starts idle', () => {
  assert.equal(typeof useUpdateStatus, 'function');
  assert.deepEqual(getUpdateStatus(), { kind: 'idle' });
});

test('a carrier that cannot answer the version is unsupported, not up to date', async () => {
  let fetchCalls = 0;
  const storage = memoryStorage();
  startUpdateCheck({
    storage,
    now: () => NOW,
    invoke: async () => { throw new Error('unsupported-runtime: cad_app_version'); },
    fetch: async () => { fetchCalls += 1; return manifestResponse('v0.1.7'); },
  });
  await settle();

  assert.deepEqual(getUpdateStatus(), {
    kind: 'unsupported',
    reason: 'unsupported-runtime: cad_app_version',
  });
  assert.equal(fetchCalls, 0, 'no manifest is fetched when the version is unknown');
  assert.equal(lastCheckAt(0, storage), null);
});

test('any other version refusal is unavailable, carrying the verbatim reason', async () => {
  startUpdateCheck({
    storage: memoryStorage(),
    invoke: async () => { throw new Error('window.__TAURI_INTERNALS__ is missing'); },
    fetch: async () => manifestResponse('v0.1.7'),
  });
  await settle();
  assert.deepEqual(getUpdateStatus(), {
    kind: 'unavailable',
    reason: 'window.__TAURI_INTERNALS__ is missing',
  });
});

test('a garbage running version is unavailable', async () => {
  startUpdateCheck({
    storage: memoryStorage(),
    invoke: async () => 'not-a-version',
    fetch: async () => manifestResponse('v0.1.7'),
  });
  await settle();
  assert.equal(getUpdateStatus().kind, 'unavailable');
  assert.match(getUpdateStatus().reason, /^update\.current_version_invalid/);
});

test('a newer release is available, and the successful read is recorded', async () => {
  const storage = memoryStorage();
  const fetched = [];
  startUpdateCheck({
    storage,
    now: () => NOW,
    invoke: async () => 'v0.1.6',
    fetch: async () => { fetched.push('manifest'); return manifestResponse('v0.1.7'); },
  });
  await settle();

  assert.deepEqual(getUpdateStatus(), { kind: 'available', current: '0.1.6', latest: '0.1.7' });
  assert.equal(fetched.length, 1);
  assert.equal(lastCheckAt(0, storage), NOW, 'lastCheckAt is written after success');
});

test('an equal or older release is up to date', async () => {
  for (const release of ['v0.1.6', '0.1.6', 'v0.1.5']) {
    resetUpdateStoreForTests();
    startUpdateCheck({
      storage: memoryStorage(),
      invoke: async () => '0.1.6',
      fetch: async () => manifestResponse(release),
    });
    await settle();
    assert.deepEqual(getUpdateStatus(), { kind: 'up-to-date', current: '0.1.6' }, release);
  }
});

test('a newer release the user already skipped stays skipped', async () => {
  const storage = memoryStorage({ [SKIPPED_VERSION_KEY]: '0.1.7' });
  startUpdateCheck({
    storage,
    now: () => NOW,
    invoke: async () => '0.1.6',
    fetch: async () => manifestResponse('0.1.7'),
  });
  await settle();
  assert.deepEqual(getUpdateStatus(), { kind: 'skipped', current: '0.1.6', latest: '0.1.7' });
});

test('a skipped version does not suppress a newer one', async () => {
  const storage = memoryStorage({ [SKIPPED_VERSION_KEY]: 'v0.1.7' });
  startUpdateCheck({
    storage,
    now: () => NOW,
    invoke: async () => '0.1.6',
    fetch: async () => manifestResponse('v0.1.8'),
  });
  await settle();
  assert.deepEqual(getUpdateStatus(), { kind: 'available', current: '0.1.6', latest: '0.1.8' });
});

test('dismiss hides the notice for the session without changing the policy', async () => {
  const storage = memoryStorage();
  startUpdateCheck({
    storage,
    now: () => NOW,
    invoke: async () => '0.1.6',
    fetch: async () => manifestResponse('v0.1.7'),
  });
  await settle();
  assert.equal(getUpdateStatus().kind, 'available');

  dismissUpdate();
  assert.deepEqual(getUpdateStatus(), { kind: 'idle' });
  assert.equal(lastCheckAt(0, storage), NOW, 'dismissal does not forget the check');
  dismissUpdate();
  assert.deepEqual(getUpdateStatus(), { kind: 'idle' }, 'dismiss outside available is a no-op');
});

test('skip persists the normalized release and moves to skipped', async () => {
  const storage = memoryStorage();
  startUpdateCheck({
    storage,
    now: () => NOW,
    invoke: async () => 'v0.1.6',
    fetch: async () => manifestResponse('v0.1.7'),
  });
  await settle();
  skipUpdateVersion();

  assert.deepEqual(getUpdateStatus(), { kind: 'skipped', current: '0.1.6', latest: '0.1.7' });
  assert.equal(storage.map.get(SKIPPED_VERSION_KEY), '0.1.7');
});

test('a throttled startup asks the host but never fetches, and stays idle', async () => {
  let invokes = 0;
  let fetchCalls = 0;
  const storage = memoryStorage({ [LAST_CHECK_KEY]: NOW - 60_000 });
  startUpdateCheck({
    storage,
    now: () => NOW,
    invoke: async () => { invokes += 1; return '0.1.6'; },
    fetch: async () => { fetchCalls += 1; return manifestResponse('v0.1.7'); },
  });
  await settle();

  assert.equal(invokes, 1);
  assert.equal(fetchCalls, 0, 'the 24h throttle prevents the network read');
  assert.deepEqual(getUpdateStatus(), { kind: 'idle' });
  assert.equal(lastCheckAt(0, storage), NOW - 60_000, 'the old timestamp is untouched');
});

test('a network failure is unavailable and is not recorded as checked', async () => {
  const storage = memoryStorage();
  startUpdateCheck({
    storage,
    now: () => NOW,
    invoke: async () => '0.1.6',
    fetch: async () => failureResponse(500),
  });
  await settle();

  assert.equal(getUpdateStatus().kind, 'unavailable');
  assert.match(getUpdateStatus().reason, /^update\.http_status: 500/);
  assert.equal(lastCheckAt(0, storage), null, 'a failure must not suppress tomorrow\'s retry');
});

test('a malformed manifest is unavailable, never up to date', async () => {
  const storage = memoryStorage();
  startUpdateCheck({
    storage,
    now: () => NOW,
    invoke: async () => '0.1.6',
    fetch: async () => manifestResponse('0.0.0'),
  });
  await settle();
  assert.equal(getUpdateStatus().kind, 'unavailable');
  assert.match(getUpdateStatus().reason, /^update\.manifest_version_invalid/);
  // The read itself succeeded, so the throttle applies to the next startup.
  assert.equal(lastCheckAt(0, storage), NOW);
});

test('startUpdateCheck is idempotent for the session', async () => {
  let invokes = 0;
  const deps = {
    storage: memoryStorage(),
    now: () => NOW,
    invoke: async () => { invokes += 1; return '0.1.6'; },
    fetch: async () => manifestResponse('v0.1.7'),
  };
  startUpdateCheck(deps);
  startUpdateCheck(deps);
  await settle();

  assert.equal(invokes, 1);
  assert.equal(getUpdateStatus().kind, 'available');
});

test('a stale result cannot overwrite a newer check', async () => {
  const older = deferred();
  const newer = deferred();
  const storage = memoryStorage();

  startUpdateCheck({
    storage,
    now: () => NOW,
    invoke: () => older.promise,
    fetch: async () => manifestResponse('v9.9.9'),
  });
  // Before the older check answers, a reset + new check starts (the reset is the
  // test seam for "a newer check starts"; production never does this).
  resetUpdateStoreForTests();
  startUpdateCheck({
    storage,
    now: () => NOW,
    invoke: () => newer.promise,
    fetch: async () => manifestResponse('v0.1.7'),
  });

  older.resolve('0.1.6');
  await settle();
  newer.resolve('0.1.6');
  await settle();

  assert.deepEqual(getUpdateStatus(), { kind: 'available', current: '0.1.6', latest: '0.1.7' });
});

test('subscribers are notified on every status change and can unsubscribe', async () => {
  const seen = [];
  const off = subscribe(() => seen.push(getUpdateStatus().kind));
  startUpdateCheck({
    storage: memoryStorage(),
    now: () => NOW,
    invoke: async () => '0.1.6',
    fetch: async () => manifestResponse('v0.1.7'),
  });
  await settle();
  assert.ok(seen.includes('checking'));
  assert.equal(seen.at(-1), 'available');

  off();
  const before = seen.length;
  dismissUpdate();
  assert.equal(seen.length, before, 'no notification after unsubscribe');
});

test('openReleasePage reports success and the verbatim refusal', async () => {
  const commands = [];
  assert.deepEqual(
    await openReleasePage({ invoke: async command => { commands.push(command); return { accepted: true }; } }),
    { ok: true },
  );
  assert.deepEqual(commands, ['cad_open_release_page']);

  assert.deepEqual(
    await openReleasePage({ invoke: async () => { throw new Error('update.popup_blocked: the browser blocked the download page'); } }),
    { ok: false, reason: 'update.popup_blocked: the browser blocked the download page' },
  );
  // A bare string rejection is normalized to its text.
  assert.deepEqual(
    await openReleasePage({ invoke: async () => { throw 'unsupported-runtime: cad_open_release_page'; } }),
    { ok: false, reason: 'unsupported-runtime: cad_open_release_page' },
  );
});
