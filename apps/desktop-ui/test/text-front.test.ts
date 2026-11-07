// TEXTTOFRONT (AutoCAD 2026) user-facing surface contract.
//
// TEXTTOFRONT is the **selection-based draw-order** command: it brings the
// selected text, leaders and dimensions in front of every other model-space
// object by rewriting their draw order. The current selection is the input (a
// declared divergence from AutoCAD's Select objects prompt), so it is an ordinary
// router button gated by one selected object — never a capture. Paper/block draw
// order and the all-objects form are NOT IMPLEMENTED and the hint says so.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  selectionRequirementMet,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { SELECTION_REQUIREMENTS } from '../src/commands/gating.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };
const withSelection = count => ({ ...loaded, selection: { count } });

const TEXTTOFRONT = {
  id: 'TEXTTOFRONT',
  labelKey: 'draw.kind.texttofront',
  icon: 'properties',
  hint: 'draw.kind.texttofront.hint',
};

test('TEXTTOFRONT is a Modify panel router button on both tabs, with no popover', () => {
  for (const tabId of ['home', 'draw']) {
    const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
    const panel = tab.panels.find(entry => entry.id === 'modify');
    const items = panel.items;
    const at = items.findIndex(item => item.id === TEXTTOFRONT.id);
    assert.ok(at !== -1, `${tabId}/modify must carry the TEXTTOFRONT button`);
    assert.equal(items.filter(item => item.id === TEXTTOFRONT.id).length, 1, `${tabId}: one button`);

    const item = items[at];
    assert.equal(item.command, TEXTTOFRONT.id);
    assert.equal(item.labelKey, TEXTTOFRONT.labelKey);
    assert.equal(item.icon, TEXTTOFRONT.icon);
    assert.equal(item.hintKey, TEXTTOFRONT.hint);
    assert.equal(item.large, undefined, `${tabId}: TEXTTOFRONT is not a primary edit tile`);
    assert.equal(item.variants, undefined, `${tabId}: there is no options popover`);
    assert.equal(item.action, undefined, 'TEXTTOFRONT must dispatch through the router');
  }
  assert.ok(new Set(ribbonCommandIds()).has(TEXTTOFRONT.id));
});

test('TEXTTOFRONT is catalogued, admitted by the router and selection-gated on one object', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === TEXTTOFRONT.id);
  assert.ok(entry, 'TEXTTOFRONT must be in the command catalog');
  assert.equal(entry.labelKey, TEXTTOFRONT.labelKey);
  assert.equal(entry.icon, TEXTTOFRONT.icon);
  assert.equal(entry.group, 'modify');

  assert.equal(isSupportedCommand('texttofront'), true);
  assert.equal(admitCommandText('TEXTTOFRONT', { status: withSelection(1) }).kind, 'admitted');
  assert.equal(admitCommandText('TEXTTOFRONT', { status: withSelection(0) }).kind, 'unavailable');
  assert.equal(
    admitCommandText('TEXTTOFRONT', { status: withSelection(0) }).reasonKey,
    TEXTTOFRONT.hint,
    'the disabled reason is the command hint',
  );
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('TEXTTOFRONT'), false);
  setCommandCatalog(HOST_NAMES);

  const requirement = SELECTION_REQUIREMENTS[TEXTTOFRONT.id];
  assert.deepEqual(requirement, { count: 1, reasonKey: TEXTTOFRONT.hint });
  assert.equal(selectionRequirementMet(TEXTTOFRONT.id, withSelection(0)), false);
  assert.equal(selectionRequirementMet(TEXTTOFRONT.id, withSelection(1)), true);

  for (const catalog of [zh, en]) {
    assert.equal(typeof catalog['draw.kind.texttofront'], 'string');
    assert.equal(typeof catalog[TEXTTOFRONT.hint], 'string');
  }
  assert.match(en[TEXTTOFRONT.hint], /model-space/);
  assert.match(en[TEXTTOFRONT.hint], /not implemented/i);
  assert.equal(loaded.documentLabel, 'fixture.dwg');
});
