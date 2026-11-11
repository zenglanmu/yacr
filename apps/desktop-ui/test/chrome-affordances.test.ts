// Contract tests for the chrome affordances that used to look live but were not.
//
// Four reader-facing facts are locked here, all as model/source assertions rather
// than screenshots:
//
//   * every ribbon panel launcher resolves through *data* (`launcherCommand`), the
//     Layers arrow runs `LAYER`, the Properties arrow runs `PROPERTIES`, and a
//     panel with no dialog behind it resolves to `null` — and the component never
//     dispatches a launcher without the shared admission;
//   * one carrier helper (`palettesOffered`) decides for **every** nav-rail palette
//     toggle, so a `dom-canvas` carrier cannot show a toggle for a panel that is
//     never rendered;
//   * the doc-tab strip has no `+` (tabs were cancelled) and its open-drawing
//     tab carries the real `CLOSE` control, while the layout strip renders the
//     host's real tabs and dispatches `SPACE` instead of a one-space placeholder;
//   * the catalog keys those sentences need are declared in one place, so the gap
//     is reportable instead of being filled with invented copy.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readComponentsSource, readModule } from './helpers.ts';
import {
  offeredItems, offeredPanels, PALETTE_COMMANDS, panelLauncherCommand, RIBBON_TABS,
} from '../src/ribbon/ribbon-model.ts';
import { admitCommandText } from '../src/commands/commands.ts';
import { LOCAL_CHROME_COMMANDS } from '../src/commands/gating.ts';
import {
  PANEL_DIALOG_MISSING_KEY, PENDING_CATALOG_KEYS,
} from '../src/limits.ts';
import { palettesOffered } from '../src/cad/surface.ts';

const ALL_PANELS = RIBBON_TABS.flatMap(tab => tab.panels.map(panel => ({ tab: tab.id, panel })));
const LAUNCHER_PANELS = ALL_PANELS.filter(({ panel }) => panel.launcher);

/** Catalog key -> the panel ids whose launcher names it, from the model alone. */
function launcherMatrix() {
  const matrix = new Map();
  for (const { panel } of LAUNCHER_PANELS) {
    const command = panelLauncherCommand(panel);
    matrix.set(panel.id, command);
  }
  return matrix;
}

test('every panel launcher resolves to a command token or to no target at all', () => {
  // Every panel answers, launcher or not: a panel without the arrow is "no
  // target", never an undefined read.
  for (const { tab, panel } of ALL_PANELS) {
    const command = panelLauncherCommand(panel);
    assert.ok(command === null || typeof command === 'string', `${tab}/${panel.id}`);
    if (!panel.launcher) assert.equal(command, null, `${tab}/${panel.id} has no arrow`);
  }
  // The panels that do carry the arrow are the ones with a real target plus the
  // ones that stay honestly empty.
  assert.ok(LAUNCHER_PANELS.length >= 5, 'the launcher arrow is still a ribbon convention');
  for (const { panel } of LAUNCHER_PANELS) {
    const command = panelLauncherCommand(panel);
    if (command === null) continue;
    assert.equal(command, command.trim(), `${panel.id}: a declared target is a trimmed token`);
    assert.ok(command.length > 0, `${panel.id}: a declared target is never empty`);
  }
});

test('the Layers arrow runs LAYER and the Properties arrow runs PROPERTIES', () => {
  const matrix = launcherMatrix();
  assert.equal(matrix.get('layers'), 'LAYER');
  assert.equal(matrix.get('properties'), 'PROPERTIES');
  // The panel titles name the very surface the arrow opens, so the tooltip and the
  // panel can never describe two different things.
  const byId = Object.fromEntries(ALL_PANELS.map(({ panel }) => [panel.id, panel]));
  assert.equal(byId.layers.titleKey, 'layers.panel');
  assert.equal(byId.properties.titleKey, 'properties.panel');
  const layerItems = byId.layers.items.map(item => item.id);
  const propertyItems = byId.properties.items.map(item => item.id);
  // The Layers panel holds the palette toggle **plus** the layer-state commands
  // (2026-10-26): `LAYISO`/`LAYUNISO`, the previous-state restore (`LAYERP`,
  // 2026-10-27) and `LAYOFF`/`LAYON` are router commands that change which layers
  // are visible, which is the one state this palette is the surface for.
  assert.deepEqual(
    layerItems,
    ['LAYER', 'LAYISO', 'LAYUNISO', 'LAYERP', 'LAYOFF', 'LAYON', 'LAYCUR', 'LAYMCUR', 'LAYFRZ', 'LAYTHW', 'LAYLCK', 'LAYULK', 'SETBYLAYER', 'LAYMCH', 'LAYMRG', 'LAYDEL', 'LAYERSTATE', 'LAYWALK'],
    'the Layers panel holds its palette toggle and the layer-state commands',
  );
  assert.deepEqual(propertyItems, ['PROPERTIES', 'LIST', 'MASSPROP']);
});

test('the panels with no dialog behind them stay disabled instead of launching anything', () => {
  const matrix = launcherMatrix();
  // Draw/Modify lead the Draw tab, Measure the Measure tab, Navigate the View tab:
  // none of them has a settings dialog, so none may carry a command token.
  for (const id of ['modify', 'measure', 'navigate']) {
    assert.ok(matrix.has(id), `${id} keeps its arrow`);
    assert.equal(matrix.get(id), null, `${id} must not invent a target`);
  }
  for (const [id, command] of matrix) {
    if (['layers', 'properties'].includes(id)) continue;
    assert.equal(command, null, `${id} has no dialog yet`);
  }
});

test('every launcher target is a chrome-local command, so admission always answers it', () => {
  for (const { tab, panel } of LAUNCHER_PANELS) {
    const command = panelLauncherCommand(panel);
    if (command === null) continue;
    assert.ok(LOCAL_CHROME_COMMANDS.includes(command), `${tab}/${panel.id}: ${command} is chrome-local`);
    // No document, no host vocabulary and an in-flight command are all irrelevant
    // to these selectors — which is why the arrow never needs a second local gate.
    assert.equal(admitCommandText(command, { status: null, pending: true }).kind, 'admitted', command);
    assert.equal(admitCommandText(command, { status: null }).kind, 'admitted', command);
  }
  // Both palette toggles the carrier filter knows are also launcher targets, so a
  // carrier without the palettes drops the arrows with the panels. The layer-state
  // commands ride the same filter (`2026-10-26`: the isolate pair, then the off/on
  // pair): the layers palette is the only surface that shows what they did, so
  // those buttons are not offered where it never renders.
  assert.deepEqual([...PALETTE_COMMANDS], ['LAYER', 'LAYISO', 'LAYUNISO', 'LAYERP', 'LAYOFF', 'LAYON', 'LAYCUR', 'LAYMCUR', 'LAYFRZ', 'LAYTHW', 'LAYLCK', 'LAYULK', 'SETBYLAYER', 'LAYMCH', 'LAYMRG', 'LAYDEL', 'LAYERSTATE', 'LAYWALK', 'FONTS']);
  // The Layers panel is all palette state, so it drops whole. The Properties
  // panel now also holds the router command LIST, so it survives with that one
  // button while the PROPERTIES palette toggle is filtered out.
  const offered = offeredPanels(RIBBON_TABS.find(tab => tab.id === 'home').panels, false);
  assert.equal(offered.some(panel => panel.id === 'layers'), false);
  const properties = offered.find(panel => panel.id === 'properties');
  assert.ok(properties, 'the Properties panel keeps its non-palette command');
  assert.deepEqual(offeredItems(properties.items, false).map(item => item.id), ['LIST', 'MASSPROP']);
  // The Font Manager is the same kind of desktop-only palette surface: its Annotate entry is
  // dropped with the palettes instead of opening a panel the carrier never mounts.
  const annotate = RIBBON_TABS.find(tab => tab.id === 'annotate').panels.find(panel => panel.id === 'annotation');
  assert.equal(annotate.items.some(item => item.id === 'FONTS'), true);
  assert.equal(offeredItems(annotate.items, false).some(item => item.id === 'FONTS'), false);
});

test('the View tab navigation cluster holds only router command tokens', () => {
  // `RESETVIEW` joined ZOOMIN/ZOOMOUT/PAN/FIT: all five are payload-free router
  // selectors, so none may be a local `action` (which would bypass the shared
  // admission) and none may carry a typed payload the name-only funnel refuses.
  //
  // `ZOOM WINDOW` / `ZOOM OBJECT` / `ZOOM PREVIOUS` (2026-10-28, object form
  // wired 2026-11-11) join the same cluster as the three **argument forms** of
  // `ZOOM`: their dispatch token is the full line (`'ZOOM WINDOW'`,
  // `'ZOOM OBJECT'`, `'ZOOM PREVIOUS'`), which the router resolves as one
  // selector plus its word argument. They still ride `cad_command` — the same
  // funnel as every neighbour — and they add no name the host advertises, so the
  // command-string assertion below is the full line, not the item id.
  const navigate = RIBBON_TABS.find(tab => tab.id === 'view').panels.find(panel => panel.id === 'navigate');
  assert.ok(navigate, 'the View tab keeps its navigate panel');
  assert.deepEqual(
    navigate.items.map(item => item.id),
    ['ZOOMIN', 'ZOOMOUT', 'ZOOM WINDOW', 'ZOOM OBJECT', 'PAN', 'FIT', 'ZOOM PREVIOUS', 'RESETVIEW'],
  );
  for (const item of navigate.items) {
    assert.equal(item.command, item.id, `${item.id} keeps its own selector`);
    assert.equal(item.action, undefined, `${item.id} must not be a local action`);
    assert.equal(item.standardView, undefined, `${item.id} carries no payload`);
  }
  // The new entry adds a command, not a launcher target: the panel's arrow still
  // has no dialog behind it, so it stays honestly null instead of opening one.
  assert.equal(panelLauncherCommand(navigate), null);
  // Its copy is the catalog's, never an inline literal.
  assert.equal(navigate.items.find(item => item.id === 'RESETVIEW').labelKey, 'view.reset');
});

test('a malformed launcher token is no target, never a live empty command', () => {
  assert.equal(panelLauncherCommand({ launcher: true }), null);
  assert.equal(panelLauncherCommand({ launcher: true, launcherCommand: null }), null);
  assert.equal(panelLauncherCommand({ launcher: true, launcherCommand: '' }), null);
  assert.equal(panelLauncherCommand({ launcher: true, launcherCommand: 42 }), null);
  assert.equal(panelLauncherCommand(undefined), null);
  assert.equal(panelLauncherCommand({ launcher: true, launcherCommand: 'LAYER' }), 'LAYER');
});

test('the ribbon reads the launcher target from the model, never from a panel id', async () => {
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('const command = panelLauncherCommand(panel);'));
  // Admission is the only gate, and the dispatch is behind it: an unadmitted
  // launcher keeps no click handler at all.
  assert.ok(ribbon.includes('const enabled = shell.canCommand(command);'));
  assert.ok(ribbon.includes('disabled={!enabled}'));
  assert.ok(ribbon.includes('onClick={enabled ? () => shell.runCommand(command) : undefined}'));
  // No ad-hoc id anywhere in the component.
  assert.equal(ribbon.includes("runCommand('LAYER')"), false);
  assert.equal(ribbon.includes("runCommand('PROPERTIES')"), false);
  assert.equal(ribbon.includes("panel.id === 'layers'"), false);
  // A launcher with no target says why in its own words, not with the generic
  // "command not available" sentence it used to reuse.
  assert.ok(ribbon.includes('title={t(PANEL_DIALOG_MISSING_KEY, { panel: t(panel.titleKey) })}'));
  const launcherBlock = ribbon.slice(ribbon.indexOf('function PanelLauncher'), ribbon.indexOf('function Panel('));
  assert.equal(launcherBlock.includes('ribbon.command_unsupported'), false);
});

test('one carrier helper decides for every nav-rail palette toggle', async () => {
  const rail = await readModule('components/NavRail.tsx');
  // One gate, one helper, both toggles inside it.
  assert.ok(rail.includes("import { palettesOffered } from '../cad/surface.ts';"));
  assert.ok(rail.includes('const palettes = palettesOffered(shell);'));
  assert.equal(rail.includes('shell.palettesAvailable !== false'), false, 'the rule is not re-spelled inline');
  const gate = rail.indexOf('{palettes ? (');
  assert.ok(gate > 0, 'both palette toggles sit behind the same gate');
  const gated = rail.slice(gate, rail.indexOf(') : null}', gate));
  assert.ok(gated.includes("t('layers.panel')"), 'the layers toggle is gated');
  assert.ok(gated.includes("t('fileinfo.title')"), 'the file info toggle is gated too');
  assert.ok(gated.includes("t('diagnostics.title')"), 'the diagnostics toggle is gated too');
  assert.ok(gated.includes('shell.toggleLayers()'));
  assert.ok(gated.includes('shell.setFileInfoOpen(!shell.fileInfoOpen)'));
  assert.ok(gated.includes('shell.setDiagnosticsOpen(!shell.diagnosticsOpen)'));
  // The navigation commands are carrier-independent and stay outside the gate.
  assert.ok(rail.slice(0, gate).includes("'FIT', 'ZOOMIN', 'ZOOMOUT', 'PAN', 'ORBIT'"));
});

test('the carrier flag itself keeps its one reading: absent means "as today"', () => {
  // Only an explicit `false` removes the palettes; a carrier that reports nothing
  // (every native-below host, and any older chrome state) keeps them.
  assert.equal(palettesOffered({ status: { hostProfile: { palettesAvailable: false } } }), false);
  assert.equal(palettesOffered({ status: { hostProfile: { palettesAvailable: true } } }), true);
  assert.equal(palettesOffered({ status: { hostProfile: { palettesAvailable: undefined } } }), true);
  assert.equal(palettesOffered({ status: { hostProfile: {} } }), true);
  assert.equal(palettesOffered({}), true);
  assert.equal(palettesOffered(null), true);
  assert.equal(palettesOffered(undefined), true);
  assert.equal(palettesOffered(null), true);
  assert.equal(palettesOffered(undefined), true);
});

test('no component re-spells the carrier comparison inline', async () => {
  const components = await readComponentsSource();
  assert.equal(components.includes('shell.palettesAvailable !== false'), false);
  const surface = await readModule('cad/surface.ts');
  assert.equal(surface.match(/palettesAvailable !== false/g).length, 1, 'exactly one reading of the flag');
});

test('the doc-tab strip has no new-tab control and closes the open drawing', async () => {
  const tabs = await readModule('components/DocTabs.tsx');
  // One drawing per session, and tabs were cancelled: a `+` here would promise a
  // second tab the product does not have. It is gone with its limitation copy.
  assert.equal(tabs.includes('icon="plus"'), false);
  assert.equal(tabs.includes('doctabs.new_tab'), false);
  assert.equal(tabs.includes('ONE_DOCUMENT_KEY'), false);
  // The open drawing carries the real close control instead: the command that
  // returns the chrome to the start page.
  assert.ok(tabs.includes('icon="close"'));
  assert.ok(tabs.includes("label={t('doctabs.close')}"));
  assert.ok(tabs.includes("shell.runCommand('CLOSE')"));
  // The press belongs to the close control, never to the tab surface around it.
  assert.ok(tabs.includes('event.stopPropagation()'));
  assert.equal(tabs.includes('ribbon.command_unsupported'), false);
});

test('the layout strip renders real tabs instead of a one-space placeholder', async () => {
  const tab = await readModule('components/LayoutTabs.tsx');
  assert.ok(tab.includes("import { layoutTabsView, layoutUnsupportedTitle } from '../layout/layout-tabs.ts';"));
  assert.equal(tab.includes('ONE_LAYOUT_SPACE_KEY'), false, 'the one-space claim is gone');
  assert.equal(tab.includes('layout.tab.add'), false, 'no dead new-layout button');
  // The same three states the old placeholder could not express: model, a real
  // active paper layout, and a disabled tab with its stable reason.
  assert.ok(tab.includes('const tabs = layoutTabsView(shell.status);'));
  assert.ok(tab.includes('role="tablist"'));
  assert.ok(tab.includes('aria-selected={tab.active}'));
  assert.ok(tab.includes('disabled={disabled}'));
  assert.ok(tab.includes('layoutUnsupportedTitle(t, tab)'));
  assert.ok(tab.includes('shell.runCommand(command)'));
});

test('the copy keys these disabled affordances need are declared in one place', async () => {
  assert.deepEqual([...PENDING_CATALOG_KEYS].sort(), [
    'ribbon.panel_dialog_missing',
  ]);
  // They are referenced as constants (never as inline literals), which is what lets
  // the catalogs be the only copy source while the keys are still missing.
  const source = `${await readModule('components/DocTabs.tsx')}${await readModule('components/LayoutTabs.tsx')}`
    + `${await readModule('components/Ribbon.tsx')}`;
  for (const key of PENDING_CATALOG_KEYS) assert.equal(source.includes(`t('${key}')`), false, key);
  // If the catalog lane has already added them, both locales must have them and the
  // launcher sentence must keep its placeholder.
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const key of PENDING_CATALOG_KEYS) {
    assert.equal(typeof zh[key] === 'string', typeof en[key] === 'string', `${key} must land in both or neither`);
  }
  if (typeof zh[PANEL_DIALOG_MISSING_KEY] === 'string') {
    assert.match(zh[PANEL_DIALOG_MISSING_KEY], /\{panel\}/);
    assert.match(en[PANEL_DIALOG_MISSING_KEY], /\{panel\}/);
  }
});

test('the pending keys are named by the module that owns the limits', async () => {
  const limits = await readModule('limits.ts');
  // The constants carry an explicit `string` annotation (they feed one
  // heterogeneous key list), so the declaration is matched with or without it.
  const declares = (name, key) => new RegExp(
    `export const ${name}(?:: string)? = '${key.replaceAll('.', '\\.')}';`,
  ).test(limits);
  // One place to read what the chrome still needs from the catalog.
  assert.equal(declares('ONE_DOCUMENT_KEY', 'doctabs.one_document'), false);
  assert.ok(declares('PANEL_DIALOG_MISSING_KEY', 'ribbon.panel_dialog_missing'));
  assert.ok(limits.includes('export const PENDING_CATALOG_KEYS'));
});

test('the File tab Plugins entry stays a chrome-local action, never a launcher target', () => {
  const file = RIBBON_TABS.find(tab => tab.id === 'file');
  const utilities = file.panels.find(panel => panel.id === 'utilities');
  const plugins = utilities.items.find(item => item.id === 'plugins');
  assert.ok(plugins, 'the File utilities panel carries the direct Plugins entry');
  assert.equal(plugins.labelKey, 'file.plugins');
  assert.equal(plugins.action, 'plugins');
  assert.equal(plugins.icon, 'settings');
  // No router token and no view payload: the Options dialog and its selected
  // page are chrome-local, so nothing here can leak into `cad_command`.
  assert.equal(plugins.command, undefined);
  assert.equal(plugins.standardView, undefined);
  // The utilities panel keeps no launcher arrow: an entry inside it never
  // becomes a target, so no launcher can point at a non-command page.
  assert.equal(panelLauncherCommand(utilities), null);
});
