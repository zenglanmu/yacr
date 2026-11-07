// The AutoCAD 2026 2D modify commands added 2026-11-07: REVERSE (selection
// based, direction flip). Sibling commands are appended to this suite as they
// land (OOPS/OVERKILL/BREAKATPOINT).
//
// What this pins: the catalogue row, both Modify-panel buttons, the selection
// requirement and its hint key, admission from the loaded host vocabulary, and
// both i18n keys. The backend semantics live in `crates/cad-app`.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  commandById,
  isSupportedCommand,
  normalizeCommandText,
  selectionRequirement,
  selectionRequirementMet,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

setCommandCatalog(HOST_NAMES);

const DOC = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  canOpen: true,
  canNew: true,
  canSave: true,
  canPlot: true,
  canFit: true,
};
const selected = count => ({ ...DOC, selection: { count } });

function modifyPanel(tabId) {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `the ${tabId} tab must exist`);
  const panel = tab.panels.find(candidate => candidate.id === 'modify');
  assert.ok(panel, `the ${tabId} tab must keep its modify panel`);
  return panel;
}

function assertCatalogued({ id, labelKey, icon, group }) {
  const entry = COMMAND_CATALOG.find(command => command.id === id);
  assert.ok(entry, `${id} must be in the command catalog`);
  assert.deepEqual({ ...entry }, { id, labelKey, icon, group });
  assert.equal(normalizeCommandText(`  ${id.toLowerCase()} `), id);
  assert.equal(commandById(id.toLowerCase()).id, id);
  assert.equal(isSupportedCommand(id.toLowerCase()), true);
  assert.ok(en[labelKey], `${labelKey} must exist in en`);
  assert.ok(zh[labelKey], `${labelKey} must exist in zh-CN`);
}

function assertRibbonButton({ id, labelKey, icon }) {
  for (const tabId of ['home', 'draw']) {
    const item = modifyPanel(tabId).items.find(candidate => candidate.id === id);
    assert.ok(item, `${tabId}/modify must carry ${id}`);
    assert.equal(item.command, id);
    assert.equal(item.labelKey, labelKey);
    assert.equal(item.icon, icon);
    assert.equal(item.variants, undefined, `${id} has no options popover`);
  }
}

function assertSelectionRule({ id, count, hintKey }) {
  assert.deepEqual({ ...selectionRequirement(id) }, { count, reasonKey: hintKey });
  assert.ok(en[hintKey], `${hintKey} must exist in en`);
  assert.ok(zh[hintKey], `${hintKey} must exist in zh-CN`);
  assert.equal(selectionRequirementMet(id, DOC), count === 0, `${id} with no selection`);
  if (count > 0) {
    assert.equal(selectionRequirementMet(id, selected(count)), true, `${id} at its count`);
    if (count > 1) {
      assert.equal(selectionRequirementMet(id, selected(count - 1)), false, `${id} below its count`);
    }
  }
}

// --- REVERSE: selection-based direction flip ---------------------------------

const REVERSE = Object.freeze({
  id: 'REVERSE',
  labelKey: 'draw.kind.reverse',
  hintKey: 'draw.kind.reverse.hint',
  icon: 'mirror',
});

test('REVERSE is catalogued and routed as its own modify selector', () => {
  assertCatalogued({ ...REVERSE, group: 'modify' });
  // A canonical selector of its own: never an alias of MIRROR or ROTATE, which
  // transform geometry rather than reverse its stored vertex order.
  assert.notEqual(REVERSE.id, 'MIRROR');
  assert.notEqual(REVERSE.id, 'ROTATE');
});

test('both Modify panels carry the small REVERSE router button', () => {
  assertRibbonButton(REVERSE);
});

test('REVERSE requires at least one selected object', () => {
  assertSelectionRule({ ...REVERSE, count: 1 });
});

// --- OOPS: restore the last ERASE --------------------------------------------

const OOPS = Object.freeze({
  id: 'OOPS',
  labelKey: 'draw.kind.oops',
  hintKey: 'draw.kind.oops.hint',
  icon: 'undo',
});

test('OOPS is catalogued as a selection-free modify selector', () => {
  assertCatalogued({ ...OOPS, group: 'modify' });
  // Canonical, like REVERSE: never an alias of UNDO (the history command) or
  // U (its AutoCAD abbreviation) — OOPS restores one erase, not a history step.
  assert.notEqual(OOPS.id, 'UNDO');
  assert.notEqual(OOPS.id, 'U');
});

test('both Modify panels carry the small OOPS router button', () => {
  assertRibbonButton(OOPS);
});

test('OOPS has no selection precondition', () => {
  assertSelectionRule({ ...OOPS, count: 0 });
  assert.equal(selectionRequirementMet(OOPS.id, { ...DOC, documentLabel: null }), false,
    'OOPS still needs an open drawing');
});

