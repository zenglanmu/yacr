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
import {
  PRESELECT_PROMPTS,
  TOOL_READY_PROMPTS,
  TOOL_STEP_PROMPTS,
  toolStepPromptKey,
} from '../src/cad/cad.ts';
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

// --- OVERKILL: exact duplicates only -----------------------------------------

const OVERKILL = Object.freeze({
  id: 'OVERKILL',
  labelKey: 'draw.kind.overkill',
  hintKey: 'draw.kind.overkill.hint',
  icon: 'erase',
});

test('OVERKILL is catalogued as a selection-based modify selector', () => {
  assertCatalogued({ ...OVERKILL, group: 'modify' });
  // Canonical, never an alias of ERASE: ERASE deletes the whole selection, while
  // OVERKILL only removes the duplicates it can prove.
  assert.notEqual(OVERKILL.id, 'ERASE');
});

test('both Modify panels carry the small OVERKILL router button', () => {
  assertRibbonButton(OVERKILL);
});

test('OVERKILL needs at least two objects to compare', () => {
  assertSelectionRule({ ...OVERKILL, count: 2 });
});

// --- BREAKATPOINT: interactive two-step split --------------------------------

const BREAKATPOINT = Object.freeze({
  id: 'BREAKATPOINT',
  labelKey: 'draw.kind.breakatpoint',
  hintKey: 'draw.kind.breakatpoint.hint',
  icon: 'trim',
  toolKey: 'draw.breakatpoint',
  selectEntity: 'tool.prompt.breakatpoint.select_entity',
  point: 'tool.prompt.breakatpoint.point',
  ready: 'tool.prompt.breakatpoint.ready',
});

test('BREAKATPOINT is catalogued and admitted as its own selector', () => {
  assertCatalogued({ ...BREAKATPOINT, group: 'modify' });
  // Never an alias of BREAK: BREAK's first click is also the first break point,
  // BREAKATPOINT's first click is only the object pick.
  assert.notEqual(BREAKATPOINT.id, 'BREAK');
  assert.ok(en[`tool.draw.breakatpoint`], 'the tool chip label must exist in en');
  assert.ok(zh[`tool.draw.breakatpoint`], 'the tool chip label must exist in zh-CN');
});

test('both Modify panels carry the BREAKATPOINT tool button', () => {
  assertRibbonButton(BREAKATPOINT);
  // It is a capture, so it is not a pre-selection tool and has no parameter row.
  assert.equal(PRESELECT_PROMPTS[BREAKATPOINT.toolKey], undefined);
});

test('BREAKATPOINT has no selection precondition because it is a tool', () => {
  assertSelectionRule({ ...BREAKATPOINT, count: 0 });
});

test('BREAKATPOINT names its two steps: object pick, then break point', () => {
  assert.deepEqual(TOOL_STEP_PROMPTS[BREAKATPOINT.toolKey], [
    BREAKATPOINT.selectEntity,
    BREAKATPOINT.point,
  ]);
  assert.equal(TOOL_STEP_PROMPTS[BREAKATPOINT.toolKey].length, 2);
  assert.notEqual(BREAKATPOINT.selectEntity, BREAKATPOINT.point);
  assert.equal(TOOL_READY_PROMPTS[BREAKATPOINT.toolKey], BREAKATPOINT.ready);
  const tool = (points, ready = false) => ({ key: BREAKATPOINT.toolKey, points, ready });
  assert.equal(toolStepPromptKey(tool(0)), BREAKATPOINT.selectEntity);
  assert.equal(toolStepPromptKey(tool(1)), BREAKATPOINT.point);
  assert.equal(toolStepPromptKey(tool(2)), BREAKATPOINT.ready);
  assert.equal(toolStepPromptKey(tool(2, true)), BREAKATPOINT.ready);
  for (const key of [BREAKATPOINT.selectEntity, BREAKATPOINT.point, BREAKATPOINT.ready]) {
    assert.ok(en[key], `${key} must exist in en`);
    assert.ok(zh[key], `${key} must exist in zh-CN`);
  }
});
