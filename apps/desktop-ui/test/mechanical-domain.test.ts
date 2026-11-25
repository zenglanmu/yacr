// Mechanical-domain contracts (ADR-0053 mechanical stage): the preliminary
// SolidWorks-derived ribbon/tray structure, the FeatureManager tree wire and the
// honest pending set.
//
// SolidWorks' CommandManager is contextual, and this build has no document-type
// model, so the fixture pins the fixed subset this build models rather than the
// whole reference product. Every label is our own catalog string and every glyph
// our own line icon.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { readCatalog } from './helpers.ts';
import { MECHANICAL_RIBBON_TABS } from '../src/domains/mechanical/ribbon.ts';
import { MECHANICAL_TRAY } from '../src/domains/mechanical/tray.ts';
import {
  MECHANICAL_TREE_COMMAND,
  bodyKindKey,
  featureOperationKey,
  mechanicalStatusKey,
  parseMechanicalTreeAnswer,
  readMechanicalTree,
} from '../src/domains/mechanical/tree.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { LOCAL_CHROME_COMMANDS } from '../src/commands/gating.ts';
import { SHELL_ACTIONS } from '../src/ribbon/actions.ts';
import { STANDARD_VIEW_KEYS } from '../src/cad/view3d.ts';
import { DOMAIN_IDS, DOMAIN_PROFILES } from '../src/domains/domains.ts';
import {
  registerDomainUi,
  registerPluginUi,
  resetDomainUi,
  uiLayoutFor,
  unregisterPluginUi,
} from '../src/domains/ui/registry.ts';
import type { RibbonItem, RibbonPanel, RibbonTab } from '../src/ribbon/ribbon-model.ts';

const FIXTURE = new URL('./fixtures/solidworks-2024-mechanical.json', import.meta.url);

async function fixture() {
  return JSON.parse(await readFile(FIXTURE, 'utf8'));
}

function allPanels(tabs: readonly RibbonTab[]): RibbonPanel[] {
  return tabs.flatMap(tab => [...tab.panels]);
}

function allItems(tabs: readonly RibbonTab[]): readonly RibbonItem[] {
  return allPanels(tabs).flatMap(panel => [...panel.items]);
}

test('the eight tabs and their panel order follow the fixture', async () => {
  const expected = await fixture();
  assert.deepEqual(MECHANICAL_RIBBON_TABS.map(tab => tab.id), expected.ribbonOrder);
  for (const tab of MECHANICAL_RIBBON_TABS) {
    assert.deepEqual(tab.panels.map(panel => panel.id), expected.tabs[tab.id], `${tab.id} panel order drifted`);
  }
});

test('the File tab leads the band and reaches every file command', () => {
  const file = MECHANICAL_RIBBON_TABS[0];
  assert.equal(file.id, 'mechanical.file');
  assert.equal(file.accent, true);
  const items = file.panels.flatMap(panel => [...panel.items]);
  assert.deepEqual(
    items.map(item => item.command ?? item.action ?? null),
    ['NEW', 'OPEN', 'SAVE', 'EXPORT', 'PLOT', 'CLOSE', SHELL_ACTIONS.options, SHELL_ACTIONS.domainSwitcher],
  );
});

test('the View tab owns showing/hiding the FeatureManager tray', () => {
  const view = MECHANICAL_RIBBON_TABS.find(tab => tab.id === 'mechanical.view');
  const item = view?.panels.flatMap(panel => [...panel.items]).find(entry => entry.id === 'mechanical.domain_tray');
  assert.equal(item?.action, SHELL_ACTIONS.domainTray);
  assert.equal(item?.pendingKey, undefined);
});

test('the tray carries the FeatureManager, PropertyManager and the honest pending panels', async () => {
  const expected = await fixture();
  assert.deepEqual(MECHANICAL_TRAY.map(panel => panel.id), expected.tray);
  assert.equal(MECHANICAL_TRAY.filter(panel => panel.kind === 'mechanicalTree').length, 1);
  assert.equal(MECHANICAL_TRAY.filter(panel => panel.kind === 'entityInfo').length, 1);
  for (const panel of MECHANICAL_TRAY.filter(entry => entry.kind === 'pending')) {
    assert.ok(panel.pendingKey, `${panel.id} needs its own sentence`);
  }
});

test('every item is either exactly one dispatch token or an explicit pending sentence', () => {
  const knownCommands = new Set(COMMAND_CATALOG.map(command => command.id));
  for (const id of ['CUTCLIP', 'COPYCLIP', 'PASTECLIP']) knownCommands.add(id);
  const local = new Set(LOCAL_CHROME_COMMANDS);
  const actions = new Set<string>(Object.values(SHELL_ACTIONS));
  for (const item of allItems(MECHANICAL_RIBBON_TABS)) {
    const tokens = [
      typeof item.command === 'string' ? item.command : null,
      typeof item.standardView === 'string' ? item.standardView : null,
      typeof item.viewPreset === 'string' ? item.viewPreset : null,
      typeof item.action === 'string' ? item.action : null,
    ].filter(value => value !== null);
    const pending = typeof item.pendingKey === 'string';
    assert.ok(tokens.length + (pending ? 1 : 0) === 1, `${item.id} must carry one token or one pending key`);
    if (pending) {
      assert.equal(tokens.length, 0, `${item.id} must not pretend to dispatch`);
      continue;
    }
    if (typeof item.command === 'string') {
      assert.ok(knownCommands.has(item.command) || local.has(item.command), `${item.id}: unknown command ${item.command}`);
    }
    if (typeof item.standardView === 'string') {
      assert.ok(STANDARD_VIEW_KEYS.includes(item.standardView), `${item.id}: unknown standard view`);
    }
    if (typeof item.action === 'string') {
      assert.ok(actions.has(item.action), `${item.id}: unknown action ${item.action}`);
    }
  }
});

test('the pending tool set is exactly the mechanical tools this kernel cannot run', async () => {
  const expected = await fixture();
  const pending = [...new Set(allItems(MECHANICAL_RIBBON_TABS)
    .filter(item => typeof item.pendingKey === 'string')
    .map(item => item.id))].sort();
  assert.deepEqual(pending, [...expected.pendingTools].sort());
});

test('every label, title and pending sentence resolves in both catalogs', async () => {
  const keys = new Set<string>();
  for (const tab of MECHANICAL_RIBBON_TABS) keys.add(tab.labelKey);
  for (const panel of allPanels(MECHANICAL_RIBBON_TABS)) keys.add(panel.titleKey);
  for (const item of allItems(MECHANICAL_RIBBON_TABS)) {
    keys.add(item.labelKey);
    if (item.hintKey) keys.add(item.hintKey);
    if (item.pendingKey) keys.add(item.pendingKey);
  }
  for (const panel of MECHANICAL_TRAY) {
    keys.add(panel.titleKey);
    if (panel.pendingKey) keys.add(panel.pendingKey);
  }
  for (const key of ['mechanical.tree.empty', 'mechanical.tree.refresh', 'mechanical.tree.loading',
    'mechanical.tree.no_rows', 'mechanical.tree.error', 'mechanical.tree.truncated']) keys.add(key);
  for (const locale of ['zh-CN', 'en']) {
    const catalog = await readCatalog(locale);
    for (const key of keys) assert.ok(catalog[key], `${locale} is missing ${key}`);
  }
});

test('the mechanical profile is wired as a domain ribbon with a left tray', () => {
  assert.deepEqual(
    {
      ribbon: DOMAIN_PROFILES.mechanical.ribbon,
      surface: DOMAIN_PROFILES.mechanical.surface,
      commandLine: DOMAIN_PROFILES.mechanical.commandLine,
      placeholder: DOMAIN_PROFILES.mechanical.placeholder,
    },
    { ribbon: true, surface: 'domain-ribbon', commandLine: false, placeholder: false },
  );
  assert.equal(DOMAIN_PROFILES.mechanical.traySide, 'left', 'SolidWorks docks the FeatureManager on the left');
  assert.deepEqual([...DOMAIN_IDS].filter(id => DOMAIN_PROFILES[id].placeholder), []);
});

test('the registry stores the registered mechanical ribbon and a patch appends panels', () => {
  resetDomainUi();
  registerDomainUi('mechanical', { ribbon: MECHANICAL_RIBBON_TABS, tray: MECHANICAL_TRAY });
  const before = uiLayoutFor('mechanical');
  assert.deepEqual(before.ribbon?.map(tab => tab.id), MECHANICAL_RIBBON_TABS.map(tab => tab.id));
  registerPluginUi({
    id: 'test.mechanical-extension',
    domain: 'mechanical',
    tray: [{ id: 'mechanical.tree', titleKey: 'mechanical.panel.tree', kind: 'mechanicalTree' }],
  });
  assert.equal(uiLayoutFor('mechanical').tray?.length, MECHANICAL_TRAY.length);
  assert.equal(unregisterPluginUi('test.mechanical-extension'), true);
  resetDomainUi();
});

test('the tree answer is normalised without inventing rows or a complete page', () => {
  const answer = parseMechanicalTreeAnswer({
    revision: 7,
    rows: [
      { kind: 'feature', id: 7, token: 'extrude', status: 'schema_only', order: 0, inputs: 1, parameters: 2 },
      { kind: 'body', id: 9, token: 'solid', status: 'ready', order: null, inputs: 0, parameters: 0 },
      { kind: 'body', id: '9', token: 'solid', status: 'ready' },
      { kind: 'body', id: 10, token: '', status: 'ready' },
      null,
    ],
    total: 12,
    truncated: true,
  });
  assert.equal(answer.revision, 7);
  assert.equal(answer.rows.length, 2, 'structurally unusable rows are dropped');
  assert.equal(answer.total, 12, 'the host total survives a short page');
  assert.equal(answer.truncated, true);
  assert.deepEqual(answer.rows[0], {
    kind: 'feature', id: 7, token: 'extrude', status: 'schema_only', order: 0, inputs: 1, parameters: 2,
  });
  assert.equal(answer.rows[1].order, null);
  // A missing/empty payload is an empty answer, never a guessed tree.
  assert.deepEqual(parseMechanicalTreeAnswer(null), { revision: 0, rows: [], total: 0, truncated: false });
});

test('the tree token maps name only what this build knows', () => {
  assert.equal(featureOperationKey('extrude'), 'mechanical.op.extrude');
  assert.equal(featureOperationKey('boolean_subtract'), 'mechanical.op.boolean_subtract');
  assert.equal(featureOperationKey('future_op'), null);
  assert.equal(bodyKindKey('solid'), 'mechanical.body.solid');
  assert.equal(bodyKindKey('future_body'), null);
  assert.equal(mechanicalStatusKey('schema_only'), 'mechanical.status.schema_only');
  assert.equal(mechanicalStatusKey('future_status'), null);
});

test('the tree read goes through the one IPC command', async () => {
  const calls: string[] = [];
  const answer = await readMechanicalTree(async (command: string) => {
    calls.push(command);
    return { revision: 1, rows: [], total: 0, truncated: false };
  });
  assert.deepEqual(calls, [MECHANICAL_TREE_COMMAND]);
  assert.equal(answer.revision, 1);
});
