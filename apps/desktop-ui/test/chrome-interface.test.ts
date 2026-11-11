// Contract tests for the 2026-11-12 interface wave: the chrome-local commands
// that answer typed words before `cad_command` and never reach the router.
//
// Fourteen visibility/window words in the first wave (`RIBBON`…`WSSETTINGS`) and
// three more in the amendment (`QCCLOSE`, `UPDATETHUMBSNOW`, `TOOLBAR`/
// `-TOOLBAR`). The suite pins the selector table, the resolution kinds, the
// catalog/ribbon/i18n surface, the visibility value (a set, never a toggle), the
// Quick View projection, the calculator arithmetic/memory, and the in-memory
// workspace store. No React DOM is rendered: the stores and models are pure.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CHROME_COMMAND_NAMES,
  CHROME_COMMANDS,
  dashToolbarVisibility,
} from '../src/commands/chrome-commands.ts';
import { localSelectorFor } from '../src/commands/local-selectors.ts';
import {
  admitCommandText,
  isLocalChromeCommand,
  isSupportedCommand,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { LOCAL_CHROME_COMMANDS } from '../src/commands/gating.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import {
  DEFAULT_CHROME_VISIBILITY,
  HIDDEN_CHROME_VISIBILITY,
  paletteChrome,
  withFileTabs,
  withPalettes,
  withRibbon,
} from '../src/chrome/visibility.ts';
import { quickViewDrawings, quickViewLayouts } from '../src/chrome/quick-view.ts';
import {
  INITIAL_CALC_STATE,
  appendCalcToken,
  backspaceCalc,
  calcCopyText,
  clearCalc,
  evaluateCalc,
  evaluateExpression,
  formatCalcNumber,
  memoryAddCalc,
  memoryClearCalc,
  memoryRecallCalc,
  setCalcExpression,
  toggleSignCalc,
} from '../src/chrome/quick-calc.ts';
import {
  DRAFTING_WORKSPACE_ID,
  deleteWorkspace,
  renameWorkspace,
  requestApplyWorkspace,
  resetWorkspaces,
  saveWorkspace,
  workspaceName,
  workspacePreset,
  workspaceSnapshot,
} from '../src/chrome/workspaces.ts';
import {
  chromeSurfaceSnapshot,
  closeQuickCalc,
  closeQuickView,
  openQuickCalc,
  openQuickView,
  openWorkspaceSelector,
  openWorkspaceSettings,
  refreshQuickView,
  resetChromeSurfaces,
} from '../src/chrome/surfaces.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

const WAVE = [
  'RIBBON', 'RIBBONCLOSE', 'HIDEPALETTES', 'SHOWPALETTES', 'FILETAB', 'FILETABCLOSE',
  'QVDRAWING', 'QVDRAWINGCLOSE', 'QVLAYOUT', 'QVLAYOUTCLOSE', 'QUICKCALC',
  'WORKSPACE', 'WSSAVE', 'WSSETTINGS',
  'QCCLOSE', 'UPDATETHUMBSNOW', 'TOOLBAR', '-TOOLBAR',
];

const CATALOG_SLUG = {
  '-TOOLBAR': 'draw.kind.dashtoolbar',
};

const slug = id => CATALOG_SLUG[id] ?? `draw.kind.${id.toLowerCase()}`;

const visibility = { ribbon: true, palettes: true, fileTabs: true };

async function iconNames() {
  const map = (await readModule('icons.tsx')).match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
}

test('the selector table is append-only and every row matches exactly one word', () => {
  assert.equal(CHROME_COMMANDS.length, 28);
  assert.deepEqual([...CHROME_COMMAND_NAMES], [
    'ABOUT', 'CLEANSCREENON', 'CLEANSCREENOFF', 'PROPERTIESCLOSE', 'LAYERCLOSE', 'DSETTINGS',
    'QUICKPROPERTIES', 'CLASSICLAYER', 'COMMANDLINE', 'COMMANDLINEHIDE',
    'RIBBON', 'RIBBONCLOSE', 'HIDEPALETTES', 'SHOWPALETTES', 'FILETAB', 'FILETABCLOSE',
    'QVDRAWING', 'QVDRAWINGCLOSE', 'QVLAYOUT', 'QVLAYOUTCLOSE', 'QUICKCALC',
    'WORKSPACE', 'WSSAVE', 'WSSETTINGS',
    'QCCLOSE', 'UPDATETHUMBSNOW', 'TOOLBAR', '-TOOLBAR',
  ]);
  for (const name of WAVE) {
    const row = CHROME_COMMANDS.find(selector => selector.name === name);
    assert.ok(row, `${name} must have a row`);
    assert.equal(row.matches(name.toLowerCase()), true, `${name} is case-insensitive`);
    assert.equal(row.matches(`  ${name}  `), true, `${name} is trimmed`);
    assert.equal(row.matches(`${name} 1`), false, `${name} is the bare word`);
  }
  // `-TOOLBAR` is the one argument-taking row; any other argument is not it.
  assert.equal(dashToolbarVisibility('-TOOLBAR'), true);
  assert.equal(dashToolbarVisibility('-toolbar show'), true);
  assert.equal(dashToolbarVisibility('-TOOLBAR HIDE'), false);
  assert.equal(dashToolbarVisibility('-TOOLBAR TOGGLE'), null);
});

test('every word resolves to a chrome-local result on desktop and mobile', () => {
  const expected = {
    RIBBON: { kind: 'ribbon', value: true },
    RIBBONCLOSE: { kind: 'ribbon', value: false },
    HIDEPALETTES: { kind: 'paletteChrome', value: false },
    SHOWPALETTES: { kind: 'paletteChrome', value: true },
    FILETAB: { kind: 'fileTabs', value: true },
    FILETABCLOSE: { kind: 'fileTabs', value: false },
    QVDRAWING: { kind: 'quickView', panel: 'drawings' },
    QVDRAWINGCLOSE: { kind: 'quickViewClose', panel: 'drawings' },
    QVLAYOUT: { kind: 'quickView', panel: 'layouts' },
    QVLAYOUTCLOSE: { kind: 'quickViewClose', panel: 'layouts' },
    QUICKCALC: { kind: 'quickCalc', value: true },
    QCCLOSE: { kind: 'quickCalc', value: false },
    UPDATETHUMBSNOW: { kind: 'updateThumbs' },
    TOOLBAR: { kind: 'toolbar', value: true },
    WORKSPACE: { kind: 'workspace' },
    WSSAVE: { kind: 'workspaceSettings', mode: 'save' },
    WSSETTINGS: { kind: 'workspaceSettings', mode: 'settings' },
  };
  for (const [word, result] of Object.entries(expected)) {
    assert.deepEqual(localSelectorFor(word, 'desktop'), result, word);
    assert.deepEqual(localSelectorFor(word.toLowerCase(), 'mobile'), result, word);
  }
  assert.deepEqual(localSelectorFor('-TOOLBAR', 'desktop'), { kind: 'toolbar', value: true });
  assert.deepEqual(localSelectorFor('-TOOLBAR SHOW', 'mobile'), { kind: 'toolbar', value: true });
  assert.deepEqual(localSelectorFor('-TOOLBAR HIDE', 'desktop'), { kind: 'toolbar', value: false });
  // A near miss is a router line, not a silent toggle.
  assert.equal(localSelectorFor('-TOOLBAR TOGGLE', 'desktop'), null);
  assert.equal(localSelectorFor('RIBBONS', 'desktop'), null);
});

test('every selector is admitted by the chrome with no document and no vocabulary', async () => {
  // The one interceptor runs before the in-flight guard and before `cad_command`.
  const app = await readModule('App.tsx');
  assert.ok(app.includes('if (runLocalSelector(text, options.fromInput === true)) return;'));
  assert.ok(app.indexOf('runLocalSelector(text') < app.indexOf('submitting.current = true'));
  assert.equal((app.match(/queueCadCommand\(/g) ?? []).length, 1, 'one router lane only');
  for (const id of WAVE) {
    assert.equal(LOCAL_CHROME_COMMANDS.includes(id), true, `${id} is chrome-local`);
    assert.equal(isLocalChromeCommand(id), true, `${id} is case-insensitive`);
    assert.equal(isSupportedCommand(id), true, `${id} is supported`);
    assert.equal(admitCommandText(id, { status: null }).kind, 'admitted', id);
    assert.equal(admitCommandText(id, { status: null, pending: true }).kind, 'admitted', id);
    assert.equal(admitCommandText(id, { status: { documentLabel: null, loading: false } }).kind, 'admitted', id);
    setCommandCatalog([]);
    assert.equal(admitCommandText(id, { status: null }).kind, 'admitted', `${id} does not need the host catalog`);
    setCommandCatalog(HOST_NAMES);
  }
});

test('the append-only catalog exposes each name exactly once with a real glyph and key', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const glyphs = await iconNames();
  for (const id of WAVE) {
    const rows = COMMAND_CATALOG.filter(row => row.id === id);
    assert.equal(rows.length, 1, `${id} must be catalogued once`);
    const [row] = rows;
    assert.equal(row.labelKey, slug(id), `${id} owns its label key`);
    assert.ok(glyphs.has(row.icon), `icons.tsx must define '${row.icon}' for ${id}`);
    for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
      assert.equal(typeof catalog[row.labelKey], 'string', `${locale} missing ${row.labelKey}`);
      assert.equal(typeof catalog[`${row.labelKey}.hint`], 'string', `${locale} missing ${row.labelKey}.hint`);
      assert.equal(/Named refusal/i.test(catalog[`${row.labelKey}.hint`]), false, `${id} is no longer a refusal`);
    }
  }
  // The RIBBON row is the old Wave 8 name, re-declared with the live copy.
  const ribbon = COMMAND_CATALOG.find(row => row.id === 'RIBBON');
  assert.match(en[`${ribbon.labelKey}.hint`], /Chrome-local selector/i);
});

test('the View interface panel dispatches the same chrome-local words', () => {
  const view = RIBBON_TABS.find(tab => tab.id === 'view');
  const panel = view.panels.find(entry => entry.id === 'interface');
  assert.ok(panel, 'the View tab keeps its interface panel');
  assert.equal(panel.titleKey, 'ribbon.panel.interface');
  const ids = panel.items.map(item => item.id);
  for (const id of ['RIBBON', 'TOOLBAR', 'HIDEPALETTES', 'FILETAB', 'QVLAYOUT', 'QUICKCALC', 'WORKSPACE']) {
    assert.ok(ids.includes(id), `${id} has a ribbon entry`);
    const item = panel.items.find(entry => entry.id === id);
    assert.equal(item.command, id, `${id} dispatches its own command token`);
    assert.equal(item.action, undefined, `${id} must not be a shell action`);
    assert.ok(LOCAL_CHROME_COMMANDS.includes(item.command), `${id} is chrome-local`);
    assert.equal(admitCommandText(item.command, { status: null }).kind, 'admitted', id);
  }
});

test('visibility is one value and every word is an explicit set, never a toggle', () => {
  assert.deepEqual(DEFAULT_CHROME_VISIBILITY, visibility);
  assert.deepEqual(HIDDEN_CHROME_VISIBILITY, { ribbon: false, palettes: false, fileTabs: false });
  assert.deepEqual(withRibbon(DEFAULT_CHROME_VISIBILITY, false), { ...visibility, ribbon: false });
  assert.deepEqual(withPalettes(DEFAULT_CHROME_VISIBILITY, false), { ...visibility, palettes: false });
  assert.deepEqual(withFileTabs(DEFAULT_CHROME_VISIBILITY, false), { ...visibility, fileTabs: false });
  assert.deepEqual(paletteChrome(false), HIDDEN_CHROME_VISIBILITY);
  assert.deepEqual(paletteChrome(true), DEFAULT_CHROME_VISIBILITY);

  const show = localSelectorFor('RIBBON', 'desktop');
  const hide = localSelectorFor('RIBBONCLOSE', 'desktop');
  assert.deepEqual(show, { kind: 'ribbon', value: true });
  assert.deepEqual(hide, { kind: 'ribbon', value: false });
  // Repeating either word answers the same value: a set, not a flip.
  assert.deepEqual(localSelectorFor('RIBBON', 'desktop'), show);
  assert.deepEqual(localSelectorFor('RIBBONCLOSE', 'desktop'), hide);
  assert.deepEqual(localSelectorFor('FILETAB', 'desktop'), { kind: 'fileTabs', value: true });
  assert.deepEqual(localSelectorFor('FILETABCLOSE', 'desktop'), { kind: 'fileTabs', value: false });
  assert.deepEqual(localSelectorFor('HIDEPALETTES', 'desktop'), { kind: 'paletteChrome', value: false });
  assert.deepEqual(localSelectorFor('SHOWPALETTES', 'desktop'), { kind: 'paletteChrome', value: true });
});

test('the shell wires the visibility value, the toolbar strip and the palette close', async () => {
  const app = await readModule('App.tsx');
  assert.ok(app.includes('setRibbonVisible: (value: boolean) => setChromeVisibility(current => withRibbon(current, value))'));
  assert.ok(app.includes('setFileTabsVisible: (value: boolean) => setChromeVisibility(current => withFileTabs(current, value))'));
  assert.ok(app.includes('setPaletteChrome: (value: boolean) =>'));
  assert.ok(app.includes('if (!value) closeAllPalettes();'));
  assert.ok(app.includes("if (!preset.visibility.palettes) closeAllPalettes();"));
  assert.ok(app.includes('{mobile || cleanScreen || !chromeVisibility.ribbon ? null : <Ribbon />}'));
  assert.ok(app.includes('{mobile || cleanScreen || !chromeVisibility.fileTabs ? null : <DocTabs />}'));

  const titlebar = await readModule('components/TitleBar.tsx');
  assert.ok(titlebar.includes('{shell.toolbarVisible ? ('), 'the QAT is the toolbar strip TOOLBAR sets');
  const workspace = await readModule('components/Workspace.tsx');
  assert.ok(workspace.includes('palettesRequested'), 'an explicit palette open still paints');
});

test('Quick View Drawings is the one open drawing, or an honest empty state', () => {
  const cards = quickViewDrawings('floor-plan.dwg', true);
  assert.equal(cards.length, 1);
  assert.deepEqual({ ...cards[0] }, { id: 'current', label: 'floor-plan.dwg', unnamed: false, active: true });
  const unnamed = quickViewDrawings(null, true);
  assert.equal(unnamed.length, 1);
  assert.equal(unnamed[0].unnamed, true);
  assert.equal(quickViewDrawings(null, false).length, 0);
});

test('Quick View Layouts lists the real layout rows and their own switch tokens', () => {
  const status = {
    layouts: [
      { id: 3, name: 'Sheet A', supported: true, viewportCount: 2 },
      { id: 4, name: 'Sheet B', supported: false, reason: 'viewport.twisted: bad' },
    ],
    activeSpace: { kind: 'model' },
  };
  const cards = quickViewLayouts(status);
  assert.deepEqual(cards.map(card => card.key), ['model', 'layout-3', 'layout-4']);
  assert.equal(cards[0].active, true);
  assert.equal(cards[0].command, null);
  assert.deepEqual(
    { key: cards[1].key, name: cards[1].name, active: cards[1].active, command: cards[1].command, viewports: cards[1].viewportCount },
    { key: 'layout-3', name: 'Sheet A', active: false, command: 'SPACE #3', viewports: 2 },
  );
  assert.equal(cards[2].supported, false);
  assert.equal(cards[2].command, null);
  // Paper active: the Model card is the one that switches back.
  const paper = quickViewLayouts({ ...status, activeSpace: { kind: 'paper', id: 3 } });
  assert.equal(paper[0].command, 'MODEL');
  assert.equal(paper[1].command, null);
});

test('the surfaces store opens, refreshes and closes idempotently', () => {
  resetChromeSurfaces();
  assert.equal(chromeSurfaceSnapshot().quickView, null);
  closeQuickView();
  closeQuickView();
  assert.equal(chromeSurfaceSnapshot().quickView, null, 'a repeat close is a success');

  // The close word names its own panel: closing the other one is a no-op.
  openQuickView('layouts');
  closeQuickView('drawings');
  assert.equal(chromeSurfaceSnapshot().quickView, 'layouts', 'QVDRAWINGCLOSE leaves Layouts open');
  closeQuickView('layouts');
  assert.equal(chromeSurfaceSnapshot().quickView, null);

  openQuickView('layouts');
  refreshQuickView();
  assert.equal(chromeSurfaceSnapshot().quickView, 'layouts', 'a refresh keeps the open panel');
  assert.equal(chromeSurfaceSnapshot().quickViewRevision, 1, 'refresh bumps the revision');

  resetChromeSurfaces();
  refreshQuickView();
  assert.equal(chromeSurfaceSnapshot().quickView, 'drawings', 'with no panel open the refresh opens Drawings');
  assert.equal(chromeSurfaceSnapshot().quickViewRevision, 1);

  openWorkspaceSelector();
  openQuickCalc();
  assert.equal(chromeSurfaceSnapshot().workspaceSelector, false, 'a modal closes the floating panel');
  assert.equal(chromeSurfaceSnapshot().quickCalc, true);
  closeQuickCalc();
  closeQuickCalc();
  assert.equal(chromeSurfaceSnapshot().quickCalc, false, 'QCCLOSE is idempotent');

  openWorkspaceSettings('save');
  assert.equal(chromeSurfaceSnapshot().workspaceSettings, 'save');
  openWorkspaceSettings('settings');
  assert.equal(chromeSurfaceSnapshot().workspaceSettings, 'settings');
  resetChromeSurfaces();
});

test('the calculator evaluates precedence, parentheses and division honestly', () => {
  assert.equal(evaluateExpression('2+3*4'), 14);
  assert.equal(evaluateExpression('(2+3)*4'), 20);
  assert.equal(evaluateExpression('-3*2'), -6);
  assert.equal(evaluateExpression('10/4'), 2.5);
  assert.equal(evaluateExpression('2*(3+(4-1))'), 12);
  assert.equal(evaluateExpression('1/0'), null, 'division by zero never fabricates a number');
  assert.equal(evaluateExpression('2+'), null);
  assert.equal(evaluateExpression('()'), null);
  assert.equal(evaluateExpression(''), null);
  assert.equal(formatCalcNumber(0.1 + 0.2), '0.3');
  assert.equal(formatCalcNumber(-0), '0');
});

test('the calculator keys, clear, backspace, sign and memory are one state machine', () => {
  let state = INITIAL_CALC_STATE;
  for (const token of ['2', '+', '3', '*', '4']) state = appendCalcToken(state, token);
  state = evaluateCalc(state);
  assert.equal(state.value, '14');
  assert.equal(state.expression, '');
  assert.equal(state.justEvaluated, true);
  // A digit after `=` starts a new expression; an operator continues from 14.
  assert.equal(appendCalcToken(state, '7').expression, '7');
  assert.equal(appendCalcToken(state, '+').expression, '14+');

  let typed = setCalcExpression(INITIAL_CALC_STATE, '12+3');
  typed = backspaceCalc(typed);
  assert.equal(typed.expression, '12+');
  typed = clearCalc(typed);
  assert.equal(typed.expression, '');
  assert.equal(typed.value, '');

  let signed = appendCalcToken(INITIAL_CALC_STATE, '5');
  signed = toggleSignCalc(signed);
  assert.equal(signed.expression, '(-5)');
  signed = toggleSignCalc(signed);
  assert.equal(signed.expression, '5');

  let memory = appendCalcToken(INITIAL_CALC_STATE, '2');
  memory = appendCalcToken(memory, '+');
  memory = appendCalcToken(memory, '3');
  memory = memoryAddCalc(memory);
  assert.equal(memory.memory, 5);
  memory = clearCalc(memory);
  memory = memoryRecallCalc(memory);
  assert.equal(memory.expression, '5');
  memory = memoryClearCalc(memory);
  assert.equal(memory.memory, 0);
  assert.equal(calcCopyText({ ...INITIAL_CALC_STATE, value: '14' }), '14');
  // Unknown tokens are ignored, never inserted.
  assert.equal(appendCalcToken(INITIAL_CALC_STATE, 'pow').expression, '');
});

test('workspaces save, rename, delete and apply in memory only', () => {
  resetWorkspaces();
  const initial = workspaceSnapshot();
  assert.equal(initial.currentId, DRAFTING_WORKSPACE_ID);
  assert.equal(initial.presets.length, 1);
  const builtin = workspacePreset(DRAFTING_WORKSPACE_ID);
  assert.equal(builtin.builtin, true);
  assert.equal(workspaceName(builtin, key => key), 'workspace.drafting_annotation');

  const saved = saveWorkspace('  Plot   Sheet  ', { ...visibility, ribbon: false }, 999);
  assert.equal(saved.ok, true);
  const preset = workspacePreset(saved.id);
  assert.equal(preset.name, 'Plot Sheet', 'internal whitespace collapses');
  assert.equal(preset.panelWidth, 460, 'the width is clamped to the palette ceiling');
  assert.equal(workspaceSnapshot().currentId, saved.id, 'saving makes it current');
  assert.equal(workspaceSnapshot().applyRevision, 0, 'saving does not repaint the running chrome');

  assert.deepEqual(saveWorkspace('plot sheet', visibility, 300), { ok: false, reason: 'duplicate' });
  assert.deepEqual(saveWorkspace('   ', visibility, 300), { ok: false, reason: 'empty' });
  assert.deepEqual(renameWorkspace(DRAFTING_WORKSPACE_ID, 'Nope'), { ok: false, reason: 'builtin' });
  assert.deepEqual(renameWorkspace(saved.id, 'Plot Sheet'), { ok: true });
  assert.equal(workspacePreset(saved.id).name, 'Plot Sheet');
  assert.deepEqual(renameWorkspace('workspace-404', 'X'), { ok: false, reason: 'unknown' });

  assert.equal(requestApplyWorkspace(DRAFTING_WORKSPACE_ID), true);
  assert.equal(workspaceSnapshot().applyRevision, 1);
  assert.equal(workspaceSnapshot().currentId, DRAFTING_WORKSPACE_ID);
  assert.equal(requestApplyWorkspace('workspace-404'), false);

  assert.equal(deleteWorkspace(DRAFTING_WORKSPACE_ID), false, 'the built-in cannot be deleted');
  assert.equal(deleteWorkspace(saved.id), true);
  assert.equal(workspacePreset(saved.id), null);
  assert.equal(workspaceSnapshot().presets.length, 1);
  resetWorkspaces();
});
