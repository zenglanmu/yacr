import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  commandCatalog,
  defaultSuggestionIndex,
  filterCommandSuggestions,
  historyText,
  isLocalChromeCommand,
  isSupportedCommand,
  normalizeCommandText,
  pushHistory,
  setCommandCatalog,
  splitCommandText,
  stepHistoryCursor,
  toolCommandForKey,
  toolLabelKey,
  unavailableReasonKey,
} from '../src/commands/commands.ts';
import {
  ARGUMENT_COMMANDS,
  CAPABILITY_GATES,
  GESTURE_ONLY_COMMANDS,
  LOCAL_CHROME_COMMANDS,
} from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS, ribbonCommandIds, ribbonTab } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

// The frozen `cad_status` capability shape: the three new booleans
// (`canNew`/`canSave`/`canPlot`) always travel with `canOpen`/`canFit`.
const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };
const empty = { documentLabel: null, entityCount: null, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: false, canPlot: false, canFit: false };

test('admission is the loaded host catalog: empty before load, capability gates stay explicit', () => {
  // Before the host answers, nothing is supported (explicitly, not by default).
  setCommandCatalog([]);
  assert.deepEqual(commandCatalog(), []);
  assert.equal(isSupportedCommand('OPEN'), false);
  assert.equal(isSupportedCommand('LINE'), false);
  assert.equal(admitCommandText('OPEN', { status: loaded }).kind, 'unsupported');

  setCommandCatalog(HOST_NAMES);
  assert.equal(isSupportedCommand('open'), true);
  assert.equal(isSupportedCommand(' fit '), true);
  assert.equal(isSupportedCommand('LINE'), true);
  assert.equal(isSupportedCommand('MOVE'), true);
  assert.equal(isSupportedCommand('UNDO'), true);
  assert.equal(isSupportedCommand('CONFIRM'), true);
  // The object-snap toggle and grid snap are both part of the host vocabulary and
  // are **different** selectors: `OSNAP` is object snap (F3), `SNAP` is grid snap
  // (F9) with the typed `SNAPUNIT` spacing sysvar.
  assert.equal(isSupportedCommand('OSNAP'), true);
  assert.equal(isSupportedCommand('snap'), true);
  assert.equal(isSupportedCommand('SNAPUNIT'), true);
  // The file-level selectors are real router commands. They are **no longer**
  // excluded for being "host-owned": the capability booleans gate them.
  for (const name of Object.keys(CAPABILITY_GATES)) assert.equal(isSupportedCommand(name), true, name);
  assert.deepEqual(
    { ...CAPABILITY_GATES },
    { NEW: 'canNew', SAVE: 'canSave', SAVEAS: 'canSave', QSAVE: 'canSave', PLOT: 'canPlot', EXPORT: 'canPlot', PRINT: 'canPlot' },
  );
  // Unknown names are never admitted.
  assert.equal(isSupportedCommand('NOT_A_COMMAND'), false);
  assert.equal(isSupportedCommand(''), false);
  assert.equal(normalizeCommandText('  fit '), 'FIT');
  // The setter normalizes and dedupes; then restore the canonical list.
  assert.deepEqual(setCommandCatalog(['line', 'LINE', ' circle ', '']), ['LINE', 'CIRCLE']);
  setCommandCatalog(HOST_NAMES);
});

test('the grid-snap affordance is a real admitted toggle next to GRID and ORTHO', async () => {
  const { readModule } = await import('./helpers.ts');
  // The View tab's display panel keeps GRID/SNAP/ORTHO on screen. All three are
  // real router commands now: GRID is the display grid (F7), SNAP is grid snap
  // (F9, with the typed `SNAPUNIT` spacing) and ORTHO is the orthographic
  // constraint (F8).
  const view = RIBBON_TABS.find(tab => tab.id === 'view');
  const display = view.panels.find(panel => panel.id === 'display');
  assert.ok(display, 'the View tab must keep its display panel');
  assert.deepEqual(display.items.map(item => item.id), ['GRID', 'SNAP', 'ORTHO']);
  const snap = display.items.find(item => item.id === 'SNAP');
  assert.equal(snap.command, 'SNAP');
  assert.equal(snap.labelKey, 'status.toggle.snap');
  // Router admission is the enable mechanism: all four drafting switches are
  // admitted, and SNAP is a different selector from OSNAP/GRID/ORTHO.
  assert.equal(admitCommandText('SNAP', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('GRID', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('OSNAP', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('ORTHO', { status: loaded }).kind, 'admitted');
  // A typed spacing line is admitted by its base selector; a bad value is refused
  // locally (the host validates it too, and never clamps).
  assert.equal(admitCommandText('SNAPUNIT 5', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('SNAPUNIT 5', { status: loaded }).command, 'SNAPUNIT 5');
  assert.equal(admitCommandText('SNAPUNIT 0', { status: loaded }).kind, 'unsupported');
  // Autocomplete keeps the same split: `SNAP` and `OSNAP` are both supported and
  // are distinct rows.
  const typedSnap = filterCommandSuggestions('SNAP');
  assert.equal(typedSnap.find(suggestion => suggestion.id === 'SNAP').supported, true);
  assert.equal(typedSnap.find(suggestion => suggestion.id === 'OSNAP').supported, true);
  // The ribbon renderer enables every command item the router admits (the same
  // gate GRID/ORTHO use).
  // The gate is per dispatch token: a command takes router admission, a typed
  // standard view takes the CAD-session gate, and nothing is locally enabled.
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(
    ribbon.includes(': item.command ? shell.canCommand(item.command)'),
    'a command ribbon item must be enabled only through router admission (`canCommand`)',
  );
  assert.ok(
    ribbon.includes('item.standardView ? standardViewAvailable(shell)'),
    'a standard-view item must be enabled only through the typed-IPC session gate',
  );
  assert.ok(ribbon.includes('disabled hint={unavailable(t, item, shell)}'), 'an unadmitted item renders disabled with the unsupported hint');
});

// Chrome catalog entries the router does not accept: they stay **visible but
// permanently disabled** (never deleted, never locally enabled). This is the
// audited dead set; a future edit that adds or restores an affordance must move
// its id out of this list.
//
// **Empty as of the `SNAP`/`SNAPUNIT` work**: every remaining drafting aid got a
// real selector.
//  - SNAP left this set: `SNAP` is a real router selector now (grid snap, F9,
//    `CommandId::ToggleSnapGrid`), with the typed `SNAPUNIT` spacing sysvar. It is
//    still a different feature from object snap (`OSNAP`) and from the
//    display-grid toggle (`GRID`), and it must never alias either.
//  - GRID left this set earlier: `GRID` is a real router selector (the display-grid
//    session switch, F7), so it is admitted like `OSNAP`.
//  - ORTHO left this set earlier: `ORTHO` is a real router selector (the orthographic
//    drafting constraint, F8), so it is admitted like `GRID`/`OSNAP`.
// The four measurement kinds (DIST/MEASUREPOLYLINE/MEASUREANGLE/AREA) and the
// fixed zoom steps (ZOOMIN/ZOOMOUT) were fixed: they are router-admitted
// selectors now and moved into HOST_NAMES. `PLINE` is now the polyline **draw**
// selector (a router-admitted `COMMAND_TABLE` entry → `BeginDraw(Polyline)`), so
// it is a real affordance, not reserved. `ANGLE` (not an AutoCAD command) is
// deliberately **not** in this set: it is an unbound name, not a dead
// affordance. `ORBIT` is **gesture-only** (router-admitted, click-disabled
// with an explicit reason) and is asserted separately via GESTURE_ONLY_COMMANDS,
// not lumped into this router-refused set. `PROPERTIES` and `LAYER` are
// **chrome-local**: the command line and ribbon open the docked inspector and the
// docked layers palette without the router (`LAYER` is not a router name at all —
// the layer *operations* are), so they are asserted separately below rather than
// counted as dead affordances.
const ROUTER_REFUSED_AFFORDANCES = [];

test('the command catalog lists exactly the router-refused dead affordances, and ERASE is not one', () => {
  const refused = COMMAND_CATALOG
    .filter(command => !GESTURE_ONLY_COMMANDS.includes(command.id)
      && !isSupportedCommand(command.id))
    .map(command => command.id)
    .sort();
  assert.deepEqual(refused, [...ROUTER_REFUSED_AFFORDANCES].sort());
  for (const id of ROUTER_REFUSED_AFFORDANCES) {
    assert.equal(isSupportedCommand(id), false, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'unsupported', id);
  }
  assert.equal(ROUTER_REFUSED_AFFORDANCES.includes('ERASE'), false, 'ERASE must be reachable');
  // `PROPERTIES` left this set: the chrome answers it locally (docked palette).
  assert.equal(ROUTER_REFUSED_AFFORDANCES.includes('PROPERTIES'), false, 'PROPERTIES is chrome-local');
  assert.equal(isSupportedCommand('PROPERTIES'), true, 'PROPERTIES is admitted by the chrome');
  assert.equal(admitCommandText('PROPERTIES', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('PROPERTIES', { status: null, pending: true }).kind, 'admitted');
  // `LAYER` left this set too: it opens the docked layers palette through the
  // chrome's own selector path, with no router command and no document needed.
  assert.equal(ROUTER_REFUSED_AFFORDANCES.includes('LAYER'), false, 'LAYER is chrome-local');
  assert.equal(isSupportedCommand('LAYER'), true, 'LAYER is admitted by the chrome');
  assert.equal(admitCommandText('LAYER', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('LAYER', { status: null, pending: true }).kind, 'admitted');
  assert.equal(isLocalChromeCommand('layer'), true, 'the selector is case-insensitive');
  assert.deepEqual([...LOCAL_CHROME_COMMANDS], ['PROPERTIES', 'LAYER']);
  // `PLINE` is now a real (draw) selector, so it is neither reserved nor dead:
  // `ANGLE` (not an AutoCAD command) stays absent from the catalog and refused.
  assert.equal(ROUTER_REFUSED_AFFORDANCES.includes('ANGLE'), false, 'ANGLE');
  assert.equal(COMMAND_CATALOG.some(command => command.id === 'ANGLE'), false, 'ANGLE');
  assert.equal(isSupportedCommand('ANGLE'), false, 'ANGLE');
  assert.equal(isSupportedCommand('PLINE'), true, 'PLINE is a router-admitted draw selector');
  assert.equal(COMMAND_CATALOG.some(command => command.id === 'PLINE'), true, 'PLINE is catalogued');
  // The fixed affordances are no longer dead.
  for (const id of ['DIST', 'MEASUREPOLYLINE', 'MEASUREANGLE', 'AREA', 'ZOOMIN', 'ZOOMOUT']) {
    assert.equal(ROUTER_REFUSED_AFFORDANCES.includes(id), false, id);
    assert.equal(isSupportedCommand(id), true, id);
  }
});

test('the five measurement buttons dispatch their own admitted kind selectors', async () => {
  const { readModule } = await import('./helpers.ts');
  const measure = RIBBON_TABS.find(tab => tab.id === 'measure');
  const panel = measure.panels.find(candidate => candidate.id === 'measure');
  assert.ok(panel, 'the Measure tab keeps its measurement panel');
  // Each button dispatches its own selector; none collapses onto
  // `MEASURE`/Distance. The internal ids keep the familiar words, but the
  // **dispatched selectors** are explicit: `PLINE` stays reserved for the P3
  // polyline draw command and `ANGLE` is deliberately unbound. ID (2026-10-26) is
  // the point-coordinate readout, appended as the fifth kind so no shipped button
  // moved.
  const dispatched = panel.items.map(item => item.command);
  assert.deepEqual(dispatched, ['DIST', 'MEASUREPOLYLINE', 'MEASUREANGLE', 'AREA', 'ID']);
  const idToSelector = Object.fromEntries(panel.items.map(item => [item.id, item.command]));
  assert.deepEqual(idToSelector, {
    DIST: 'DIST',
    PLINE: 'MEASUREPOLYLINE',
    ANGLE: 'MEASUREANGLE',
    AREA: 'AREA',
    ID: 'ID',
  });
  for (const id of dispatched) {
    assert.equal(isSupportedCommand(id), true, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
  }
  // The old words are not selectors at all (neither shadows the measurement),
  // except `PLINE`, which is now the polyline **draw** selector: it is admitted
  // but never a measurement selector (it must not collapse onto MEASUREPOLYLINE).
  assert.equal(isSupportedCommand('PLINE'), true, 'PLINE is the draw selector');
  assert.equal(admitCommandText('PLINE', { status: loaded }).kind, 'admitted');
  assert.equal(dispatched.includes('PLINE'), false, 'PLINE is not a measurement selector');
  assert.equal(isSupportedCommand('ANGLE'), false, 'ANGLE');
  assert.equal(admitCommandText('ANGLE', { status: loaded }).kind, 'unsupported', 'ANGLE');
  // Distinct labels, so five different measurement kinds are presented.
  assert.equal(new Set(panel.items.map(item => item.labelKey)).size, 5);
  // The renderer dispatches each selector through the single command funnel.
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
});

test('PAN is an admitted router command, not a local action', () => {
  assert.equal(isSupportedCommand('PAN'), true);
  assert.equal(admitCommandText('PAN', { status: loaded }).kind, 'admitted');
  for (const tabId of ['home', 'view']) {
    const navigate = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'navigate');
    const pan = navigate.items.find(item => item.id === 'PAN');
    assert.ok(pan, `${tabId} keeps its PAN affordance`);
    assert.equal(pan.command, 'PAN');
    assert.equal(pan.action, undefined, 'PAN must dispatch through the router');
  }
});

test('ZOOMIN and ZOOMOUT dispatch admitted fixed zoom steps', () => {
  for (const id of ['ZOOMIN', 'ZOOMOUT']) {
    assert.equal(isSupportedCommand(id), true, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
  }
  const zoomIn = COMMAND_CATALOG.find(command => command.id === 'ZOOMIN');
  const zoomOut = COMMAND_CATALOG.find(command => command.id === 'ZOOMOUT');
  assert.equal(zoomIn.labelKey, 'toolbar.zoom_in');
  assert.equal(zoomOut.labelKey, 'toolbar.zoom_out');
  // The View tab exposes both as router commands.
  const navigate = RIBBON_TABS.find(tab => tab.id === 'view').panels.find(panel => panel.id === 'navigate');
  for (const id of ['ZOOMIN', 'ZOOMOUT']) {
    assert.equal(navigate.items.find(item => item.id === id).command, id);
  }
});

// `PLOT` and `RESETVIEW` are host-implemented router selectors that only lacked
// a chrome entry: `PLOT` is the SVG/PDF output command (the `EXPORT`/`PRINT`
// aliases normalise onto it) and `RESETVIEW` restores the drawing's saved view.
// This test locks the three surfaces they now share with every other entry —
// the display catalog, the loaded vocabulary that actually enables them, and the
// ribbon that offers them — so a display row cannot drift from the host.
test('PLOT and RESETVIEW are catalogued and offered by their tabs', async () => {
  const { readCatalog } = await import('./helpers.ts');
  const plot = COMMAND_CATALOG.find(command => command.id === 'PLOT');
  const reset = COMMAND_CATALOG.find(command => command.id === 'RESETVIEW');
  assert.ok(plot, 'PLOT must be a catalogued display entry');
  assert.ok(reset, 'RESETVIEW must be a catalogued display entry');
  // Catalog ordering: PLOT sits in the file group next to SAVE, RESETVIEW in the
  // view group with the other navigation selectors.
  assert.deepEqual({ ...plot }, { id: 'PLOT', labelKey: 'file.plot', icon: 'save', group: 'file' });
  assert.deepEqual({ ...reset }, { id: 'RESETVIEW', labelKey: 'view.reset', icon: 'view', group: 'view' });
  // `commandCatalog()` is the **host** vocabulary (`cad_commands`), which is what
  // enables a row: both names must be on it, and admission follows from that.
  for (const id of ['PLOT', 'RESETVIEW']) {
    assert.equal(commandCatalog().includes(id), true, `${id} is in the loaded host vocabulary`);
    assert.equal(isSupportedCommand(id), true, id);
    assert.equal(isSupportedCommand(id.toLowerCase()), true, `${id} is case-insensitive`);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
    assert.equal(admitCommandText(id.toLowerCase(), { status: loaded }).kind, 'admitted', id);
  }
  // PLOT keeps the same capability gate as its aliases (`canPlot`): with no
  // drawing open the honest sentence is "open a drawing first", never
  // "unsupported". RESETVIEW has no gate, so the same status admits it.
  assert.equal(CAPABILITY_GATES.PLOT, 'canPlot');
  assert.deepEqual(admitCommandText('PLOT', { status: empty }), {
    kind: 'unavailable',
    command: 'PLOT',
    reasonKey: 'plot.no_document',
  });
  assert.equal(CAPABILITY_GATES.RESETVIEW, undefined, 'RESETVIEW has no capability gate');
  assert.equal(unavailableReasonKey('RESETVIEW', { status: loaded }), 'command.unsupported');
  // The ribbon actually offers both selectors (a catalog row nothing renders is
  // not an affordance), each exactly once in its own tab.
  const ids = ribbonCommandIds();
  assert.equal(ids.includes('PLOT'), true, 'PLOT is offered by the File tab');
  assert.equal(ids.includes('RESETVIEW'), true, 'RESETVIEW is offered by the View tab');
  const tabItems = tabId => RIBBON_TABS.find(tab => tab.id === tabId).panels.flatMap(panel => panel.items);
  assert.deepEqual(tabItems('file').filter(item => item.id === 'PLOT').map(item => item.command), ['PLOT']);
  assert.deepEqual(tabItems('view').filter(item => item.id === 'RESETVIEW').map(item => item.command), ['RESETVIEW']);
  // Both are router tokens, never a local action: the renderer's `canCommand`
  // admission stays the only switch (no second, local enable path).
  const resetItem = tabItems('view').find(item => item.id === 'RESETVIEW');
  assert.equal(resetItem.action, undefined, 'RESETVIEW must dispatch through the router');
  // Both label keys resolve in both catalogs — no new copy is invented here.
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const [id, key] of [['PLOT', 'file.plot'], ['RESETVIEW', 'view.reset']]) {
    assert.equal(typeof zh[key], 'string', `${id}: ${key} (zh-CN)`);
    assert.equal(typeof en[key], 'string', `${id}: ${key} (en)`);
  }
});

test('ORBIT stays visible but gesture-disabled with an explicit orbit reason', async () => {
  const { readModule, readCatalog } = await import('./helpers.ts');
  // The router knows ORBIT, but a click cannot supply yaw/pitch; the chrome
  // disables it with a gesture hint (drag in a 3D view) instead of fabricating
  // an orbit. This is an explicit disable, never a local enable.
  assert.equal(isSupportedCommand('ORBIT'), false);
  assert.equal(admitCommandText('ORBIT', { status: loaded }).kind, 'unsupported');
  assert.deepEqual([...GESTURE_ONLY_COMMANDS], ['ORBIT']);
  assert.equal(unavailableReasonKey('ORBIT'), 'view.orbit.gesture');
  assert.equal(unavailableReasonKey('GRID'), 'command.unsupported');
  // It stays in the ribbon (visible, never silently deleted) as a command.
  const navigate = RIBBON_TABS.find(tab => tab.id === 'home').panels.find(panel => panel.id === 'navigate');
  const orbit = navigate.items.find(item => item.id === 'ORBIT');
  assert.ok(orbit, 'ORBIT must stay visible');
  assert.equal(orbit.command, 'ORBIT');
  assert.equal(orbit.labelKey, 'view.orbit_label');
  // The ribbon renders the explicit reason, and the catalog defines it.
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('unavailableReasonKey(item.command, { status: shell.status, pending: shell.pending })'), 'the ribbon must use the explicit reason key');
  const en = await readCatalog('en');
  const zh = await readCatalog('zh-CN');
  assert.equal(typeof en['view.orbit.gesture'], 'string');
  assert.equal(typeof zh['view.orbit.gesture'], 'string');
});

test('autocomplete prefers prefix matches, then usage, then alphabetical', () => {
  const all = filterCommandSuggestions('');
  assert.ok(all.length > 0);
  const prefix = filterCommandSuggestions('co');
  assert.equal(prefix[0].id, 'COPY');
  // Mid-string search still finds commands that do not start with the needle.
  assert.ok(filterCommandSuggestions('ine').some(suggestion => suggestion.id === 'LINE'));
  const used = filterCommandSuggestions('c', { query: 'c', history: ['CIRCLE', 'CIRCLE'] });
  assert.equal(used[0].id, 'CIRCLE');
  const limited = filterCommandSuggestions('', { limit: 3 });
  assert.equal(limited.length, 3);
  assert.equal('score' in limited[0], false);
  assert.equal('usage' in limited[0], false);
});

test('autocomplete marks supported commands and never injects unknown ids', () => {
  const line = filterCommandSuggestions('LINE');
  assert.equal(line[0].id, 'LINE');
  assert.equal(line.find(suggestion => suggestion.id === 'LINE').supported, true);
  // The polyline/angle measurement selectors are explicit `MEASURE*` names, so
  // they neither shadow the planned P3 `PLINE` draw command nor a vague `ANGLE`.
  const measure = filterCommandSuggestions('MEASURE');
  assert.equal(measure.find(suggestion => suggestion.id === 'MEASUREPOLYLINE').supported, true);
  assert.equal(measure.find(suggestion => suggestion.id === 'MEASUREANGLE').supported, true);
  // `PLINE` is now the polyline **draw** command: it is a real suggestion
  // (catalogued + router-admitted), and it never shadows the explicit
  // `MEASUREPOLYLINE` measurement selector. `ANGLE` (not an AutoCAD command)
  // stays unbound, so it is not a suggestion at all.
  const pline = filterCommandSuggestions('PLINE');
  assert.equal(pline.some(suggestion => suggestion.id === 'PLINE'), true);
  assert.equal(pline.find(suggestion => suggestion.id === 'PLINE').supported, true);
  assert.equal(pline.some(suggestion => suggestion.id === 'MEASUREPOLYLINE'), false);
  assert.equal(filterCommandSuggestions('ANGLE').some(suggestion => suggestion.id === 'ANGLE'), false);
  // A chrome-local palette selector (`LAYER`) is offered as supported even though
  // the router has no such command: the chrome answers the word itself.
  const layer = filterCommandSuggestions('LAYER');
  assert.equal(layer.find(suggestion => suggestion.id === 'LAYER').supported, true);
  const open = filterCommandSuggestions('open');
  assert.deepEqual(open.map(suggestion => suggestion.id), ['OPEN']);
  assert.equal(open[0].supported, true);
  assert.deepEqual(filterCommandSuggestions('zzz'), []);
  // Before the host answers, every suggestion is unsupported — never "allowed".
  setCommandCatalog([]);
  assert.ok(filterCommandSuggestions('LINE').every(suggestion => suggestion.supported === false));
  setCommandCatalog(HOST_NAMES);
});

test('default suggestion index highlights an exact match, else the first row', () => {
  const suggestions = [{ id: 'FIT' }, { id: 'FITNESS' }];
  assert.equal(defaultSuggestionIndex(suggestions, 'FITNESS'), 1);
  assert.equal(defaultSuggestionIndex(suggestions, 'FIT'), 0);
  assert.equal(defaultSuggestionIndex(suggestions, ''), 0);
  assert.equal(defaultSuggestionIndex([], ''), -1);
});

test('history cursor walks older entries then returns to the draft', () => {
  const history = ['OPEN', 'FIT', 'LINE'];
  let index = null;
  index = stepHistoryCursor(history, index, -1);
  assert.equal(index, 2);
  assert.equal(historyText(history, index, ''), 'LINE');
  index = stepHistoryCursor(history, index, -1);
  assert.equal(index, 1);
  index = stepHistoryCursor(history, index, 1);
  assert.equal(index, 2);
  index = stepHistoryCursor(history, index, 1);
  assert.equal(index, null);
  assert.equal(historyText(history, index, 'draft'), 'draft');
  assert.equal(stepHistoryCursor([], null, -1), null);
  index = stepHistoryCursor(history, 0, -1);
  assert.equal(index, 0);
});

test('command history is bounded and drops adjacent duplicates', () => {
  assert.deepEqual(pushHistory([], 'OPEN'), ['OPEN']);
  assert.deepEqual(pushHistory(['OPEN'], 'OPEN'), ['OPEN']);
  assert.deepEqual(pushHistory(['OPEN'], '  '), ['OPEN']);
  let history = [];
  for (const id of ['A', 'B', 'C', 'D']) history = pushHistory(history, id, 2);
  assert.deepEqual(history, ['C', 'D']);
});

test('admission mirrors cad_command: unsupported never reaches IPC, capability gates apply', () => {
  assert.equal(admitCommandText('', { status: loaded }).kind, 'empty');
  // Interactive tools the router accepts are admitted; unknown names are not.
  assert.equal(admitCommandText('LINE', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('CONFIRM', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('NOT_A_COMMAND', { status: loaded }).kind, 'unsupported');
  assert.equal(admitCommandText('OPEN', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('open', { pending: true, status: loaded }).kind, 'unavailable');
  assert.equal(admitCommandText('OPEN', { status: { ...loaded, loading: true } }).kind, 'unavailable');
  assert.equal(admitCommandText('OPEN', { status: { ...loaded, canOpen: false } }).kind, 'unavailable');
  assert.equal(admitCommandText('FIT', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('FIT', { status: empty }).kind, 'unavailable');
  assert.equal(admitCommandText('FIT', { status: null }).kind, 'unavailable');
});

test('Enter and Escape drive the active tool, but never while typing or without a tool', () => {
  assert.equal(toolCommandForKey('Enter', { toolActive: true }), 'CONFIRM');
  assert.equal(toolCommandForKey('Escape', { toolActive: true }), 'CANCEL');
  // A field owns its keys (the command line handles its own Enter).
  assert.equal(toolCommandForKey('Enter', { toolActive: true, typing: true }), null);
  assert.equal(toolCommandForKey('Escape', { toolActive: true, typing: true }), null);
  // IME composition and auto-repeat are not confirm/cancel.
  assert.equal(toolCommandForKey('Enter', { toolActive: true, composing: true }), null);
  assert.equal(toolCommandForKey('Enter', { toolActive: true, repeat: true }), null);
  // No active tool: Escape must not send CANCEL (it keeps closing panels).
  assert.equal(toolCommandForKey('Escape', { toolActive: false }), null);
  assert.equal(toolCommandForKey('Enter', { toolActive: false }), null);
  // Unrelated keys are never intercepted.
  assert.equal(toolCommandForKey('F2', { toolActive: true }), null);
  assert.equal(toolCommandForKey('a', { toolActive: true }), null);
});

test('an armed grip drag makes Escape cancel it and enables no other tool-only key', () => {
  // A grip drag is **not** a tool: `cad_status.tool` stays `null` while it is
  // armed, so the drag flag is the only thing that can route Escape to the host.
  assert.equal(toolCommandForKey('Escape', { gripDragActive: true }), 'CANCEL');
  assert.equal(toolCommandForKey('Escape', { toolActive: false, gripDragActive: true }), 'CANCEL');
  // Enter must stay tool-only: confirming a capture that does not exist would
  // dispatch a `CONFIRM` the host could only refuse.
  assert.equal(toolCommandForKey('Enter', { gripDragActive: true }), null);
  assert.equal(toolCommandForKey('Enter', { toolActive: false, gripDragActive: true }), null);
  // A drag does not widen any other key either.
  for (const key of ['F2', 'F3', 'F7', 'F8', 'F9', 'a', ' ', 'ArrowDown']) {
    assert.equal(toolCommandForKey(key, { gripDragActive: true }), null, key);
  }
  // The existing gates still apply to the drag case: a field, an IME
  // composition and an auto-repeat own their keys.
  assert.equal(toolCommandForKey('Escape', { gripDragActive: true, typing: true }), null);
  assert.equal(toolCommandForKey('Escape', { gripDragActive: true, composing: true }), null);
  assert.equal(toolCommandForKey('Escape', { gripDragActive: true, repeat: true }), null);
  // Idle (no tool, no drag) keeps Escape's panel-closing behaviour: no CANCEL.
  assert.equal(toolCommandForKey('Escape', { toolActive: false, gripDragActive: false }), null);
  // A tool and a drag together are still one CANCEL, never two commands.
  assert.equal(toolCommandForKey('Escape', { toolActive: true, gripDragActive: true }), 'CANCEL');
});

test('a host tool selector maps to its catalog key', () => {
  assert.equal(toolLabelKey('draw.line'), 'tool.draw.line');
  assert.equal(toolLabelKey('measure.distance'), 'tool.measure.distance');
  assert.equal(toolLabelKey('  draw.arc '), 'tool.draw.arc');
  assert.equal(toolLabelKey(''), null);
  assert.equal(toolLabelKey(null), null);
  assert.equal(toolLabelKey(undefined), null);
});

test('every ribbon command id and label key exists in the shared catalog', async () => {
  const { readCatalog } = await import('./helpers.ts');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const ids = ribbonCommandIds();
  assert.ok(ids.length >= 15, `ribbon should reference many commands, saw ${ids.length}`);
  const known = new Set(COMMAND_CATALOG.map(command => command.id));
  for (const id of ids) {
    if (known.has(id)) continue;
    // The one other class a ribbon token may belong to is an **argument form**
    // (`ZOOM WINDOW`, `ZOOM PREVIOUS`): the host advertises only the base
    // selector (`ZOOM`), so there is deliberately **no catalog row** and no
    // vocabulary entry for the word — a row here would be the second name list
    // the command line must not have. Its authority is the argument table, which
    // `splitCommandText` + `ARGUMENT_COMMANDS` is what this asserts.
    const split = splitCommandText(id);
    assert.notEqual(split.argument, null, `ribbon command ${id} is not in the catalog and carries no argument`);
    const spec = ARGUMENT_COMMANDS[split.name];
    assert.ok(spec, `${id}: ${split.name} takes no typed argument`);
    assert.equal(spec.parse(split.argument), true, `${id}: the argument table refuses ${split.argument}`);
  }
  for (const tab of RIBBON_TABS) {
    assert.equal(typeof zh[tab.labelKey], 'string', tab.labelKey);
    for (const panel of tab.panels) {
      assert.equal(typeof zh[panel.titleKey], 'string', panel.titleKey);
      assert.equal(typeof en[panel.titleKey], 'string', panel.titleKey);
      for (const item of panel.items) {
        assert.equal(typeof zh[item.labelKey], 'string', item.labelKey);
        assert.equal(typeof en[item.labelKey], 'string', item.labelKey);
        // A payload-carrying standard view must never also be a `command`: the
        // name-only funnel would refuse it, so the button would lie about its lane.
        if (item.standardView) assert.equal(item.command, undefined, item.id);
        for (const variant of item.variants ?? []) {
          assert.equal(typeof zh[variant.labelKey], 'string', variant.labelKey);
          assert.equal(typeof en[variant.labelKey], 'string', variant.labelKey);
        }
      }
    }
  }
  assert.equal(ribbonTab('nope').id, 'home');
});

// The chrome-local selectors are the one place where typing a word reaches no
// router command at all. This test enumerates the set so a new local selector has
// to be added deliberately (with its dispatch path) rather than by accident.
test('every chrome-local selector is admitted without the host vocabulary and has a dispatch path', async () => {
  const { readModule } = await import('./helpers.ts');
  // The set, in one place: PROPERTIES opens the docked inspector, LAYER opens the
  // docked layers palette. Both are answered by the chrome before the router, so
  // neither appears in the host vocabulary and neither needs a document.
  assert.deepEqual([...LOCAL_CHROME_COMMANDS].sort(), ['LAYER', 'PROPERTIES']);
  for (const id of LOCAL_CHROME_COMMANDS) {
    assert.equal(isLocalChromeCommand(id), true, id);
    assert.equal(isLocalChromeCommand(id.toLowerCase()), true, id);
    assert.equal(isSupportedCommand(id), true, id);
    assert.equal(admitCommandText(id, { status: null }).kind, 'admitted', id);
    assert.equal(admitCommandText(id, { status: null, pending: true }).kind, 'admitted', id);
    assert.equal(admitCommandText(id, { status: empty }).kind, 'admitted', id);
    // The router vocabulary is irrelevant: an empty catalog changes nothing.
    setCommandCatalog([]);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
    setCommandCatalog(HOST_NAMES);
  }
  // A name that looks similar but is not in the set is never admitted locally.
  assert.equal(isLocalChromeCommand('LAYERS'), false);
  assert.equal(LOCAL_CHROME_COMMANDS.includes('LAYERSTATE'), false);
  // The app answers each selector in the command funnel, before the in-flight
  // guard, and never forwards the text to `cad_command`.
  const app = await readModule('App.tsx');
  const guard = app.indexOf('submitting.current = true');
  for (const call of ['isOptionsCommand(text)', 'isPropertiesCommand(text)', 'isLayersCommand(text)']) {
    assert.ok(app.includes(call), `${call} must be intercepted`);
    assert.ok(app.indexOf(call) < guard, `${call} must run before the in-flight guard`);
  }
});

test('the QAT undo/redo buttons dispatch real commands, gated by admission only', async () => {
  const { readModule } = await import('./helpers.ts');
  const titlebar = await readModule('components/TitleBar.tsx');
  // A tokenless QAT entry renders permanently disabled, so both history buttons
  // must carry a real `runCommand` action — the same funnel the command line uses.
  for (const id of ['UNDO', 'REDO']) {
    assert.ok(
      titlebar.includes(`action: () => shell.runCommand('${id}')`),
      `the QAT ${id} entry must dispatch through shell.runCommand`,
    );
    assert.equal(COMMAND_CATALOG.some(entry => entry.id === id), true, `${id} is a catalogued command`);
    assert.equal(isSupportedCommand(id), true, `${id} is a router command`);
  }
  // Availability is the shared admission, never a second local gate: the renderer
  // asks `shell.canCommand(id)` for every QAT entry and nothing else.
  assert.ok(titlebar.includes('const enabled = Boolean(item.action) && shell.canCommand(item.id);'));
  assert.equal(titlebar.includes("item.id === 'UNDO'"), false, 'no per-button local gate');
  // Both are admitted with a drawing open and (like the command line) still
  // accepted with none: they touch no document-scoped capability.
  for (const id of ['UNDO', 'REDO']) {
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
    assert.equal(admitCommandText(id, { status: empty }).kind, 'admitted', id);
    assert.equal(unavailableReasonKey(id, { status: loaded }), 'command.unsupported', `${id} has no special reason`);
  }
  // An empty host vocabulary is the one honest disable: the router does not know
  // the name yet, and the tooltip says so with the shared reason key.
  setCommandCatalog([]);
  assert.equal(admitCommandText('UNDO', { status: loaded }).kind, 'unsupported');
  assert.ok(titlebar.includes('unavailableReasonKey(item.id, { status: shell.status, pending: shell.pending })'));
  setCommandCatalog(HOST_NAMES);
});
