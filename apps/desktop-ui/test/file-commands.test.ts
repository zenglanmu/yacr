// File-level router selectors — `NEW`, `SAVE`/`SAVEAS`/`QSAVE`,
// `PLOT`/`EXPORT`/`PRINT` — are admitted by the host vocabulary and gated by the
// `cad_status` capability booleans (`canNew`/`canSave`/`canPlot`), never by a
// "host-owned" exclusion list. This suite is split from `commands.test.mjs` (both
// would otherwise cross the 1000-line code-file cap).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  BUSY_KEY,
  CAPABILITY_GATES,
  setCommandCatalog,
  unavailableReasonKey,
} from '../src/commands/commands.ts';

// The router's vocabulary for these names: the canonical selectors plus their
// full-word aliases, as `admissible_names()` reports them.
const HOST_NAMES = ['OPEN', 'NEW', 'SAVE', 'SAVEAS', 'QSAVE', 'PLOT', 'EXPORT', 'PRINT', 'FIT', 'LINE'];
setCommandCatalog(HOST_NAMES);

// The frozen `cad_status` capability shape: the three new booleans always travel
// with `canOpen`/`canFit`.
const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };
const empty = { documentLabel: null, entityCount: null, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: false, canPlot: false, canFit: false };

test('file-level commands are gated by the host capability booleans, never by a host-owned exclusion', async () => {
  // NEW: a blank drawing needs no document, so it is admitted on the start page.
  assert.deepEqual(admitCommandText('NEW', { status: empty }), { kind: 'admitted', command: 'NEW', reasonKey: null });
  assert.deepEqual(admitCommandText('new', { status: loaded }), { kind: 'admitted', command: 'NEW', reasonKey: null });
  assert.equal(CAPABILITY_GATES.NEW, 'canNew');
  // A false `canNew` with nothing loading means the host cannot create one at
  // all — the permanent reason, not a document or busy one.
  assert.deepEqual(
    admitCommandText('NEW', { status: { ...empty, canNew: false } }),
    { kind: 'unavailable', command: 'NEW', reasonKey: 'command.unsupported' },
  );
  // A false `canNew` while the host is loading reads as the busy sentence, and
  // that key already exists in both catalogs (no new copy is invented).
  assert.deepEqual(
    admitCommandText('NEW', { status: { ...empty, canNew: false, loading: true } }),
    { kind: 'unavailable', command: 'NEW', reasonKey: BUSY_KEY },
  );
  assert.equal(BUSY_KEY, 'linux.busy');
  // An in-flight command is the same temporary state for every name.
  assert.equal(admitCommandText('SAVE', { status: loaded, pending: true }).reasonKey, BUSY_KEY);

  // SAVE/SAVEAS/QSAVE share `canSave`.
  const saveFamily = ['SAVE', 'SAVEAS', 'QSAVE'];
  for (const typed of saveFamily) {
    assert.equal(CAPABILITY_GATES[typed], 'canSave', typed);
    // Document open + capability true: admitted.
    assert.deepEqual(
      admitCommandText(typed, { status: loaded }),
      { kind: 'admitted', command: typed, reasonKey: null },
      typed,
    );
    // No document (and nothing loading): the honest sentence is "no drawing".
    assert.deepEqual(
      admitCommandText(typed, { status: empty }),
      { kind: 'unavailable', command: typed, reasonKey: 'plot.no_document' },
      typed,
    );
    // A document is open but the host still cannot save: a real limitation.
    assert.deepEqual(
      admitCommandText(typed, { status: { ...loaded, canSave: false } }),
      { kind: 'unavailable', command: typed, reasonKey: 'command.unsupported' },
      typed,
    );
  }
  // PLOT/EXPORT/PRINT share `canPlot`, with the same three outcomes.
  for (const typed of ['PLOT', 'EXPORT', 'PRINT']) {
    assert.equal(CAPABILITY_GATES[typed], 'canPlot', typed);
    assert.deepEqual(admitCommandText(typed, { status: loaded }), { kind: 'admitted', command: typed, reasonKey: null }, typed);
    assert.deepEqual(
      admitCommandText(typed, { status: empty }),
      { kind: 'unavailable', command: typed, reasonKey: 'plot.no_document' },
      typed,
    );
    assert.deepEqual(
      admitCommandText(typed, { status: { ...loaded, canPlot: false } }),
      { kind: 'unavailable', command: typed, reasonKey: 'command.unsupported' },
      typed,
    );
  }
  // The reason helper the chrome's buttons use agrees with the admission, so a
  // disabled button and a refused submission word the same fact identically.
  const cases = [
    ['NEW', { ...empty, canNew: false }, 'command.unsupported'],
    ['NEW', { ...empty, canNew: false, loading: true }, BUSY_KEY],
    ['SAVE', empty, 'plot.no_document'],
    ['SAVEAS', empty, 'plot.no_document'],
    ['QSAVE', { ...loaded, canSave: false }, 'command.unsupported'],
    ['PLOT', empty, 'plot.no_document'],
    ['EXPORT', { ...loaded, canPlot: false }, 'command.unsupported'],
    ['PRINT', empty, 'plot.no_document'],
    ['LINE', loaded, 'command.unsupported'],
  ];
  for (const [id, status, key] of cases) {
    assert.equal(unavailableReasonKey(id, { status }), key, id);
  }
  // The busy sentence is the same key the admission reports while loading.
  assert.equal(unavailableReasonKey('SAVE', { status: loaded, pending: true }), BUSY_KEY);
  // Without a status the capability cannot be judged, so the generic reason stays.
  assert.equal(unavailableReasonKey('SAVE'), 'command.unsupported');
  // Every reason key above exists in both catalogs (this lane never edits them).
  const { readCatalog } = await import('./helpers.ts');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const key of ['command.unsupported', 'plot.no_document', BUSY_KEY]) {
    assert.equal(typeof zh[key], 'string', key);
    assert.equal(typeof en[key], 'string', key);
  }
});

test('the QAT keeps NEW/OPEN/SAVE as real router actions gated by shared admission', async () => {
  const { readModule } = await import('./helpers.ts');
  const source = await readModule('components/TitleBar.tsx');
  // A tokenless QAT entry renders permanently disabled, so the three file
  // actions must carry a real `runCommand` action: availability is then decided
  // by `shell.canCommand` (the shared admission), never by a second local check.
  for (const id of ['NEW', 'OPEN', 'SAVE']) {
    assert.ok(
      source.includes(`action: () => shell.runCommand('${id}')`),
      `the QAT ${id} entry must dispatch through shell.runCommand`,
    );
  }
  // The refusal hint is the shared reason key, not a hardcoded sentence.
  assert.ok(source.includes('unavailableReasonKey(item.id, { status: shell.status, pending: shell.pending })'));
  assert.ok(source.includes('disabled={!enabled}'));
});

test('the start panel offers New through the same admission, not a permanent disable', async () => {
  const { readModule } = await import('./helpers.ts');
  const source = await readModule('components/StartPanel.tsx');
  assert.ok(source.includes("onClick={() => shell.runCommand('NEW')}"));
  assert.ok(source.includes('disabled={!canNew}'));
  assert.ok(source.includes("const canNew = shell.canCommand('NEW');"));
  // The old permanent-disable copy stays in the catalogs (another lane owns
  // them) but is no longer what the button says: its title is the shared reason.
  assert.equal(source.includes('start.new_unavailable'), false);
  assert.ok(source.includes("refusal('NEW', 'qat.new')"));
});

test('the File tab keeps NEW/SAVE as command tokens enabled by admission', async () => {
  const { RIBBON_TABS } = await import('../src/ribbon/ribbon-model.ts');
  const { readModule } = await import('./helpers.ts');
  const file = RIBBON_TABS.find(tab => tab.id === 'file').panels.find(panel => panel.id === 'file');
  const byId = Object.fromEntries(file.items.map(item => [item.id, item]));
  for (const id of ['NEW', 'SAVE']) {
    assert.equal(byId[id].command, id, `${id} must stay a router command token`);
    // No local `disabled`/`enabled` override: the renderer's `canCommand` gate is
    // the only switch.
    assert.equal('disabled' in byId[id], false, id);
    assert.equal('action' in byId[id], false, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
  }
  // The ribbon renders a refused command through the shared reason key + state.
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes(': item.command ? shell.canCommand(item.command)'));
});
