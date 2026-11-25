// Concept-domain contracts (ADR-0053 stage 3): the SketchUp-derived menu /
// toolbar / tray transcription, the plain-key shortcut table, and the
// domain/plugin UI slot registry.
//
// The fixture is a hand transcription of the official structure (menu order,
// toolbar groups, tray panel list, default single-key table). It is the
// contract: a rename, reorder or invented tool fails here before it ships.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { readCatalog, readCss } from './helpers.ts';
import { CONCEPT_MENUS } from '../src/domains/concept/menus.ts';
import { CONCEPT_TOOLBAR, CONCEPT_SELECT_ACTION } from '../src/domains/concept/toolbar.ts';
import { CONCEPT_TRAY } from '../src/domains/concept/tray.ts';
import { CONCEPT_SHORTCUTS, conceptShortcut } from '../src/domains/concept/shortcuts.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { LOCAL_CHROME_COMMANDS } from '../src/commands/gating.ts';
import { SHELL_ACTIONS } from '../src/ribbon/actions.ts';
import { STANDARD_VIEW_KEYS } from '../src/cad/view3d.ts';
import { DOMAIN_PROFILES } from '../src/domains/domains.ts';
import {
  pluginUiIds,
  registerDomainUi,
  registerPluginUi,
  resetDomainUi,
  uiLayoutFor,
  unregisterPluginUi,
} from '../src/domains/ui/registry.ts';
import type { RibbonPanel, RibbonTab } from '../src/ribbon/ribbon-model.ts';

const FIXTURE = new URL('./fixtures/sketchup-2024-concept.json', import.meta.url);

async function fixture() {
  return JSON.parse(await readFile(FIXTURE, 'utf8'));
}

function allPanels(tabs: readonly RibbonTab[]): RibbonPanel[] {
  return tabs.flatMap(tab => [...tab.panels]);
}

function allItems(tabs: readonly RibbonTab[], toolbar: readonly RibbonPanel[]) {
  return [...allPanels(tabs).flatMap(panel => [...panel.items]), ...toolbar.flatMap(group => [...group.items])];
}

test('the nine menus and their panel order are the reference order', async () => {
  const expected = await fixture();
  assert.deepEqual(CONCEPT_MENUS.map(tab => tab.id), expected.menuOrder);
  for (const tab of CONCEPT_MENUS) {
    assert.deepEqual(tab.panels.map(panel => panel.id), expected.menus[tab.id], `${tab.id} panel order drifted`);
  }
});

test('the tool column and tray carry the documented groups/panels', async () => {
  const expected = await fixture();
  assert.deepEqual(CONCEPT_TOOLBAR.map(group => group.id), expected.toolbar);
  assert.deepEqual(CONCEPT_TRAY.map(panel => panel.id), expected.tray);
  for (const group of CONCEPT_TOOLBAR) assert.ok(group.items.length > 0, `${group.id} must not be empty`);
});

test('every item is either exactly one dispatch token or an explicit pending sentence', async () => {
  const items = allItems(CONCEPT_MENUS, CONCEPT_TOOLBAR);
  const knownCommands = new Set(COMMAND_CATALOG.map(command => command.id));
  // The clipboard family is a real host vocabulary but not a catalog row (the
  // context menu and the Ctrl chords are its only drafting entry points).
  for (const id of ['CUTCLIP', 'COPYCLIP', 'PASTECLIP']) knownCommands.add(id);
  const local = new Set(LOCAL_CHROME_COMMANDS);
  const actions = new Set(Object.values(SHELL_ACTIONS));
  for (const item of items) {
    const tokens = [
      typeof item.command === 'string' ? item.command : null,
      typeof item.standardView === 'string' ? item.standardView : null,
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
      assert.ok(actions.has(item.action) || item.action === CONCEPT_SELECT_ACTION, `${item.id}: unknown action ${item.action}`);
    }
  }
});

test('the pending tool set is exactly the reference tools this kernel cannot run', async () => {
  const expected = await fixture();
  const pending = [...allItems(CONCEPT_MENUS, CONCEPT_TOOLBAR)]
    .filter(item => typeof item.pendingKey === 'string')
    .map(item => item.id);
  assert.deepEqual([...new Set(pending)].sort(), [...expected.pendingTools].sort());
});

test('every label, title and pending sentence resolves in both catalogs', async () => {
  const items = allItems(CONCEPT_MENUS, CONCEPT_TOOLBAR);
  const keys = new Set<string>();
  for (const tab of CONCEPT_MENUS) keys.add(tab.labelKey);
  for (const panel of [...allPanels(CONCEPT_MENUS), ...CONCEPT_TOOLBAR]) keys.add(panel.titleKey);
  for (const item of items) {
    keys.add(item.labelKey);
    if (item.hintKey) keys.add(item.hintKey);
    if (item.pendingKey) keys.add(item.pendingKey);
  }
  for (const panel of CONCEPT_TRAY) {
    keys.add(panel.titleKey);
    if (panel.pendingKey) keys.add(panel.pendingKey);
  }
  for (const locale of ['zh-CN', 'en']) {
    const catalog = await readCatalog(locale);
    for (const key of keys) assert.ok(catalog[key], `${locale} is missing ${key}`);
  }
});

test('the concept profile is wired and the placeholders stay placeholders', () => {
  assert.deepEqual(
    {
      ribbon: DOMAIN_PROFILES.concept.ribbon,
      surface: DOMAIN_PROFILES.concept.surface,
      commandLine: DOMAIN_PROFILES.concept.commandLine,
      placeholder: DOMAIN_PROFILES.concept.placeholder,
    },
    { ribbon: true, surface: 'menu', commandLine: false, placeholder: false },
  );
  for (const id of ['bim', 'mechanical']) assert.equal(DOMAIN_PROFILES[id].placeholder, true);
});

test('the shortcut table is the official one, including F=Offset and O=Orbit', async () => {
  const expected = await fixture();
  assert.deepEqual({ ...CONCEPT_SHORTCUTS }, expected.shortcuts);
  // The brief's O=Offset is not the reference mapping; the table must not drift.
  assert.equal(CONCEPT_SHORTCUTS.f, 'OFFSET');
  assert.equal(CONCEPT_SHORTCUTS.o, 'ORBIT');
});

test('concept shortcuts decline text entry, popups and terminal text steps', () => {
  assert.deepEqual(conceptShortcut({ key: 'l' }), { kind: 'command', command: 'LINE' });
  assert.deepEqual(conceptShortcut({ key: ' ' }), { kind: 'select' });
  assert.deepEqual(conceptShortcut({ key: 'Z', shiftKey: true }), { kind: 'command', command: 'FIT' });
  assert.deepEqual(conceptShortcut({ key: 'l', ctrlKey: true }), { kind: 'none' });
  assert.deepEqual(conceptShortcut({ key: 'l', metaKey: true }), { kind: 'none' });
  assert.deepEqual(conceptShortcut({ key: 'l' }, { typing: true }), { kind: 'none' });
  assert.deepEqual(conceptShortcut({ key: 'l' }, { popupOpen: true }), { kind: 'none' });
  assert.deepEqual(conceptShortcut({ key: 'l' }, { textStep: true }), { kind: 'none' });
  assert.deepEqual(conceptShortcut({ key: 'l', isComposing: true }), { kind: 'none' });
  assert.deepEqual(conceptShortcut({ key: '1' }), { kind: 'none' });
});

test('a plugin patch extends a menu, a tool group and the tray additively', () => {
  resetDomainUi();
  registerDomainUi('concept', { menu: CONCEPT_MENUS, toolbar: CONCEPT_TOOLBAR, tray: CONCEPT_TRAY });
  const before = uiLayoutFor('concept');
  registerPluginUi({
    id: 'test.extension',
    domain: '*',
    menu: [{
      id: 'tools',
      labelKey: 'concept.menu.tools',
      panels: [{
        id: 'concept.tools.other',
        titleKey: 'concept.section.other',
        items: [{ id: 'EXTRA', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE' }],
      }],
    }],
    toolbar: [{
      id: 'concept.standard',
      titleKey: 'concept.toolbar.standard',
      items: [{ id: 'EXTRA', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE' }],
    }],
    tray: [{ id: 'concept.extension', titleKey: 'concept.panel.outliner', kind: 'pending', pendingKey: 'concept.pending.panel' }],
  });
  const after = uiLayoutFor('concept');
  const tools = after.menu?.find(tab => tab.id === 'tools');
  const other = tools?.panels.find(panel => panel.id === 'concept.tools.other');
  assert.ok(other?.items.some(item => item.id === 'EXTRA'));
  assert.equal(after.toolbar?.find(group => group.id === 'concept.standard')?.items.at(-1)?.id, 'EXTRA');
  assert.ok(after.tray?.some(panel => panel.id === 'concept.extension'));
  assert.equal(before.menu?.find(tab => tab.id === 'tools')?.panels.find(panel => panel.id === 'concept.tools.other')?.items.length, 2);
  assert.deepEqual(pluginUiIds(), ['test.extension']);
  assert.equal(unregisterPluginUi('test.extension'), true);
  assert.equal(uiLayoutFor('concept').menu?.find(tab => tab.id === 'tools')?.panels.find(panel => panel.id === 'concept.tools.other')?.items.length, 2);
  assert.deepEqual(pluginUiIds(), []);
  resetDomainUi();
});

test('a malformed patch is refused instead of repaired', () => {
  resetDomainUi();
  assert.throws(() => registerPluginUi({ id: 'bad', domain: 'concept' }), /domain\.ui: patch bad contributes no slot/);
  assert.throws(() => registerPluginUi({ id: 'bad', domain: 'nope', tray: [] }), /domain\.ui: patch\.domain/);
  registerPluginUi({ id: 'once', domain: 'concept', tray: [{ id: 'x', titleKey: 'concept.panel.tags', kind: 'tags' }] });
  assert.throws(
    () => registerPluginUi({ id: 'once', domain: 'concept', tray: [{ id: 'y', titleKey: 'concept.panel.tags', kind: 'tags' }] }),
    /domain\.ui: duplicate patch/,
  );
  resetDomainUi();
});

test('the concept stylesheet frames the tool column, tray and measurements box', async () => {
  const css = await readCss();
  for (const selector of ['.concept-toolbar', '.concept-tray', '.concept-measurements', '.domain-menubar']) {
    assert.ok(css.includes(selector), `${selector} must be styled`);
  }
  assert.match(css, /\[data-domain="concept"\]/);
});
