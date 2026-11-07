// The 2026-11-07 view batch's chrome half: catalog rows, typed-argument
// grammars, document gating and the View tab's view-state panel.
//
// The Rust lane is the authority for behaviour; this suite pins only the
// user-facing surface so a missing label, a locally admitted out-of-grammar line
// or a button that would have to invent an argument fails here.
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { ARGUMENT_COMMANDS, DOCUMENT_SCOPED_COMMANDS } from '../src/commands/gating.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

const BATCH = [
  'REDRAW', 'REDRAWALL', 'REGEN', 'REGENALL',
  'VIEW', 'VIEWGO', 'VIEWBACK', 'VIEWFORWARD', 'VPOINT',
];

test('every batch command is catalogued exactly once with a real label key', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const id of BATCH) {
    const rows = COMMAND_CATALOG.filter(command => command.id === id);
    assert.equal(rows.length, 1, `${id} must appear exactly once`);
    assert.equal(rows[0].group, 'view', `${id} belongs to the view group`);
    assert.equal(typeof zh[rows[0].labelKey], 'string', `zh-CN missing ${rows[0].labelKey}`);
    assert.equal(typeof en[rows[0].labelKey], 'string', `en missing ${rows[0].labelKey}`);
    assert.equal(HOST_NAMES.includes(id), true, `${id} must be in the vocabulary mirror`);
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id), true, `${id} needs an open drawing`);
  }
});

test('the typed grammars mirror the Rust parser', () => {
  const view = ARGUMENT_COMMANDS.VIEW;
  assert.ok(view, 'VIEW must declare its grammar');
  for (const admitted of ['SAVE Plan A', 'RESTORE plan a', 'LIST', 'DELETE Old']) {
    assert.equal(view.parse(admitted), true, admitted);
  }
  for (const refused of ['', 'SAVE', 'RESTORE', 'DELETE', 'LIST extra', 'BOGUS']) {
    assert.equal(view.parse(refused), false, refused);
  }

  const viewgo = ARGUMENT_COMMANDS.VIEWGO;
  assert.ok(viewgo, 'VIEWGO must declare its grammar');
  assert.equal(viewgo.parse('Plan A'), true);
  assert.equal(viewgo.parse('   '), false);

  const vpoint = ARGUMENT_COMMANDS.VPOINT;
  assert.ok(vpoint, 'VPOINT must declare its grammar');
  for (const admitted of ['TOP', 'bottom', '1 -2.5 3']) {
    assert.equal(vpoint.parse(admitted), true, admitted);
  }
  for (const refused of ['', 'ISOMETRIC', '1 2', '1 2 3 4', 'x 0 0', '1e3 0 0']) {
    assert.equal(vpoint.parse(refused), false, refused);
  }
});

test('the View tab exposes only payload-free refresh and stack buttons', () => {
  const tab = RIBBON_TABS.find(entry => entry.id === 'view');
  assert.ok(tab, 'the View tab must exist');
  const panel = tab.panels.find(entry => entry.id === 'viewstate');
  assert.ok(panel, 'the View tab must carry the view-state panel');
  assert.deepEqual(
    panel.items.map(item => item.id),
    ['REDRAW', 'REDRAWALL', 'REGEN', 'REGENALL', 'VIEWBACK', 'VIEWFORWARD'],
  );
  for (const item of panel.items) {
    assert.equal(item.action, undefined, `${item.id} reaches the router`);
    assert.equal(item.command, item.id, `${item.id} dispatches its own selector`);
    assert.equal(typeof item.hintKey, 'string', `${item.id} states its limit`);
  }
  // The argument-taking selectors intentionally have no button: a click cannot
  // invent a view name or a direction.
  const ids = tab.panels.flatMap(entry => entry.items.map(item => item.id));
  for (const typed of ['VIEW', 'VIEWGO', 'VPOINT']) {
    assert.equal(ids.includes(typed), false, `${typed} has no button`);
  }
});
