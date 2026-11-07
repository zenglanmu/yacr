// SELECTSIMILAR (AutoCAD 2026): the type-based selection grower.
//
// This suite pins the UI half only — catalogue row, both Modify panels, the
// shortcut-menu entry, the selection requirement and both-locale copy. It does
// not claim the Rust command's runtime behaviour: `HOST_NAMES` is the local
// mirror of the router vocabulary, and the real enable state is the host's
// `canCommand`.
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { buildContextMenu, menuItems } from '../src/commands/context-menu.ts';
import { SELECTION_REQUIREMENTS } from '../src/commands/gating.ts';
import {
  admitCommandText,
  setCommandCatalog,
  unavailableReasonKey,
} from '../src/commands/commands.ts';
import { readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const ID = 'SELECTSIMILAR';
const LABEL_KEY = 'draw.kind.selectsimilar';
const HINT_KEY = 'draw.kind.selectsimilar.hint';
const MENU_KEY = 'menu.selection.similar';

setCommandCatalog([ID]);

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
const NO_DOCUMENT = { ...DOC, documentLabel: null, entityCount: null };
const selected = count => ({ ...DOC, selection: { count } });

const READY_MENU = {
  selection: { count: 1 },
  tool: null,
  lastCommand: 'LINE',
  canFit: true,
  hasDocument: true,
};

function modifyPanel(tabId) {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `${tabId} tab must exist`);
  const panel = tab.panels.find(candidate => candidate.id === 'modify');
  assert.ok(panel, `${tabId} must keep its modify panel`);
  return panel;
}

function menuItem(input) {
  return menuItems(buildContextMenu(input)).find(item => item.command === ID);
}

test('SELECTSIMILAR is a modify-group catalogue row appended after the shipped rows', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === ID);
  assert.ok(entry, 'SELECTSIMILAR must be in the command catalog');
  assert.deepEqual({ ...entry }, {
    id: ID,
    labelKey: LABEL_KEY,
    icon: 'selectAll',
    group: 'modify',
  });
  assert.equal(COMMAND_CATALOG.filter(command => command.id === ID).length, 1);
  const order = COMMAND_CATALOG.map(command => command.id);
  const at = order.indexOf(ID);
  // Append-only: the row sits after the last shipped pre-2026-11-07 row
  // (`PDSIZE`) and after JOIN; later rows (REVCLOUD/AUDIT) may follow it.
  assert.ok(at > order.indexOf('PDSIZE'), 'appended after shipped rows');
  assert.ok(at > order.indexOf('JOIN'));
  assert.ok(at < order.indexOf('REVCLOUD'));
  assert.ok(at < order.indexOf('AUDIT'));
});

test('both modify panels carry SELECTSIMILAR as one small selection-gated button', () => {
  for (const tabId of ['home', 'draw']) {
    const items = modifyPanel(tabId).items;
    const item = items.find(candidate => candidate.id === ID);
    assert.ok(item, `${tabId}: SELECTSIMILAR is present`);
    // The 2026-11-07 modify commands are appended after the shipped tail.
    assert.deepEqual(
      items.slice(items.findIndex(candidate => candidate.id === ID) + 1).map(candidate => candidate.id),
      ['REVERSE', 'PEDIT', 'OOPS', 'OVERKILL', 'BREAKATPOINT', 'HATCHTOBACK', 'TEXTTOFRONT',
        'SSX', 'SELECTCOUNT', 'HIDEOBJECTS', 'ISOLATEOBJECTS', 'UNISOLATEOBJECTS',
        'TREX', 'EXTRIM', 'MPEDIT', 'DRAWORDER'],
      `${tabId}: only the documented additions follow SELECTSIMILAR`,
    );
    assert.equal(item.command, ID, 'the dispatch token is the router name');
    assert.equal(item.labelKey, LABEL_KEY);
    assert.equal(item.icon, 'selectAll');
    assert.equal(item.large, undefined, `${tabId}: SELECTSIMILAR is a small button`);
    assert.equal(item.variants, undefined, `${tabId}: type-only v1 has no popover`);
    assert.equal(item.action, undefined, 'SELECTSIMILAR must dispatch through the router');
    assert.equal(item.standardView, undefined);
    assert.equal(items.filter(candidate => candidate.id === ID).length, 1, `${tabId}: one button`);
  }
  // No other panel smuggles the command in.
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      if (panel.id === 'modify') continue;
      assert.equal(panel.items.some(item => item.command === ID), false, `${tab.id}/${panel.id}`);
    }
  }
});

test('the shortcut menu offers SELECTSIMILAR only with a selection and no capture', () => {
  const item = menuItem(READY_MENU);
  assert.ok(item, 'the selection section carries the item');
  assert.equal(item.key, MENU_KEY);
  assert.equal(item.enabled, true);
  assert.equal(menuItem({ ...READY_MENU, selection: { count: 0 } }).enabled, false);
  assert.equal(menuItem({ ...READY_MENU, selection: null }).enabled, false);
  // A capturing tool must never be overridden, even with a selection present.
  assert.equal(menuItem({ ...READY_MENU, tool: { key: 'LINE', step: 1 } }).enabled, false);
});

test('SELECTSIMILAR needs at least one selected object, and says so before the host refuses', () => {
  assert.deepEqual(
    { ...SELECTION_REQUIREMENTS.SELECTSIMILAR },
    { count: 1, reasonKey: HINT_KEY },
  );
  assert.equal(admitCommandText(ID, { status: selected(1) }).kind, 'admitted');
  assert.deepEqual(admitCommandText(ID, { status: selected(0) }), {
    kind: 'unavailable',
    command: ID,
    reasonKey: HINT_KEY,
  });
  assert.equal(unavailableReasonKey(ID, { status: selected(0) }), HINT_KEY);
  // With no drawing open the honest answer is the missing document.
  assert.equal(admitCommandText(ID, { status: NO_DOCUMENT }).reasonKey, 'plot.no_document');
});

test('the copy exists in both catalogs and names the type-only v1 boundary', () => {
  for (const key of [LABEL_KEY, HINT_KEY, MENU_KEY]) {
    for (const [name, catalog] of [['zh-CN', zh], ['en', en]]) {
      assert.equal(typeof catalog[key], 'string', `${name} missing ${key}`);
      assert.ok(catalog[key].trim().length > 0, `${name} has empty ${key}`);
    }
  }
  assert.equal(zh[LABEL_KEY], '选择类似对象');
  assert.equal(en[LABEL_KEY], 'Select Similar');
  // The hint states the type rule, not an attribute filter that v1 lacks.
  assert.match(en[HINT_KEY], /same type/i);
  assert.match(zh[HINT_KEY], /同类型/);
  assert.doesNotMatch(en[HINT_KEY], /layer|colour|color|linetype|lineweight|transparency/i);
});
