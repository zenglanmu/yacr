// The EXPORT entry (ADR-0041).
//
// EXPORT is its own router selector now: the host opens an export picker and the
// chosen extension decides the writer. The chrome offers one entry — no format
// submenu — in the File ribbon panel and in the command catalog, and it
// dispatches the plain command text `EXPORT` through the same admission gate.
import test from 'node:test';
import assert from 'node:assert/strict';
import { admitCommandText, commandById, setCommandCatalog } from '../src/commands/commands.ts';
import { CAPABILITY_GATES } from '../src/commands/gating.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { readCatalog, readModule } from './helpers.ts';

setCommandCatalog(['OPEN', 'NEW', 'SAVE', 'PLOT', 'EXPORT', 'PRINT', 'CLOSE', 'FIT']);

const loaded = {
  documentLabel: 'fixture.ymodel', entityCount: 3, loading: false,
  canOpen: true, canNew: true, canSave: true, canPlot: true, canFit: true,
};

test('EXPORT is a File ribbon entry that dispatches its own command text', async () => {
  const file = RIBBON_TABS.find(tab => tab.id === 'file').panels.find(panel => panel.id === 'file');
  assert.ok(file, 'the File panel must exist');
  const exportItem = file.items.find(item => item.id === 'EXPORT');
  assert.ok(exportItem, 'the File panel must carry EXPORT');
  assert.equal(exportItem.command, 'EXPORT');
  assert.equal(exportItem.labelKey, 'file.export');
  assert.equal(exportItem.icon, 'export');
  assert.equal(exportItem.large, true, 'EXPORT is a primary file action like SAVE');
  // One button, no second-level format list: the host's picker decides.
  assert.equal('variants' in exportItem, false, 'the format choice belongs to the host picker');
  assert.equal('action' in exportItem, false, 'EXPORT is a router command, not a chrome action');
  // It sits with the output actions, after SAVE and before PLOT.
  const ids = file.items.map(item => item.id);
  assert.ok(ids.indexOf('SAVE') < ids.indexOf('EXPORT'), 'EXPORT follows SAVE');
  assert.ok(ids.indexOf('EXPORT') < ids.indexOf('PLOT'), 'EXPORT precedes PLOT');
  assert.equal(ribbonCommandIds().filter(id => id === 'EXPORT').length, 1);

  // The button dispatches the item's command text into the shell's one funnel;
  // admission (not the ribbon) decides whether it is live.
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
  assert.equal(admitCommandText('EXPORT', { status: loaded }).kind, 'admitted');
  assert.deepEqual(admitCommandText('export', { status: loaded }), {
    kind: 'admitted', command: 'EXPORT', reasonKey: null,
  });
});

test('the command catalog lists EXPORT under its own label, gated by canSave', async () => {
  assert.deepEqual({ ...commandById('EXPORT') }, {
    id: 'EXPORT', labelKey: 'file.export', icon: 'export', group: 'file',
  });
  // The command palette reads this table, so the label must exist in both
  // catalogs or the row would print a brace.
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const catalog of [zh, en]) {
    assert.equal(typeof catalog['file.export'], 'string');
    assert.ok(catalog['file.export'].trim().length > 0);
  }
  // The host checks the save capability before it opens the export picker, so
  // EXPORT carries SAVE's gate — not PLOT's, which stays on the PRINT alias.
  assert.equal(CAPABILITY_GATES.EXPORT, 'canSave');
  assert.equal(CAPABILITY_GATES.PLOT, 'canPlot');
  assert.equal(CAPABILITY_GATES.PRINT, 'canPlot');
  assert.deepEqual(
    admitCommandText('EXPORT', { status: { ...loaded, canSave: false } }),
    { kind: 'unavailable', command: 'EXPORT', reasonKey: 'command.unsupported' },
  );
  assert.deepEqual(
    admitCommandText('EXPORT', { status: { ...loaded, documentLabel: null, entityCount: null, canSave: false } }),
    { kind: 'unavailable', command: 'EXPORT', reasonKey: 'plot.no_document' },
  );
});
