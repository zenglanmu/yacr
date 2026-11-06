// Contract tests for the compact layout's **reachable surfaces**.
//
// Three defects are locked here, none of them a screenshot:
//
//   * a chrome shortcut must mean something in the layout that receives it: on
//     mobile it either opens the compact equivalent (the history sheet) or is
//     not claimed at all, and the properties palette — which the compact layout
//     never renders — is refused in words;
//   * a typed `PROPERTIES` / `LAYER` is a desktop-palette selector, so the same
//     rule applies: open the floating palette on desktop, refuse with the one
//     catalog sentence on mobile (never flip a flag that paints nothing);
//   * `data-layout` has exactly one writer (the hook, on the document element),
//     and every compact rule keys on `:root[data-layout="mobile"]`;
//   * the model/layout strip that the drawer does not inherit from the desktop
//     band is present as a compact, read-only stand-in with the same structural
//     reason.
import test from 'node:test';
import assert from 'node:assert/strict';
import { DOCKED_PANEL_REFUSAL_KEY, dockedPaletteOpenable, shortcutAction } from '../src/layout/layout-surfaces.ts';
import { readCatalog, readModule } from './helpers.ts';

test('the desktop layout keeps all four chrome shortcuts', () => {
  assert.equal(shortcutAction('desktop', 'cleanScreen'), 'toggle-clean-screen');
  assert.equal(shortcutAction('desktop', 'properties'), 'toggle-properties');
  assert.equal(shortcutAction('desktop', 'commandLine'), 'toggle-command-line');
  assert.equal(shortcutAction('desktop', 'history'), 'toggle-history');
  // The desktop band renders the history panel in a text step too.
  assert.equal(shortcutAction('desktop', 'history', { textStep: true }), 'toggle-history');
});

test('the compact layout never claims a shortcut whose surface it does not render', () => {
  // The palette layer is hidden, so the properties key is refused in words, not ignored.
  assert.equal(shortcutAction('mobile', 'properties'), 'refuse-docked-panel');
  // The two bars are unconditional: there is no clean screen to toggle and no
  // docked command line to hide. The shell claims neither chord (Ctrl+0's WebKit
  // zoom reset is separately suppressed in every layout).
  assert.equal(shortcutAction('mobile', 'cleanScreen'), 'none');
  assert.equal(shortcutAction('mobile', 'commandLine'), 'none');
  // F2 opens the command bar's history sheet...
  assert.equal(shortcutAction('mobile', 'history'), 'toggle-history');
  // ...except while the bar is a tool's text prompt, where that sheet is hidden.
  assert.equal(shortcutAction('mobile', 'history', { textStep: true }), 'none');
});

test('only the desktop layout renders the desktop palettes', () => {
  assert.equal(dockedPaletteOpenable('desktop'), true);
  assert.equal(dockedPaletteOpenable('mobile'), false);
});

test('App routes every shortcut through the layout table', async () => {
  const app = await readModule('App.tsx');
  const runner = await readModule('layout/shell-shortcuts.ts');
  assert.ok(runner.includes('shortcutAction(layout, shortcut, { textStep })'), 'the pure table decides');
  assert.ok(app.includes('createShortcutRunner(layout, textStep, t, {'), 'App uses the one runner');
  for (const shortcut of ['cleanScreen', 'properties', 'commandLine', 'history']) {
    assert.ok(app.includes(`runShortcut('${shortcut}')`), `${shortcut} is routed, never toggled inline`);
  }
  // Each chord is prevented only when this layout really handled it; otherwise
  // the platform keeps the key (no half-handled chord, no invisible state).
  for (const shortcut of ['cleanScreen', 'properties', 'commandLine', 'history']) {
    assert.ok(
      app.includes(`if (runShortcut('${shortcut}')) event.preventDefault();`),
      `${shortcut} is prevented only when handled`,
    );
  }
});

test('a typed PROPERTIES / LAYER is refused on mobile and opens the palette on desktop', async () => {
  const app = await readModule('App.tsx');
  const selectors = await readModule('commands/local-selectors.ts');
  // Both selectors go through the one chrome-local table.
  assert.ok(app.includes('if (runLocalSelector(text, options.fromInput === true)) return;'));
  assert.ok(selectors.includes('if (isPropertiesCommand(text))'));
  assert.ok(selectors.includes('if (isLayersCommand(text))'));
  const guardAt = selectors.indexOf('dockedPaletteOpenable(layout)');
  assert.ok(guardAt > 0, 'the layout check is paired with each desktop palette');
  assert.ok(selectors.includes('LOCAL_SELECTOR_PANEL_KEYS.properties'));
  assert.ok(selectors.includes('LOCAL_SELECTOR_PANEL_KEYS.layers'));
  assert.ok(app.includes('setLayersFocus(value => value + 1)'), 'the desktop LAYER refresh is unchanged');
  // The refusal is the catalog's own sentence, naming the panel that is missing.
  assert.ok(selectors.includes('DOCKED_PANEL_REFUSAL_KEY'), 'the typed path uses the one refusal key');
  assert.ok(selectors.includes('panel: translate(result.panelKey)'), 'and the panel label');
});

test('the refusal copy is one catalog sentence in both languages', async () => {
  const en = await readCatalog('en');
  const zh = await readCatalog('zh-CN');
  for (const catalog of [en, zh]) {
    assert.equal(typeof catalog[DOCKED_PANEL_REFUSAL_KEY], 'string', DOCKED_PANEL_REFUSAL_KEY);
    assert.ok(catalog[DOCKED_PANEL_REFUSAL_KEY].includes('{panel}'), 'the placeholder names the panel');
    assert.ok(catalog[DOCKED_PANEL_REFUSAL_KEY].trim().length > 0);
  }
  assert.deepEqual(Object.keys(zh).sort(), Object.keys(en).sort(), 'identical key sets');
});

test('data-layout has one writer, on the document element', async () => {
  const hook = await readModule('layout/use-layout-mode.ts');
  const writes = hook.match(/setAttribute\(LAYOUT_ATTRIBUTE/g) ?? [];
  assert.equal(writes.length, 1, 'the hook is the only writer');
  assert.ok(hook.includes('document.documentElement.setAttribute(LAYOUT_ATTRIBUTE, next)'));

  const app = await readModule('App.tsx');
  assert.equal(app.includes('data-layout'), false, 'the shell renders no second copy');

  // Every compact rule keys on the node that actually carries the attribute.
  const css = (await readModule('mobile.css')).replace(/\/\*[\s\S]*?\*\//g, '');
  const guards = css.match(/data-layout="mobile"/g) ?? [];
  assert.ok(guards.length > 10, 'the compact rules are still guarded');
  const roots = css.match(/:root\[data-layout="mobile"\]/g) ?? [];
  assert.equal(roots.length, guards.length, 'every guard starts at :root');
});

test('the drawer keeps the real model/layout strip the desktop band shows', async () => {
  const drawer = await readModule('components/MobileRibbonDrawer.tsx');
  assert.ok(drawer.includes('mobile-drawer__space'), 'a compact space strip exists');
  assert.equal(drawer.includes('ONE_LAYOUT_SPACE_KEY'), false, 'the one-space claim is gone');
  // Both layouts read the same projection from the same `cad_status`: no second
  // source, no hand-built chip list.
  assert.ok(drawer.includes("import { layoutTabsView, layoutUnsupportedTitle } from '../layout/layout-tabs.ts';"));
  assert.ok(drawer.includes('const spaces = layoutTabsView(shell.status);'));
  assert.ok(drawer.includes("t('layout.tab.model')"), 'the model space keeps the desktop label');
  assert.ok(drawer.includes('layoutUnsupportedTitle(t, space)'), 'unsupported reasons are the desktop ones');
  assert.ok(drawer.includes('shell.runCommand(command)'), 'a supported tab dispatches the same SPACE line');
  // References and the Font Manager are reachable on the compact layout without
  // the desktop palette layer: both are in-DOM sheets in this tree.
  assert.ok(drawer.includes('shell.setReferencesOpen(true)'), 'the drawer opens the references sheet');
  assert.ok(drawer.includes('shell.setFontsOpen(true)'), 'the drawer opens the Font Manager sheet');

  const tabs = await readModule('components/LayoutTabs.tsx');
  assert.ok(tabs.includes('layoutTabsView(shell.status)'), 'the desktop strip reads the same projection');

  const css = await readModule('mobile.css');
  assert.ok(css.includes('.mobile-drawer__space-chip'), 'the strip has its compact styling');
  assert.ok(css.includes('.mobile-drawer__space-note'), 'the reason is visible text, not a tooltip');
  assert.ok(css.includes('.mobile-drawer__space-scroll'), 'many layouts scroll instead of shrinking');
  assert.ok(css.includes('.references--floating'), 'the compact references sheet has its styling');
  assert.ok(css.includes('.fonts--floating'), 'the compact Font Manager sheet has its styling');
  assert.ok(css.includes('.fonts--floating .ui-icon-button'), 'the sheet close control is a 44px target');
});
