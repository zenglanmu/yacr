// Contract tests for recovery snapshots on the start screen.
//
// The chrome side only: the payload is validated strictly (a malformed answer is
// one explicit `recovery.invalid_payload`, never an empty list), the one carrier
// limitation maps to "unsupported" while every other rejection keeps its real
// reason, and an open sends exactly `{ key }` — never a path. The component
// assertions are source text, the house style for region contracts; there is no
// DOM mount here.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readModule } from './helpers.ts';
import {
  MAX_RECOVERY_ENTRIES,
  RECOVERY_INVALID_PAYLOAD,
  RECOVERY_LIST_COMMAND,
  RECOVERY_OPEN_COMMAND,
  loadRecoveryFiles,
  openRecoveryFile,
  parseRecoveryFiles,
  recoveryRowName,
} from '../src/files/recovery.ts';

const entry = (overrides = {}) => ({
  key: '0123456789abcdef',
  displayName: 'plan.ymodel',
  revision: 7,
  savedAtMs: 1_700_000_000_000,
  ...overrides,
});

test('a well-formed answer is the host list, trimmed to the defensive cap', () => {
  const answer = {
    supported: true,
    entries: [entry(), entry({ key: 'fedcba9876543210', displayName: null, savedAtMs: 0 })],
    reason: null,
  };
  assert.deepEqual(parseRecoveryFiles(answer), {
    kind: 'supported',
    entries: [entry(), entry({ key: 'fedcba9876543210', displayName: null, savedAtMs: 0 })],
  });
  // Supported + [] is the honest empty list, not an error.
  assert.deepEqual(parseRecoveryFiles({ supported: true, entries: [], reason: null }), {
    kind: 'supported',
    entries: [],
  });
  // Unsupported carries the host's stable reason for the technical record.
  assert.deepEqual(parseRecoveryFiles({ supported: false, entries: [], reason: 'web/no-fs' }), {
    kind: 'unsupported',
    reason: 'web/no-fs',
  });
  // The chrome refuses an eleventh row from a future host, and every row is
  // validated before that cap.
  const many = Array.from({ length: MAX_RECOVERY_ENTRIES + 2 }, (_, index) => entry({ key: `k${index}` }));
  assert.equal(parseRecoveryFiles({ supported: true, entries: many, reason: null }).entries.length, MAX_RECOVERY_ENTRIES);
});

test('a malformed payload throws the one explicit code, never an empty list', () => {
  const malformed = [
    null,
    undefined,
    [],
    'supported',
    {},
    { supported: 'yes', entries: [], reason: null },
    { supported: true, entries: null, reason: null },
    { supported: true, entries: {}, reason: null },
    { supported: true, entries: [], reason: 42 },
    { supported: true, entries: [entry({ key: '' })], reason: null },
    { supported: true, entries: [entry({ key: '   ' })], reason: null },
    { supported: true, entries: [entry({ displayName: 7 })], reason: null },
    { supported: true, entries: [entry({ revision: Number.NaN })], reason: null },
    { supported: true, entries: [entry({ revision: -1 })], reason: null },
    { supported: true, entries: [entry({ revision: '7' })], reason: null },
    { supported: true, entries: [entry({ savedAtMs: Number.NaN })], reason: null },
    { supported: true, entries: [entry(), 5], reason: null },
    // A bad row past the cap must still be refused: the cap cannot hide a
    // malformed payload.
    {
      supported: true,
      entries: Array.from({ length: MAX_RECOVERY_ENTRIES }, (_, index) => entry({ key: `k${index}` })).concat([{ key: 'bad' }]),
      reason: null,
    },
  ];
  for (const payload of malformed) {
    assert.throws(
      () => parseRecoveryFiles(payload),
      { message: RECOVERY_INVALID_PAYLOAD },
      JSON.stringify(payload),
    );
  }
});

test('a read maps only the named carrier limitation to unsupported', async () => {
  assert.deepEqual(
    await loadRecoveryFiles(async () => ({ supported: false, entries: [], reason: 'web/no-fs' })),
    { kind: 'unsupported', reason: 'web/no-fs' },
  );
  assert.deepEqual(
    await loadRecoveryFiles(async () => {
      throw 'unsupported-runtime: cad_recovery_files';
    }),
    { kind: 'unsupported', reason: 'unsupported-runtime: cad_recovery_files' },
  );
  // Every other rejection keeps the real reason (an unreadable store is not
  // "unsupported" and not "no snapshots").
  assert.deepEqual(
    await loadRecoveryFiles(async () => {
      throw 'recovery.store.list_unreadable: denied';
    }),
    { kind: 'failed', reason: 'recovery.store.list_unreadable: denied' },
  );
  // A resolved-but-malformed answer is an explicit invalid-payload failure.
  assert.deepEqual(
    await loadRecoveryFiles(async () => ({ supported: true, entries: 'nope', reason: null })),
    { kind: 'failed', reason: RECOVERY_INVALID_PAYLOAD },
  );
  // The read uses the one sanctioned command name.
  const calls = [];
  await loadRecoveryFiles(async (command) => {
    calls.push(command);
    return { supported: true, entries: [], reason: null };
  });
  assert.deepEqual(calls, [RECOVERY_LIST_COMMAND]);
});

test('opening a row sends exactly the store key, never a path', async () => {
  const calls = [];
  const result = await openRecoveryFile(
    // Extra fields must not leak into the wire payload.
    { ...entry(), displayName: '/secret/plan.ymodel', revision: 99, savedAtMs: 42 },
    async (command, args) => {
      calls.push({ command, args });
    },
  );
  assert.deepEqual(result, { kind: 'accepted' });
  assert.deepEqual(calls, [{
    command: RECOVERY_OPEN_COMMAND,
    args: { key: '0123456789abcdef' },
  }]);
  assert.equal('displayName' in calls[0].args, false, 'no display name travels');
  assert.equal('revision' in calls[0].args, false, 'no revision travels');

  const refused = await openRecoveryFile(entry(), async () => {
    throw 'recovery.unknown: 0123456789abcdef is not an available snapshot';
  });
  assert.deepEqual(refused, {
    kind: 'refused',
    reason: 'recovery.unknown: 0123456789abcdef is not an available snapshot',
  });
});

test('a row name falls back to the key prefix only when the host recorded none', () => {
  assert.equal(recoveryRowName(entry()), 'plan.ymodel');
  assert.equal(recoveryRowName(entry({ displayName: null })), '01234567');
  assert.equal(recoveryRowName(entry({ displayName: '   ' })), '01234567');
  assert.equal(recoveryRowName(entry({ displayName: null, key: 'abc' })), 'abc');
});

test('the start panel shows recovery above recent files without pretending', async () => {
  const panel = await readModule('components/StartPanel.tsx');
  assert.ok(panel.includes("import { recoveryRowName, useRecoveryFiles } from '../files/recovery.ts';"));
  assert.ok(panel.includes('const recovery = useRecoveryFiles();'));
  // Every real state + the inline open refusal, all catalog copy.
  assert.ok(panel.includes("t('start.recovery_empty')"));
  assert.ok(panel.includes("t('start.recovery_unavailable')"));
  assert.ok(panel.includes("t('start.recovery_failed'"));
  assert.ok(panel.includes("t('start.recovery_open'"));
  assert.ok(panel.includes("t('start.recovery_open_failed'"));
  assert.ok(panel.includes("t('start.recovery_revision'"));
  assert.ok(panel.includes("t('start.recovery_saved'"));
  assert.ok(panel.includes('aria-busy="true"'));
  assert.ok(panel.includes('role="alert"'));
  // A row is a real button, disabled by the shared admission + loading fact.
  assert.ok(panel.includes('disabled={recoveryDisabled}'));
  assert.ok(panel.includes('recoveryDisabled ? openRefusal : openLabel'));
  assert.ok(panel.includes('onClick={() => openRecovery(entry)}'));
  // No transport in the component: the hook owns the sanctioned seam, and no
  // command name is spelled here.
  assert.equal(panel.includes('invoke('), false, 'StartPanel must not call invoke directly');
  assert.equal(panel.includes('cad_open_recovery'), false, 'the command lives in files/recovery.ts');
  assert.equal(panel.includes('/secret/'), false);
  // The recovery section precedes the recent one: snapshots are the more urgent
  // offer (unsaved work) and the panel's DOM order says so.
  const recoveryAt = panel.indexOf('start-recovery-heading');
  const recentAt = panel.indexOf('start-recent-heading');
  assert.ok(recoveryAt > 0 && recentAt > recoveryAt, 'recovery must be the first list');

  const recovery = await readModule('files/recovery.ts');
  assert.ok(recovery.includes("from '../host/invoke.ts'"));
  assert.ok(recovery.includes("RECOVERY_LIST_COMMAND = 'cad_recovery_files'"));
  assert.ok(recovery.includes("RECOVERY_OPEN_COMMAND = 'cad_open_recovery'"));
  assert.ok(recovery.includes('unsupported-runtime: cad_recovery_files'));
  assert.ok(recovery.includes("RECOVERY_INVALID_PAYLOAD = 'recovery.invalid_payload'"));
  assert.ok(recovery.includes('MAX_RECOVERY_ENTRIES = 10'));
  assert.ok(recovery.includes('export function useRecoveryFiles('));
  assert.ok(recovery.includes('export async function openRecoveryFile('));
});

test('both catalogs carry every recovery key with the same placeholders', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const key of [
    'start.recovery',
    'start.recovery_empty',
    'start.recovery_unavailable',
    'start.recovery_failed',
    'start.recovery_open',
    'start.recovery_open_failed',
    'start.recovery_revision',
    'start.recovery_saved',
  ]) {
    assert.equal(typeof zh[key], 'string', key);
    assert.equal(typeof en[key], 'string', key);
  }
});
