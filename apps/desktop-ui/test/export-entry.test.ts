// The EXPORT split button (ADR-0041 + ADR-0052).
//
// EXPORT is its own router selector: the host opens an export picker and the
// chosen extension decides the writer. The File ribbon keeps **one** item for it,
// a split button whose caret lists the typed `EXPORT <TOKEN>` forms — argument
// forms of the same selector, not commands of their own. Every row therefore
// dispatches its full command text through the one admission funnel, and the
// parent's `canSave` gate covers the whole list (admission runs on the base name).
import test from 'node:test';
import assert from 'node:assert/strict';
import { admitCommandText, commandById, setCommandCatalog } from '../src/commands/commands.ts';
import { ARGUMENT_COMMANDS, CAPABILITY_GATES } from '../src/commands/gating.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { readCatalog, readModule } from './helpers.ts';

setCommandCatalog(['OPEN', 'NEW', 'SAVE', 'PLOT', 'EXPORT', 'PRINT', 'CLOSE', 'FIT']);

const loaded = {
  documentLabel: 'fixture.ymodel', entityCount: 3, loading: false,
  canOpen: true, canNew: true, canSave: true, canPlot: true, canFit: true,
};

/** The File panel's EXPORT split button. */
function exportItem() {
  const file = RIBBON_TABS.find(tab => tab.id === 'file').panels.find(panel => panel.id === 'file');
  assert.ok(file, 'the File panel must exist');
  const item = file.items.find(entry => entry.id === 'EXPORT');
  assert.ok(item, 'the File panel must carry EXPORT');
  return { file, item };
}

/** `[id, labelKey, command]` per row, in menu order. */
const ROWS = [
  ['EXPORT', 'file.export'],
  ['EXPORT DXF', 'file.export.dxf'],
  ['EXPORT DWG', 'file.export.dwg'],
  ['EXPORT GLTF', 'file.export.gltf'],
  ['EXPORT GLB', 'file.export.glb'],
  ['EXPORT STL', 'file.export.stl'],
  ['EXPORT OBJ', 'file.export.obj'],
  ['EXPORT PLY', 'file.export.ply'],
  ['EXPORT OFF', 'file.export.off'],
  ['EXPORT IFC', 'file.export.ifc'],
  ['EXPORT SKP', 'file.export.skp'],
];

test('EXPORT is a File ribbon split button whose caret lists the format forms', async () => {
  const { file, item } = exportItem();
  assert.equal(item.command, 'EXPORT');
  assert.equal(item.labelKey, 'file.export');
  assert.equal(item.icon, 'export');
  assert.equal(item.large, true, 'EXPORT is a primary file action like SAVE');
  assert.equal('action' in item, false, 'EXPORT is a router command, not a chrome action');
  // It sits with the output actions, after SAVE and before PLOT.
  const ids = file.items.map(entry => entry.id);
  assert.ok(ids.indexOf('SAVE') < ids.indexOf('EXPORT'), 'EXPORT follows SAVE');
  assert.ok(ids.indexOf('EXPORT') < ids.indexOf('PLOT'), 'EXPORT precedes PLOT');
  // One panel item: the ten format rows live in its caret, never as extra cells.
  assert.equal(ribbonCommandIds().filter(id => id === 'EXPORT').length, 1);

  const variants = item.variants ?? [];
  assert.deepEqual(variants.map(row => [row.id, row.labelKey]), ROWS,
    'the caret lists the plain picker first, then one row per format, in this order');
  for (const row of variants) {
    assert.equal(row.command, row.id, `${row.id}: the row sends its own full command text`);
    assert.equal(row.icon, 'export', `${row.id}: every row reuses the export glyph`);
    assert.equal('large' in row, false, `${row.id}: a menu row carries no tile size`);
  }

  // The button dispatches the item's command text into the shell's one funnel;
  // admission (not the ribbon) decides whether it is live.
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
  assert.equal(admitCommandText('EXPORT', { status: loaded }).kind, 'admitted');
  assert.deepEqual(admitCommandText('export', { status: loaded }), {
    kind: 'admitted', command: 'EXPORT', reasonKey: null,
  });
});

test('every format row is admitted by the EXPORT argument grammar', () => {
  const { item } = exportItem();
  const variants = item.variants ?? [];
  assert.equal(variants.length, ROWS.length);
  for (const row of variants) {
    const space = row.command.indexOf(' ');
    if (space === -1) {
      assert.equal(row.command, 'EXPORT', 'only the parent is the bare selector');
      continue;
    }
    const token = row.command.slice(space + 1);
    assert.equal(ARGUMENT_COMMANDS.EXPORT.parse(token), true, `the grammar refuses ${token}`);
    assert.equal(ARGUMENT_COMMANDS.EXPORT.parse(token.toLowerCase()), true,
      `${token}: the token is case-insensitive`);
    assert.deepEqual(admitCommandText(row.command.toLowerCase(), { status: loaded }), {
      kind: 'admitted', command: row.command, reasonKey: null,
    }, `${row.command}: admission normalises the line and keeps it whole`);
  }
  // The grammar is a closed token list: an unknown format never reaches IPC.
  assert.equal(ARGUMENT_COMMANDS.EXPORT.parse('ZIP'), false);
  assert.deepEqual(admitCommandText('EXPORT ZIP', { status: loaded }), {
    kind: 'unsupported', command: 'EXPORT ZIP', reasonKey: 'command.unsupported',
  });
  // COFF is grammar-only: the host accepts the token, the File panel has no row.
  assert.equal(ARGUMENT_COMMANDS.EXPORT.parse('COFF'), true);
  assert.equal(variants.some(row => row.command === 'EXPORT COFF'), false);
});

test('the format rows inherit the parent\'s canSave gate', () => {
  for (const row of ['EXPORT', 'EXPORT DXF', 'EXPORT GLB', 'EXPORT SKP']) {
    assert.deepEqual(
      admitCommandText(row, { status: { ...loaded, canSave: false } }),
      { kind: 'unavailable', command: row, reasonKey: 'command.unsupported' },
      `${row}: the save capability gates the whole family`,
    );
    assert.deepEqual(
      admitCommandText(row, {
        status: { ...loaded, documentLabel: null, entityCount: null, canSave: false },
      }),
      { kind: 'unavailable', command: row, reasonKey: 'plot.no_document' },
      `${row}: no open document is the whole reason on the start page`,
    );
  }
  assert.equal(CAPABILITY_GATES.EXPORT, 'canSave');
});

test('the command catalog lists EXPORT under its own label, gated by canSave', async () => {
  // `EXPORT <TOKEN>` is an argument form, not a published name: it gets no
  // catalog row (`ZIP` proves the palette is not a second name list).
  assert.deepEqual({ ...commandById('EXPORT') }, {
    id: 'EXPORT', labelKey: 'file.export', icon: 'export', group: 'file',
  });
  assert.equal(commandById('EXPORT DXF'), null);
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

test('every format row has a bilingual label that matches its menu order', async () => {
  const { item } = exportItem();
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const variant of item.variants ?? []) {
    assert.equal(typeof zh[variant.labelKey], 'string', `zh-CN missing ${variant.labelKey}`);
    assert.equal(typeof en[variant.labelKey], 'string', `en missing ${variant.labelKey}`);
    // The plain picker keeps its existing wording; every format row names the
    // format, so a reader never has to open the picker to learn what it writes.
    if (variant.id === 'EXPORT') continue;
    const token = variant.id.slice('EXPORT '.length);
    assert.ok(en[variant.labelKey].toUpperCase().includes(token),
      `${variant.labelKey} must name ${token}`);
    assert.ok(zh[variant.labelKey].toLowerCase().includes(token.toLowerCase()),
      `${variant.labelKey} must name ${token}`);
  }
});
