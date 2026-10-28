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
//   * the doc-tab `+` and the layout tabs are disabled for a stated limitation, not
//     with "unsupported" or "not wired";
//   * the catalog keys those sentences need are declared in one place, so the gap
//     is reportable instead of being filled with invented copy.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readComponentsSource, readModule } from './helpers.ts';
import {
  offeredPanels, PALETTE_COMMANDS, panelLauncherCommand, RIBBON_TABS,
} from '../src/ribbon/ribbon-model.ts';
import { admitCommandText } from '../src/commands/commands.ts';
import { LOCAL_CHROME_COMMANDS } from '../src/commands/gating.ts';
import {
  ONE_DOCUMENT_KEY, ONE_LAYOUT_SPACE_KEY, PANEL_DIALOG_MISSING_KEY, PENDING_CATALOG_KEYS,
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
    ['LAYER', 'LAYISO', 'LAYUNISO', 'LAYERP', 'LAYOFF', 'LAYON'],
    'the Layers panel holds its palette toggle and the layer-state commands',
  );
  assert.deepEqual(propertyItems, ['PROPERTIES']);
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
  assert.deepEqual([...PALETTE_COMMANDS], ['LAYER', 'LAYISO', 'LAYUNISO', 'LAYERP', 'LAYOFF', 'LAYON']);
  assert.equal(offeredPanels(RIBBON_TABS.find(tab => tab.id === 'home').panels, false)
    .some(panel => panel.id === 'layers' || panel.id === 'properties'), false);
});

test('the View tab navigation cluster holds only router command tokens', () => {
  // `RESETVIEW` joined ZOOMIN/ZOOMOUT/PAN/FIT: all five are payload-free router
  // selectors, so none may be a local `action` (which would bypass the shared
  // admission) and none may carry a typed payload the name-only funnel refuses.
  //
  // `ZOOM WINDOW` / `ZOOM PREVIOUS` (2026-10-28) join the same cluster as the two
  // **argument forms** of `ZOOM`: their dispatch token is the full line
  // (`'ZOOM WINDOW'`, `'ZOOM PREVIOUS'`), which the router resolves as one
  // selector plus its word argument. They still ride `cad_command` — the same
  // funnel as every neighbour — and they add no name the host advertises, so the
  // command-string assertion below is the full line, not the item id.
  const navigate = RIBBON_TABS.find(tab => tab.id === 'view').panels.find(panel => panel.id === 'navigate');
  assert.ok(navigate, 'the View tab keeps its navigate panel');
  assert.deepEqual(
    navigate.items.map(item => item.id),
    ['ZOOMIN', 'ZOOMOUT', 'ZOOM WINDOW', 'PAN', 'FIT', 'ZOOM PREVIOUS', 'RESETVIEW'],
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
  assert.ok(gated.includes("t('diagnostics.title')"), 'the diagnostics toggle is gated too');
  assert.ok(gated.includes('shell.toggleLayers()'));
  assert.ok(gated.includes('shell.setDiagnosticsOpen(!shell.diagnosticsOpen)'));
  // The navigation commands are carrier-independent and stay outside the gate.
  assert.ok(rail.slice(0, gate).includes("'FIT', 'ZOOMIN', 'ZOOMOUT', 'PAN', 'ORBIT'"));
});

test('the carrier flag itself keeps its one reading: absent means "as today"', () => {
  // Only an explicit `false` removes the palettes; a carrier that reports nothing
  // (every native-below host, and any older chrome state) keeps them.
  assert.equal(palettesOffered({ palettesAvailable: false }), false);
  assert.equal(palettesOffered({ palettesAvailable: true }), true);
  assert.equal(palettesOffered({ palettesAvailable: undefined }), true);
  assert.equal(palettesOffered({}), true);
  assert.equal(palettesOffered(null), true);
  assert.equal(palettesOffered(undefined), true);
});

test('no component re-spells the carrier comparison inline', async () => {
  const components = await readComponentsSource();
  assert.equal(components.includes('shell.palettesAvailable !== false'), false);
  const surface = await readModule('cad/surface.ts');
  assert.equal(surface.match(/palettesAvailable !== false/g).length, 1, 'exactly one reading of the flag');
});

test('the doc-tab "+" states the one-drawing limit instead of nothing', async () => {
  const tabs = await readModule('components/DocTabs.tsx');
  assert.ok(tabs.includes("import { ONE_DOCUMENT_KEY } from '../limits.ts';"));
  assert.ok(tabs.includes('title={t(ONE_DOCUMENT_KEY)}'));
  // The `+` keeps its label and stays disabled: the limit is real, the wiring is not
  // missing, so the title must not borrow the generic unsupported sentence.
  assert.ok(tabs.includes("label={t('doctabs.new_tab')} disabled title={t(ONE_DOCUMENT_KEY)}"));
  assert.equal(tabs.includes('ribbon.command_unsupported'), false);
});

test('the layout tabs state the one-space limit, not "not wired"', async () => {
  const tab = await readModule('components/LayoutTabs.tsx');
  assert.ok(tab.includes("import { ONE_LAYOUT_SPACE_KEY } from '../limits.ts';"));
  assert.equal((tab.match(/title=\{reason\}/g) ?? []).length, 2, 'both layout affordances say why');
  assert.ok(tab.includes('const reason = t(ONE_LAYOUT_SPACE_KEY);'));
  // The old key claimed a wiring gap ("host provides no space selection") for a
  // design decision, so it must not be rendered again.
  assert.equal(tab.includes("t('layout.switch_unwired')"), false);
  assert.equal(tab.includes("t('command.unsupported')"), false);
  // Still disabled, still labelled — the limitation is stated, not hidden.
  assert.ok(tab.includes("aria-label={t('layout.tab.add')}"));
  assert.ok(tab.includes('className="layout-tab is-disabled" disabled title={reason}'));
  const addBlock = tab.slice(tab.indexOf('className="layouttabs__add"'), tab.indexOf('>', tab.indexOf('className="layouttabs__add"')));
  assert.ok(addBlock.includes('\n        disabled'), 'the new-layout button stays disabled');
});

test('the copy keys these disabled affordances need are declared in one place', async () => {
  assert.deepEqual([...PENDING_CATALOG_KEYS].sort(), [
    'doctabs.one_document',
    'layout.one_space',
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
  assert.ok(declares('ONE_DOCUMENT_KEY', 'doctabs.one_document'));
  assert.ok(declares('ONE_LAYOUT_SPACE_KEY', 'layout.one_space'));
  assert.ok(declares('PANEL_DIALOG_MISSING_KEY', 'ribbon.panel_dialog_missing'));
  assert.ok(limits.includes('export const PENDING_CATALOG_KEYS'));
});
