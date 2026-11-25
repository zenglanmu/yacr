// BIM-domain contracts (ADR-0053 stage 4): the Revit-derived ribbon and tray
// transcription, the domain `ribbon` slot, and the honest pending set.
//
// The fixture is a hand transcription of the reference **structure** (tab order,
// panel order, pending tool set), not of its artwork: every label is our own
// catalog string and every glyph our own line icon. A rename, reorder or invented
// live tool fails here before it ships.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { readCatalog } from './helpers.ts';
import { BIM_RIBBON_TABS } from '../src/domains/bim/ribbon.ts';
import { BIM_TRAY } from '../src/domains/bim/tray.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { LOCAL_CHROME_COMMANDS } from '../src/commands/gating.ts';
import { SHELL_ACTIONS } from '../src/ribbon/actions.ts';
import { BIM_ACTION } from '../src/ribbon/bim-panel.ts';
import { STANDARD_VIEW_KEYS } from '../src/cad/view3d.ts';
import { SET_VIEW_PRESETS } from '../src/cad/set-view.ts';
import { DOMAIN_PROFILES } from '../src/domains/domains.ts';
import {
  registerDomainUi,
  registerPluginUi,
  resetDomainUi,
  uiLayoutFor,
  unregisterPluginUi,
} from '../src/domains/ui/registry.ts';
import type { RibbonItem, RibbonPanel, RibbonTab } from '../src/ribbon/ribbon-model.ts';

const FIXTURE = new URL('./fixtures/revit-2024-bim.json', import.meta.url);

async function fixture() {
  return JSON.parse(await readFile(FIXTURE, 'utf8'));
}

function allPanels(tabs: readonly RibbonTab[]): RibbonPanel[] {
  return tabs.flatMap(tab => [...tab.panels]);
}

function allItems(tabs: readonly RibbonTab[]): readonly RibbonItem[] {
  return allPanels(tabs).flatMap(panel => [...panel.items]);
}

test('the ten tabs (File + the nine reference tabs) and their panel order are the reference order', async () => {
  const expected = await fixture();
  assert.deepEqual(BIM_RIBBON_TABS.map(tab => tab.id), expected.ribbonOrder);
  for (const tab of BIM_RIBBON_TABS) {
    assert.deepEqual(tab.panels.map(panel => panel.id), expected.tabs[tab.id], `${tab.id} panel order drifted`);
  }
});

test('the File tab leads the band and reaches every file command', () => {
  const file = BIM_RIBBON_TABS[0];
  assert.equal(file.id, 'bim.file');
  assert.equal(file.accent, true);
  const items = file.panels.flatMap(panel => [...panel.items]);
  assert.deepEqual(
    items.map(item => item.command ?? item.action ?? null),
    ['NEW', 'OPEN', 'SAVE', 'EXPORT', 'PLOT', 'CLOSE', SHELL_ACTIONS.options, SHELL_ACTIONS.domainSwitcher],
  );
});

test('the View tab owns showing/hiding the tray', () => {
  const view = BIM_RIBBON_TABS.find(tab => tab.id === 'bim.view');
  const item = view?.panels.flatMap(panel => [...panel.items]).find(entry => entry.id === 'bim.domain_tray');
  assert.equal(item?.action, SHELL_ACTIONS.domainTray);
  assert.equal(item?.pendingKey, undefined);
});

test('the tray carries the documented panel list', async () => {
  const expected = await fixture();
  assert.deepEqual(BIM_TRAY.map(panel => panel.id), expected.tray);
  // Exactly one live BIM-query launcher; every other panel is either a real body
  // or an explicit pending sentence.
  assert.equal(BIM_TRAY.filter(panel => panel.kind === 'bimQuery').length, 1);
});

test('every item is either exactly one dispatch token or an explicit pending sentence', () => {
  const knownCommands = new Set(COMMAND_CATALOG.map(command => command.id));
  // The clipboard family is a real host vocabulary but not a catalog row.
  for (const id of ['CUTCLIP', 'COPYCLIP', 'PASTECLIP']) knownCommands.add(id);
  const local = new Set(LOCAL_CHROME_COMMANDS);
  const actions = new Set<string>([...Object.values(SHELL_ACTIONS), BIM_ACTION]);
  for (const item of allItems(BIM_RIBBON_TABS)) {
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
    if (typeof item.viewPreset === 'string') {
      assert.ok((SET_VIEW_PRESETS as readonly string[]).includes(item.viewPreset), `${item.id}: unknown view preset`);
    }
    if (typeof item.action === 'string') {
      assert.ok(actions.has(item.action), `${item.id}: unknown action ${item.action}`);
    }
  }
});

test('the pending tool set is exactly the BIM tools this kernel cannot run', async () => {
  const expected = await fixture();
  const pending = [...new Set(allItems(BIM_RIBBON_TABS)
    .filter(item => typeof item.pendingKey === 'string')
    .map(item => item.id))].sort();
  assert.deepEqual(pending, [...expected.pendingTools].sort());
});

test('every label, title and pending sentence resolves in both catalogs', async () => {
  const keys = new Set<string>();
  for (const tab of BIM_RIBBON_TABS) keys.add(tab.labelKey);
  for (const panel of allPanels(BIM_RIBBON_TABS)) keys.add(panel.titleKey);
  for (const item of allItems(BIM_RIBBON_TABS)) {
    keys.add(item.labelKey);
    if (item.hintKey) keys.add(item.hintKey);
    if (item.pendingKey) keys.add(item.pendingKey);
  }
  for (const panel of BIM_TRAY) {
    keys.add(panel.titleKey);
    if (panel.pendingKey) keys.add(panel.pendingKey);
  }
  for (const locale of ['zh-CN', 'en']) {
    const catalog = await readCatalog(locale);
    for (const key of keys) assert.ok(catalog[key], `${locale} is missing ${key}`);
  }
});

test('the BIM profile is wired as a domain ribbon and no profile is a placeholder', () => {
  assert.deepEqual(
    {
      ribbon: DOMAIN_PROFILES.bim.ribbon,
      surface: DOMAIN_PROFILES.bim.surface,
      commandLine: DOMAIN_PROFILES.bim.commandLine,
      placeholder: DOMAIN_PROFILES.bim.placeholder,
    },
    { ribbon: true, surface: 'domain-ribbon', commandLine: false, placeholder: false },
  );
  assert.equal(DOMAIN_PROFILES.bim.traySide, 'left', 'Revit docks its palettes on the left');
  assert.equal(DOMAIN_PROFILES.mechanical.placeholder, false);
});

test('the registry stores the registered BIM ribbon and a patch appends panels', () => {
  resetDomainUi();
  registerDomainUi('bim', { ribbon: BIM_RIBBON_TABS, tray: BIM_TRAY });
  const before = uiLayoutFor('bim');
  assert.deepEqual(before.ribbon?.map(tab => tab.id), BIM_RIBBON_TABS.map(tab => tab.id));
  registerPluginUi({
    id: 'test.bim-extension',
    domain: 'bim',
    ribbon: [{
      id: 'bim.architecture',
      labelKey: 'bim.tab.architecture',
      panels: [{
        id: 'bim.build',
        titleKey: 'bim.panel.build',
        items: [{ id: 'EXTRA', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE' }],
      }],
    }],
  });
  const architecture = uiLayoutFor('bim').ribbon?.find(tab => tab.id === 'bim.architecture');
  const build = architecture?.panels.find(panel => panel.id === 'bim.build');
  assert.equal(build?.items.at(-1)?.id, 'EXTRA');
  assert.equal(
    before.ribbon?.find(tab => tab.id === 'bim.architecture')?.panels.find(panel => panel.id === 'bim.build')?.items.length,
    build ? build.items.length - 1 : -1,
  );
  assert.equal(unregisterPluginUi('test.bim-extension'), true);
  resetDomainUi();
});
