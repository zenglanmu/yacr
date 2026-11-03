// Contract tests for the external-references surface: the `RESOURCES`
// selector, the `cad_status.references` projection and the strict payload
// validation. No screenshot and no invented host action.
import test from 'node:test';
import assert from 'node:assert/strict';
import { parseCadStatus } from '../src/cad/cad-status.ts';
import {
  isResourcesCommand, referenceHintKey, referenceReasonCode, referenceRows, referenceTone,
  REFERENCE_STATUS_KEYS, RESOURCES_COMMAND,
} from '../src/references/references.ts';
import { readCatalog, readModule } from './helpers.ts';

const base = {
  documentLabel: null, entityCount: null, loading: false, importProgress: null,
  error: null, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false,
  snapSpacing: 10, orthoEnabled: false, canSave: false, canPlot: false, canFit: false,
  backend: null, adapter: null, camera: null, importCount: 0, frameCount: 0,
  cursorWorld: null,
  selection: { count: 0, empty: true, rows: [], mixedKeys: [] },
};

const reference = (overrides = {}) => ({
  name: 'plan-base',
  path: 'C:\\projects\\base.dwg',
  resolvedPath: 'C:\\projects\\base.dwg',
  status: 'loaded',
  reason: null,
  overlay: false,
  depth: 0,
  ...overrides,
});

const status = overrides => ({ ...base, ...overrides });

test('an older host with no references block renders the empty state', () => {
  assert.deepEqual(referenceRows(status()), []);
  assert.deepEqual(referenceRows(null), []);
  const older = status();
  assert.equal(parseCadStatus(older), older);
});

test('a loaded reference exposes the real resolved path and no warning hint', () => {
  const rows = referenceRows(status({ references: [reference()] }));
  assert.equal(rows.length, 1);
  assert.equal(rows[0].name, 'plan-base');
  assert.equal(rows[0].path, 'C:\\projects\\base.dwg');
  assert.equal(rows[0].resolvedPath, 'C:\\projects\\base.dwg');
  assert.equal(rows[0].status, 'loaded');
  assert.equal(rows[0].statusKey, 'references.status.loaded');
  assert.equal(rows[0].tone, 'ok');
  assert.equal(rows[0].hintKey, null);
  assert.equal(rows[0].overlay, false);
  assert.equal(rows[0].depth, 0);
});

test('a missing reference keeps the saved path and says it was not found', () => {
  const rows = referenceRows(status({
    references: [reference({
      status: 'missing', resolvedPath: null, reason: 'reference.missing: file not found', depth: 2,
    })],
  }));
  assert.equal(rows[0].path, 'C:\\projects\\base.dwg');
  assert.equal(rows[0].resolvedPath, null);
  assert.equal(rows[0].statusKey, 'references.status.missing');
  assert.equal(rows[0].tone, 'error');
  assert.equal(rows[0].hintKey, 'references.hint.missing');
  assert.equal(rows[0].reasonCode, 'reference.missing', 'only the stable code is shown');
});

test('a browser-unavailable reference explains the carrier limit, not a fake retry', () => {
  const rows = referenceRows(status({
    references: [reference({ status: 'browser-unavailable', resolvedPath: null, overlay: true })],
  }));
  assert.equal(rows[0].statusKey, 'references.status.browser_unavailable');
  assert.equal(rows[0].tone, 'error');
  assert.equal(rows[0].hintKey, 'references.hint.browser_unavailable');
  assert.equal(rows[0].overlay, true);
});

test('every closed state has a catalog badge key and a tone', () => {
  const states = [
    'loaded', 'missing', 'browser-unavailable', 'refused-path', 'unloaded', 'cycle',
    'depth-exceeded', 'skipped-overlay', 'unsupported',
  ];
  assert.deepEqual(Object.keys(REFERENCE_STATUS_KEYS).sort(), [...states].sort());
  for (const state of states) assert.match(REFERENCE_STATUS_KEYS[state], /^references\.status\./);
  assert.equal(referenceTone('loaded'), 'ok');
  assert.equal(referenceTone('cycle'), 'error');
  assert.equal(referenceTone('unloaded'), 'muted');
  assert.equal(referenceTone('unsupported'), 'muted');
  assert.equal(referenceHintKey('missing'), 'references.hint.missing');
  assert.equal(referenceHintKey('browser-unavailable'), 'references.hint.browser_unavailable');
  assert.equal(referenceHintKey('loaded'), null);
});

test('RESOURCES is one local selector, case-insensitive, and never a forwarded command', () => {
  assert.equal(RESOURCES_COMMAND, 'RESOURCES');
  assert.equal(isResourcesCommand('resources'), true);
  assert.equal(isResourcesCommand('  Resources '), true);
  assert.equal(isResourcesCommand('RESOURCES 2'), false);
  assert.equal(isResourcesCommand(''), false);
});

test('malformed reference rows are rejected instead of coerced', () => {
  const bad = [
    { references: {} },
    { references: [{ ...reference(), name: 3 }] },
    { references: [{ ...reference(), path: null }] },
    { references: [{ ...reference(), resolvedPath: 7 }] },
    { references: [{ ...reference(), status: 'broken' }] },
    { references: [{ ...reference(), reason: 9 }] },
    { references: [{ ...reference(), overlay: 'true' }] },
    { references: [{ ...reference(), depth: -1 }] },
    { references: [{ ...reference(), depth: 1.5 }] },
  ];
  for (const block of bad) {
    assert.throws(() => parseCadStatus(status(block)), /CAD_STATUS_INVALID/, JSON.stringify(block));
  }
  // Null reason/resolved path are real wire values, not malformed ones.
  const good = status({ references: [reference({ status: 'unloaded', resolvedPath: null, reason: null })] });
  assert.equal(parseCadStatus(good), good);
});

test('the panel renders the shared rows and offers no unsupported action', async () => {
  const panel = await readModule('components/ReferencesPanel.tsx');
  assert.ok(panel.includes('const rows = referenceRows(shell.status);'));
  assert.ok(panel.includes("t('resources.references.none')"), 'the empty state reuses the existing key');
  assert.ok(panel.includes('t(row.statusKey)'));
  assert.ok(panel.includes('t(row.hintKey)'));
  assert.ok(panel.includes('references__reason'), 'the stable reason code is visible');
  assert.ok(panel.includes('references__badge'));
  assert.equal(panel.includes('RELOAD'), false, 'no invented reload action');
  assert.equal(panel.includes('LOCATE'), false, 'no invented locate action');

  const app = await readModule('App.tsx');
  const selectors = await readModule('commands/local-selectors.ts');
  assert.ok(selectors.includes('if (isResourcesCommand(text))'), 'RESOURCES is a chrome-local selector');
  assert.ok(app.includes('openResources: () => setReferencesOpen(true)'), 'it opens the panel');
  const drawer = await readModule('components/MobileRibbonDrawer.tsx');
  assert.ok(drawer.includes('shell.setReferencesOpen(true)'), 'the compact layout reaches the same panel');
});

test('the browser limitation copy is grounded and mentions the desktop app', async () => {
  const en = await readCatalog('en');
  const zh = await readCatalog('zh-CN');
  assert.match(en['references.hint.browser_unavailable'], /browser/i);
  assert.match(en['references.hint.browser_unavailable'], /desktop/i);
  assert.match(zh['references.hint.browser_unavailable'], /浏览器/);
  assert.match(zh['references.hint.browser_unavailable'], /桌面/);
  for (const key of Object.values(REFERENCE_STATUS_KEYS)) {
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
  }
});
