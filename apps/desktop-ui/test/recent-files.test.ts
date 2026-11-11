// Contract tests for recent drawings on the start screen.
//
// The wire is new (the host lane implements it), so these tests pin the chrome
// side: the payload is validated strictly (a malformed answer is one explicit
// `recent.invalid_payload`, never an empty list), the one carrier limitation maps
// to "unsupported" while every other rejection keeps its real reason, and an open
// sends exactly `{ opaque, displayName }`. The component assertions are source
// text, the house style for region contracts; there is no DOM mount here.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readModule } from './helpers.ts';
import {
  MAX_RECENT_ENTRIES,
  RECENT_INVALID_PAYLOAD,
  RECENT_LIST_COMMAND,
  RECENT_OPEN_COMMAND,
  loadRecentFiles,
  openRecentFile,
  parseRecentFiles,
} from '../src/files/recent.ts';

const entry = (overrides = {}) => ({
  displayName: 'plan.dwg',
  opaque: 'opaque-1',
  location: '/home/user/plan.dwg',
  openedAtMs: 1_700_000_000_000,
  ...overrides,
});

test('a well-formed answer is the host list, trimmed to the ten-row defence', () => {
  const answer = {
    supported: true,
    entries: [entry(), entry({ displayName: 'site.dxf', location: null })],
    reason: null,
  };
  assert.deepEqual(parseRecentFiles(answer), {
    kind: 'supported',
    entries: [
      entry(),
      entry({ displayName: 'site.dxf', location: null }),
    ],
  });
  // Supported + [] is the honest empty list, not an error.
  assert.deepEqual(parseRecentFiles({ supported: true, entries: [], reason: null }), {
    kind: 'supported',
    entries: [],
  });
  // Unsupported carries the host's stable reason for the technical record.
  assert.deepEqual(parseRecentFiles({ supported: false, entries: [], reason: 'web/no-fs' }), {
    kind: 'unsupported',
    reason: 'web/no-fs',
  });
  // The host caps at ten; the chrome still refuses an eleventh row from a future
  // host, and every row is validated before that cap.
  const many = Array.from({ length: MAX_RECENT_ENTRIES + 2 }, (_, index) => entry({ opaque: `opaque-${index}` }));
  assert.equal(parseRecentFiles({ supported: true, entries: many, reason: null }).entries.length, MAX_RECENT_ENTRIES);
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
    { supported: true, entries: [entry({ opaque: '' })], reason: null },
    { supported: true, entries: [entry({ displayName: '   ' })], reason: null },
    { supported: true, entries: [entry({ location: 7 })], reason: null },
    { supported: true, entries: [entry({ openedAtMs: Number.NaN })], reason: null },
    { supported: true, entries: [entry({ openedAtMs: 'now' })], reason: null },
    { supported: true, entries: [entry(), 5], reason: null },
    // A bad row past the cap must still be refused: the cap cannot hide a
    // malformed payload.
    {
      supported: true,
      entries: Array.from({ length: MAX_RECENT_ENTRIES }, (_, index) => entry({ opaque: `opaque-${index}` })).concat([{ displayName: 'bad' }]),
      reason: null,
    },
  ];
  for (const payload of malformed) {
    assert.throws(
      () => parseRecentFiles(payload),
      { message: RECENT_INVALID_PAYLOAD },
      JSON.stringify(payload),
    );
  }
});

test('a read maps only the named carrier limitation to unsupported', async () => {
  assert.deepEqual(
    await loadRecentFiles(async () => ({ supported: false, entries: [], reason: 'web/no-fs' })),
    { kind: 'unsupported', reason: 'web/no-fs' },
  );
  assert.deepEqual(
    await loadRecentFiles(async () => {
      throw 'unsupported-runtime: cad_recent_files';
    }),
    { kind: 'unsupported', reason: 'unsupported-runtime: cad_recent_files' },
  );
  // Every other rejection keeps the real reason.
  assert.deepEqual(
    await loadRecentFiles(async () => {
      throw 'recent.read_failed: permission denied';
    }),
    { kind: 'failed', reason: 'recent.read_failed: permission denied' },
  );
  // A resolved-but-malformed answer is an explicit invalid-payload failure.
  assert.deepEqual(
    await loadRecentFiles(async () => ({ supported: true, entries: 'nope', reason: null })),
    { kind: 'failed', reason: RECENT_INVALID_PAYLOAD },
  );
  // The read uses the one sanctioned command name.
  const calls = [];
  await loadRecentFiles(async (command) => {
    calls.push(command);
    return { supported: true, entries: [], reason: null };
  });
  assert.deepEqual(calls, [RECENT_LIST_COMMAND]);
});

test('opening a row sends exactly the host token and the display name', async () => {
  const calls = [];
  const result = await openRecentFile(
    // Extra fields must not leak into the wire payload.
    { ...entry(), location: '/secret/path', openedAtMs: 42 },
    async (command, args) => {
      calls.push({ command, args });
    },
  );
  assert.deepEqual(result, { kind: 'accepted' });
  assert.deepEqual(calls, [{
    command: RECENT_OPEN_COMMAND,
    args: { opaque: 'opaque-1', displayName: 'plan.dwg' },
  }]);

  const refused = await openRecentFile(entry(), async () => {
    throw 'recent.open_failed: file moved';
  });
  assert.deepEqual(refused, { kind: 'refused', reason: 'recent.open_failed: file moved' });
});

test('the start panel uses the hook and renders every state without pretending', async () => {
  const panel = await readModule('components/StartPanel.tsx');
  assert.ok(panel.includes("import { useRecentFiles } from '../files/recent.ts';"));
  assert.ok(panel.includes('const recent = useRecentFiles();'));
  assert.ok(panel.includes("import '../files/recent.css';"));
  // Four real states + the inline open refusal, all catalog copy.
  assert.ok(panel.includes("t('start.recent_empty')"));
  assert.ok(panel.includes("t('start.recent_unavailable')"));
  assert.ok(panel.includes("t('start.recent_failed'"));
  assert.ok(panel.includes("t('start.recent_open'"));
  assert.ok(panel.includes('aria-busy="true"'));
  assert.ok(panel.includes('role="alert"'));
  // A row is a real button, disabled by the shared admission + loading fact.
  assert.ok(panel.includes('className="recent-files__row"'));
  assert.ok(panel.includes('disabled={recentDisabled}'));
  assert.ok(panel.includes('recentDisabled ? openRefusal : openLabel'));
  assert.ok(panel.includes('onClick={() => openRecent(entry)}'));
  // No transport in the component: the hook owns the sanctioned seam.
  assert.equal(panel.includes('invoke('), false, 'StartPanel must not call invoke directly');
  assert.equal(panel.includes('cad_open_recent'), false, 'the command lives in files/recent.ts');

  const recent = await readModule('files/recent.ts');
  assert.ok(recent.includes("from '../host/invoke.ts'"));
  assert.ok(recent.includes("RECENT_LIST_COMMAND = 'cad_recent_files'"));
  assert.ok(recent.includes("RECENT_OPEN_COMMAND = 'cad_open_recent'"));
  assert.ok(recent.includes('unsupported-runtime: cad_recent_files'));
  assert.ok(recent.includes('RECENT_INVALID_PAYLOAD = \'recent.invalid_payload\''));
  assert.ok(recent.includes('MAX_RECENT_ENTRIES = 10'));

  const css = await readModule('files/recent.css');
  assert.ok(css.includes('min-height: 44px'), 'a row is a 44px target in every layout');
  assert.ok(css.includes('text-overflow: ellipsis'), 'long names and paths truncate');
  assert.ok(css.includes('.recent-files__row:focus-visible'));
  for (const token of ['var(--row-hover)', 'var(--pressed)', 'var(--field)', 'var(--line-soft)', 'var(--focus)', 'var(--danger)']) {
    assert.ok(css.includes(token), `the list uses ${token}`);
  }
  assert.equal(/position:\s*fixed/.test(css), false, 'the list stays inside the start panel');
});
