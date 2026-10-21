import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  COMMAND_CATALOG,
  commandCatalog,
  defaultSuggestionIndex,
  excludedCommandNames,
  filterCommandSuggestions,
  GESTURE_ONLY_COMMANDS,
  HOST_EXCLUDED_COMMANDS,
  historyText,
  isSupportedCommand,
  normalizeCommandText,
  pushHistory,
  setCommandCatalog,
  stepHistoryCursor,
  toolCommandForKey,
  toolLabelKey,
  unavailableReasonKey,
} from '../src/commands.mjs';
import { RIBBON_TABS, ribbonCommandIds, ribbonTab } from '../src/ribbon-model.mjs';

// Mirrors `cad_ui_core::session::admissible_names()` (canonical `CommandId`
// selectors, interaction-only measurement/zoom selectors, then aliases).
// NEW/SAVE/PLOT are present because the **router** accepts them; the chrome
// excludes them explicitly. `SNAP` is **absent** because the router refuses it
// (AutoCAD's `SNAP` is grid snap, a different feature); `OSNAP` is the
// object-snap toggle the router does accept.
const HOST_NAMES = [
  'OPEN', 'NEW', 'PLOT', 'SAVE', 'CANCEL', 'PAN', 'ZOOM', 'FIT', 'RESETVIEW', 'TOGGLELAYER',
  'SETLAYERS', 'RESTORELAYERS', 'SPACE', 'SELECT', 'SELECTALL', 'MEASURE', 'CONFIRM',
  'CANCELMEASURE', 'UNDO', 'REDO', 'RESOURCES', 'DIAGNOSTICS', 'SWITCHBACKEND', 'SWITCH2D3D',
  'STANDARDVIEW', 'ORBIT', 'PROJECTION', 'MODE', 'LINE', 'CIRCLE', 'ARC', 'PLINE', 'RECTANG', 'ELLIPSE', 'POLYGON', 'POINT', 'SPLINE', 'DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER', 'DIMANGULAR', 'DIMORDINATE', 'DIMBASELINE', 'DIMCONTINUE', 'MOVE', 'TRIM',
  'EXTEND', 'CHAMFER', 'FILLET', 'ARRAYRECT', 'ARRAYPOLAR', 'ALIGN', 'STRETCH', 'ERASE', 'EXPLODE', 'COPY', 'ROTATE', 'SCALE', 'MIRROR', 'COPYCLIP', 'CUTCLIP', 'PASTECLIP',
  'OFFSET', 'SETACTIVELAYER', 'HOVERPREVIEW', 'OSNAP', 'HATCH',
  // TEXT (ADR 0019 P7a): the single-line text command whose text step travels on
  // `cad_set_tool_text`. Its own suite lives in `text-tool.test.mjs`.
  'TEXT',
  // MLEADER (ADR 0019 P7c): the two-point leader whose text step uses the same
  // channel. Its own suite lives in `mleader-tool.test.mjs`.
  'MLEADER',
  // Interaction-only selectors: one per measurement kind, plus fixed zoom steps.
  'DIST', 'MEASUREPOLYLINE', 'MEASUREANGLE', 'AREA', 'ZOOMIN', 'ZOOMOUT',
  // Full-word aliases.
  'EXPORT', 'PRINT', 'SAVEAS', 'QSAVE', 'ZOOM EXTENTS', 'ENTER',
];
setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canFit: true };
const empty = { documentLabel: null, entityCount: null, loading: false, canOpen: true, canFit: false };

test('admission is the loaded host catalog: empty before load, exclusions stay explicit', () => {
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
  // The object-snap toggle is part of the host vocabulary, but grid snap is not:
  // the router refuses `SNAP` (a different feature from object snap), so the
  // grid-snap affordance stays visible yet disabled.
  assert.equal(isSupportedCommand('OSNAP'), true);
  assert.equal(isSupportedCommand('snap'), false);
  // Host-owned names the React host cannot service are excluded explicitly.
  assert.deepEqual(excludedCommandNames(), ['NEW', 'SAVE', 'SAVEAS', 'QSAVE', 'PLOT', 'EXPORT', 'PRINT']);
  assert.deepEqual([...HOST_EXCLUDED_COMMANDS], ['NEW', 'SAVE', 'SAVEAS', 'QSAVE', 'PLOT', 'EXPORT', 'PRINT']);
  for (const name of HOST_EXCLUDED_COMMANDS) assert.equal(isSupportedCommand(name), false, name);
  // Unknown names are never admitted.
  assert.equal(isSupportedCommand('NOT_A_COMMAND'), false);
  assert.equal(isSupportedCommand(''), false);
  assert.equal(normalizeCommandText('  fit '), 'FIT');
  // The setter normalizes and dedupes; then restore the canonical list.
  assert.deepEqual(setCommandCatalog(['line', 'LINE', ' circle ', '']), ['LINE', 'CIRCLE']);
  setCommandCatalog(HOST_NAMES);
});

test('the grid-snap affordance stays visible but router-disabled, and OSNAP stays reachable', async () => {
  const { readModule } = await import('./helpers.mjs');
  // The View tab's display panel keeps GRID/SNAP/ORTHO on screen. Grid snap is
  // not in the router vocabulary, so `SNAP` must stay present yet disabled —
  // never silently deleted and never re-pointed at the object-snap toggle.
  const view = RIBBON_TABS.find(tab => tab.id === 'view');
  const display = view.panels.find(panel => panel.id === 'display');
  assert.ok(display, 'the View tab must keep its display panel');
  assert.deepEqual(display.items.map(item => item.id), ['GRID', 'SNAP', 'ORTHO']);
  const snap = display.items.find(item => item.id === 'SNAP');
  assert.equal(snap.command, 'SNAP');
  assert.equal(snap.labelKey, 'status.toggle.snap');
  // Router admission is the disable mechanism: grid snap and the other unwired
  // aids are refused, while the object-snap selector (a real toggle) is admitted.
  for (const id of ['GRID', 'SNAP', 'ORTHO']) {
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'unsupported', id);
  }
  assert.equal(admitCommandText('OSNAP', { status: loaded }).kind, 'admitted');
  // Autocomplete keeps the same split: `SNAP` shows unsupported, `OSNAP` supported.
  const typedSnap = filterCommandSuggestions('SNAP');
  assert.equal(typedSnap.find(suggestion => suggestion.id === 'SNAP').supported, false);
  assert.equal(typedSnap.find(suggestion => suggestion.id === 'OSNAP').supported, true);
  // The ribbon renderer disables every command item the router does not admit
  // (the same gate GRID/ORTHO use), so the SNAP button is present but disabled.
  // The gate is per dispatch token: a command takes router admission, a typed
  // standard view takes the CAD-session gate, and nothing is locally enabled.
  const ribbon = await readModule('components/Ribbon.jsx');
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

test('ERASE is a modify affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule } = await import('./helpers.mjs');
  // The catalog entry mirrors the neighbouring edit commands (`draw.kind.*`).
  const entry = COMMAND_CATALOG.find(command => command.id === 'ERASE');
  assert.ok(entry, 'ERASE must be in the command catalog');
  assert.equal(entry.labelKey, 'draw.kind.erase');
  assert.equal(entry.icon, 'erase');
  assert.equal(entry.group, 'modify');
  // Placed with the other modify actions on the Home and Draw tabs, never in the
  // view panel, and wired as a router command (no local shell action).
  for (const tabId of ['home', 'draw']) {
    const modify = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'modify');
    assert.ok(modify, `${tabId} must keep its modify panel`);
    const item = modify.items.find(candidate => candidate.id === 'ERASE');
    assert.ok(item, `${tabId}/modify must carry the ERASE affordance`);
    assert.equal(item.command, 'ERASE');
    assert.equal(item.labelKey, 'draw.kind.erase');
    assert.equal(item.action, undefined, 'ERASE must not be a local shell action');
  }
  const view = RIBBON_TABS.find(tab => tab.id === 'view');
  for (const panel of view.panels) {
    assert.equal(panel.items.some(item => item.id === 'ERASE'), false, panel.id);
  }
  // Router admission is the single enable gate: admitted when the host vocabulary
  // carries ERASE, unsupported (never locally enabled) when it does not.
  assert.equal(isSupportedCommand('ERASE'), true);
  assert.equal(admitCommandText('ERASE', { status: loaded }).kind, 'admitted');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('ERASE'), false);
  assert.equal(admitCommandText('ERASE', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  // The renderer dispatches the ERASE selector through the normal command funnel.
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('EXPLODE is a modify affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule, readCatalog } = await import('./helpers.mjs');
  const id = 'EXPLODE';
  // The catalog entry mirrors the neighbouring edit commands (`draw.kind.*`).
  const entry = COMMAND_CATALOG.find(command => command.id === id);
  assert.ok(entry, `${id} must be in the command catalog`);
  assert.equal(entry.labelKey, 'draw.kind.explode');
  assert.equal(entry.icon, 'explode');
  assert.equal(entry.group, 'modify');
  // Placed with the other modify actions on the Home and Draw tabs, wired as a
  // router command (no local shell action).
  for (const tabId of ['home', 'draw']) {
    const modify = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'modify');
    assert.ok(modify, `${tabId} must keep its modify panel`);
    const item = modify.items.find(candidate => candidate.id === id);
    assert.ok(item, `${tabId}/modify must carry the ${id} affordance`);
    assert.equal(item.command, id);
    assert.equal(item.labelKey, 'draw.kind.explode');
    assert.equal(item.action, undefined, `${id} must not be a local shell action`);
  }
  // EXPLODE runs on the current selection with no capture, so it has no
  // active-tool chip key: only the catalog key exists (unlike the draw tools).
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.explode'], 'string');
  assert.equal(typeof en['draw.kind.explode'], 'string');
  // Router admission is the single enable gate: admitted when the host vocabulary
  // carries EXPLODE, unsupported (never locally enabled) when it does not.
  assert.equal(isSupportedCommand(id), true, id);
  assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
  setCommandCatalog([]);
  assert.equal(isSupportedCommand(id), false, id);
  assert.equal(admitCommandText(id, { status: loaded }).kind, 'unsupported', id);
  setCommandCatalog(HOST_NAMES);
  // The renderer dispatches the EXPLODE selector through the normal command funnel.
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
});

test('PLINE is a draw affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule } = await import('./helpers.mjs');
  // The catalog entry mirrors the neighbouring draw commands (`draw.kind.*`).
  const entry = COMMAND_CATALOG.find(command => command.id === 'PLINE');
  assert.ok(entry, 'PLINE must be in the command catalog');
  assert.equal(entry.labelKey, 'draw.kind.polyline');
  assert.equal(entry.icon, 'polyline');
  assert.equal(entry.group, 'draw');
  // Placed with the other draw actions on the Home and Draw tabs, wired as a
  // router command (no local shell action).
  for (const tabId of ['home', 'draw']) {
    const draw = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'draw');
    assert.ok(draw, `${tabId} must keep its draw panel`);
    const item = draw.items.find(candidate => candidate.id === 'PLINE');
    assert.ok(item, `${tabId}/draw must carry the PLINE affordance`);
    assert.equal(item.command, 'PLINE');
    assert.equal(item.labelKey, 'draw.kind.polyline');
    assert.equal(item.action, undefined, 'PLINE must not be a local shell action');
  }
  // The measurement panel must not dispatch `PLINE` (it dispatches the explicit
  // `MEASUREPOLYLINE`), so the polyline draw command never shadows the length
  // measurement.
  const measurePanel = RIBBON_TABS.find(tab => tab.id === 'measure').panels.find(panel => panel.id === 'measure');
  assert.equal(measurePanel.items.some(item => item.command === 'PLINE'), false);
  // Router admission is the single enable gate: admitted when the host vocabulary
  // carries PLINE, unsupported (never locally enabled) when it does not.
  assert.equal(isSupportedCommand('PLINE'), true);
  assert.equal(admitCommandText('PLINE', { status: loaded }).kind, 'admitted');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('PLINE'), false);
  assert.equal(admitCommandText('PLINE', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  // The renderer dispatches the PLINE selector through the normal command funnel.
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('RECTANG is a draw affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule } = await import('./helpers.mjs');
  // The catalog entry mirrors the neighbouring draw commands (`draw.kind.*`).
  const entry = COMMAND_CATALOG.find(command => command.id === 'RECTANG');
  assert.ok(entry, 'RECTANG must be in the command catalog');
  assert.equal(entry.labelKey, 'draw.kind.rectang');
  assert.equal(entry.icon, 'rectang');
  assert.equal(entry.group, 'draw');
  // Placed with the other draw actions on the Home and Draw tabs, wired as a
  // router command (no local shell action).
  for (const tabId of ['home', 'draw']) {
    const draw = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'draw');
    assert.ok(draw, `${tabId} must keep its draw panel`);
    const item = draw.items.find(candidate => candidate.id === 'RECTANG');
    assert.ok(item, `${tabId}/draw must carry the RECTANG affordance`);
    assert.equal(item.command, 'RECTANG');
    assert.equal(item.labelKey, 'draw.kind.rectang');
    assert.equal(item.action, undefined, 'RECTANG must not be a local shell action');
  }
  // The measurement panel never dispatches `RECTANG`: it is a draw command only.
  const measurePanel = RIBBON_TABS.find(tab => tab.id === 'measure').panels.find(panel => panel.id === 'measure');
  assert.equal(measurePanel.items.some(item => item.command === 'RECTANG'), false);
  // Router admission is the single enable gate: admitted when the host vocabulary
  // carries RECTANG, unsupported (never locally enabled) when it does not.
  assert.equal(isSupportedCommand('RECTANG'), true);
  assert.equal(admitCommandText('RECTANG', { status: loaded }).kind, 'admitted');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('RECTANG'), false);
  assert.equal(admitCommandText('RECTANG', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  // The label keys resolve in both catalogs (the ribbon/list name source).
  const { readCatalog } = await import('./helpers.mjs');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.rectang'], 'string');
  assert.equal(typeof en['draw.kind.rectang'], 'string');
  assert.equal(typeof zh['tool.draw.rectang'], 'string');
  assert.equal(typeof en['tool.draw.rectang'], 'string');
  // The renderer dispatches the RECTANG selector through the normal command funnel.
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('ELLIPSE is a draw affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule } = await import('./helpers.mjs');
  // The catalog entry mirrors the neighbouring draw commands (`draw.kind.*`).
  const entry = COMMAND_CATALOG.find(command => command.id === 'ELLIPSE');
  assert.ok(entry, 'ELLIPSE must be in the command catalog');
  assert.equal(entry.labelKey, 'draw.kind.ellipse');
  assert.equal(entry.icon, 'ellipse');
  assert.equal(entry.group, 'draw');
  // Placed with the other draw actions on the Home and Draw tabs, wired as a
  // router command (no local shell action).
  for (const tabId of ['home', 'draw']) {
    const draw = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'draw');
    assert.ok(draw, `${tabId} must keep its draw panel`);
    const item = draw.items.find(candidate => candidate.id === 'ELLIPSE');
    assert.ok(item, `${tabId}/draw must carry the ELLIPSE affordance`);
    assert.equal(item.command, 'ELLIPSE');
    assert.equal(item.labelKey, 'draw.kind.ellipse');
    assert.equal(item.action, undefined, 'ELLIPSE must not be a local shell action');
  }
  // The measurement panel never dispatches `ELLIPSE`: it is a draw command only.
  const measurePanel = RIBBON_TABS.find(tab => tab.id === 'measure').panels.find(panel => panel.id === 'measure');
  assert.equal(measurePanel.items.some(item => item.command === 'ELLIPSE'), false);
  // Router admission is the single enable gate: admitted when the host vocabulary
  // carries ELLIPSE, unsupported (never locally enabled) when it does not.
  assert.equal(isSupportedCommand('ELLIPSE'), true);
  assert.equal(admitCommandText('ELLIPSE', { status: loaded }).kind, 'admitted');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('ELLIPSE'), false);
  assert.equal(admitCommandText('ELLIPSE', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  // The label keys resolve in both catalogs (the ribbon/list name source), and the
  // active-tool chip key (`tool.draw.ellipse`) is what the host key maps onto.
  const { readCatalog } = await import('./helpers.mjs');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.ellipse'], 'string');
  assert.equal(typeof en['draw.kind.ellipse'], 'string');
  assert.equal(typeof zh['tool.draw.ellipse'], 'string');
  assert.equal(typeof en['tool.draw.ellipse'], 'string');
  assert.equal(toolLabelKey('draw.ellipse'), 'tool.draw.ellipse');
  // The renderer dispatches the ELLIPSE selector through the normal command funnel.
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('POLYGON is a draw affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule } = await import('./helpers.mjs');
  // The catalog entry mirrors the neighbouring draw commands (`draw.kind.*`).
  const entry = COMMAND_CATALOG.find(command => command.id === 'POLYGON');
  assert.ok(entry, 'POLYGON must be in the command catalog');
  assert.equal(entry.labelKey, 'draw.kind.polygon');
  assert.equal(entry.icon, 'polygon');
  assert.equal(entry.group, 'draw');
  // Placed with the other draw actions on the Home and Draw tabs, wired as a
  // router command (no local shell action).
  for (const tabId of ['home', 'draw']) {
    const draw = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'draw');
    assert.ok(draw, `${tabId} must keep its draw panel`);
    const item = draw.items.find(candidate => candidate.id === 'POLYGON');
    assert.ok(item, `${tabId}/draw must carry the POLYGON affordance`);
    assert.equal(item.command, 'POLYGON');
    assert.equal(item.labelKey, 'draw.kind.polygon');
    assert.equal(item.action, undefined, 'POLYGON must not be a local shell action');
  }
  // The measurement panel never dispatches `POLYGON`: it is a draw command only.
  const measurePanel = RIBBON_TABS.find(tab => tab.id === 'measure').panels.find(panel => panel.id === 'measure');
  assert.equal(measurePanel.items.some(item => item.command === 'POLYGON'), false);
  // Router admission is the single enable gate: admitted when the host vocabulary
  // carries POLYGON, unsupported (never locally enabled) when it does not.
  assert.equal(isSupportedCommand('POLYGON'), true);
  assert.equal(admitCommandText('POLYGON', { status: loaded }).kind, 'admitted');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('POLYGON'), false);
  assert.equal(admitCommandText('POLYGON', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  // The label keys resolve in both catalogs (the ribbon/list name source).
  const { readCatalog } = await import('./helpers.mjs');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.polygon'], 'string');
  assert.equal(typeof en['draw.kind.polygon'], 'string');
  assert.equal(typeof zh['tool.draw.polygon'], 'string');
  assert.equal(typeof en['tool.draw.polygon'], 'string');
  assert.equal(toolLabelKey('draw.polygon'), 'tool.draw.polygon');
  // The renderer dispatches the POLYGON selector through the normal command funnel.
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('POINT is a draw affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'POINT');
  assert.ok(entry, 'POINT must be in the command catalog');
  assert.equal(entry.labelKey, 'draw.kind.point');
  assert.equal(entry.icon, 'point');
  assert.equal(entry.group, 'draw');
  for (const tabId of ['home', 'draw']) {
    const draw = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'draw');
    const item = draw.items.find(candidate => candidate.id === 'POINT');
    assert.ok(item, `${tabId}/draw must carry the POINT affordance`);
    assert.equal(item.command, 'POINT');
    assert.equal(item.labelKey, 'draw.kind.point');
    assert.equal(item.action, undefined, 'POINT must not be a local shell action');
  }
  const measurePanel = RIBBON_TABS.find(tab => tab.id === 'measure').panels.find(panel => panel.id === 'measure');
  assert.equal(measurePanel.items.some(item => item.command === 'POINT'), false);
  assert.equal(isSupportedCommand('POINT'), true);
  assert.equal(admitCommandText('POINT', { status: loaded }).kind, 'admitted');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('POINT'), false);
  assert.equal(admitCommandText('POINT', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  const { readCatalog } = await import('./helpers.mjs');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.point'], 'string');
  assert.equal(typeof en['draw.kind.point'], 'string');
  assert.equal(typeof zh['tool.draw.point'], 'string');
  assert.equal(typeof en['tool.draw.point'], 'string');
  assert.equal(toolLabelKey('draw.point'), 'tool.draw.point');
});

test('SPLINE is a draw affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'SPLINE');
  assert.ok(entry, 'SPLINE must be in the command catalog');
  assert.equal(entry.labelKey, 'draw.kind.spline');
  assert.equal(entry.icon, 'spline');
  assert.equal(entry.group, 'draw');
  for (const tabId of ['home', 'draw']) {
    const draw = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'draw');
    const item = draw.items.find(candidate => candidate.id === 'SPLINE');
    assert.ok(item, `${tabId}/draw must carry the SPLINE affordance`);
    assert.equal(item.command, 'SPLINE');
    assert.equal(item.labelKey, 'draw.kind.spline');
    assert.equal(item.action, undefined, 'SPLINE must not be a local shell action');
  }
  const measurePanel = RIBBON_TABS.find(tab => tab.id === 'measure').panels.find(panel => panel.id === 'measure');
  assert.equal(measurePanel.items.some(item => item.command === 'SPLINE'), false);
  assert.equal(isSupportedCommand('SPLINE'), true);
  assert.equal(admitCommandText('SPLINE', { status: loaded }).kind, 'admitted');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('SPLINE'), false);
  assert.equal(admitCommandText('SPLINE', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  const { readCatalog } = await import('./helpers.mjs');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.spline'], 'string');
  assert.equal(typeof en['draw.kind.spline'], 'string');
  assert.equal(typeof zh['tool.draw.spline'], 'string');
  assert.equal(typeof en['tool.draw.spline'], 'string');
  assert.equal(toolLabelKey('draw.spline'), 'tool.draw.spline');
});

test('CHAMFER and FILLET are modify affordances in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule, readCatalog } = await import('./helpers.mjs');
  for (const id of ['CHAMFER', 'FILLET']) {
    const entry = COMMAND_CATALOG.find(command => command.id === id);
    assert.ok(entry, `${id} must be in the command catalog`);
    assert.equal(entry.labelKey, `draw.kind.${id.toLowerCase()}`);
    assert.equal(entry.icon, id.toLowerCase());
    assert.equal(entry.group, 'modify');
    // Placed with the other modify actions on the Home and Draw tabs.
    for (const tabId of ['home', 'draw']) {
      const modify = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'modify');
      assert.ok(modify, `${tabId} must keep its modify panel`);
      const item = modify.items.find(candidate => candidate.id === id);
      assert.ok(item, `${tabId}/modify must carry the ${id} affordance`);
      assert.equal(item.command, id);
      assert.equal(item.labelKey, `draw.kind.${id.toLowerCase()}`);
      assert.equal(item.action, undefined, `${id} must not be a local shell action`);
    }
    // The measurement panel never dispatches it.
    const measurePanel = RIBBON_TABS.find(tab => tab.id === 'measure').panels.find(panel => panel.id === 'measure');
    assert.equal(measurePanel.items.some(item => item.command === id), false);
    // Router admission is the single enable gate.
    assert.equal(isSupportedCommand(id), true, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
    setCommandCatalog([]);
    assert.equal(isSupportedCommand(id), false, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'unsupported', id);
    setCommandCatalog(HOST_NAMES);
    // The catalog keys resolve in both catalogs, including the active-tool chip.
    const zh = await readCatalog('zh-CN');
    const en = await readCatalog('en');
    assert.equal(typeof zh[`draw.kind.${id.toLowerCase()}`], 'string');
    assert.equal(typeof en[`draw.kind.${id.toLowerCase()}`], 'string');
    assert.equal(typeof zh[`tool.draw.${id.toLowerCase()}`], 'string');
    assert.equal(typeof en[`tool.draw.${id.toLowerCase()}`], 'string');
    assert.equal(toolLabelKey(`draw.${id.toLowerCase()}`), `tool.draw.${id.toLowerCase()}`);
  }
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
});

test('ARRAYRECT and ARRAYPOLAR are modify affordances in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule, readCatalog } = await import('./helpers.mjs');
  for (const id of ['ARRAYRECT', 'ARRAYPOLAR']) {
    const entry = COMMAND_CATALOG.find(command => command.id === id);
    assert.ok(entry, `${id} must be in the command catalog`);
    assert.equal(entry.labelKey, `draw.kind.${id.toLowerCase()}`);
    assert.equal(entry.icon, id.toLowerCase());
    assert.equal(entry.group, 'modify');
    // Placed with the other modify actions on the Home and Draw tabs.
    for (const tabId of ['home', 'draw']) {
      const modify = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'modify');
      assert.ok(modify, `${tabId} must keep its modify panel`);
      const item = modify.items.find(candidate => candidate.id === id);
      assert.ok(item, `${tabId}/modify must carry the ${id} affordance`);
      assert.equal(item.command, id);
      assert.equal(item.labelKey, `draw.kind.${id.toLowerCase()}`);
      assert.equal(item.action, undefined, `${id} must not be a local shell action`);
    }
    // The measurement panel never dispatches it.
    const measurePanel = RIBBON_TABS.find(tab => tab.id === 'measure').panels.find(panel => panel.id === 'measure');
    assert.equal(measurePanel.items.some(item => item.command === id), false);
    // Router admission is the single enable gate.
    assert.equal(isSupportedCommand(id), true, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
    setCommandCatalog([]);
    assert.equal(isSupportedCommand(id), false, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'unsupported', id);
    setCommandCatalog(HOST_NAMES);
    // The catalog keys resolve in both catalogs, including the active-tool chip.
    const zh = await readCatalog('zh-CN');
    const en = await readCatalog('en');
    assert.equal(typeof zh[`draw.kind.${id.toLowerCase()}`], 'string');
    assert.equal(typeof en[`draw.kind.${id.toLowerCase()}`], 'string');
    assert.equal(typeof zh[`tool.draw.${id.toLowerCase()}`], 'string');
    assert.equal(typeof en[`tool.draw.${id.toLowerCase()}`], 'string');
    assert.equal(toolLabelKey(`draw.${id.toLowerCase()}`), `tool.draw.${id.toLowerCase()}`);
  }
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
});

test('ALIGN is a modify affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule, readCatalog } = await import('./helpers.mjs');
  const id = 'ALIGN';
  const entry = COMMAND_CATALOG.find(command => command.id === id);
  assert.ok(entry, `${id} must be in the command catalog`);
  assert.equal(entry.labelKey, 'draw.kind.align');
  assert.equal(entry.icon, 'align');
  assert.equal(entry.group, 'modify');
  // Placed with the other modify actions on the Home and Draw tabs.
  for (const tabId of ['home', 'draw']) {
    const modify = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'modify');
    assert.ok(modify, `${tabId} must keep its modify panel`);
    const item = modify.items.find(candidate => candidate.id === id);
    assert.ok(item, `${tabId}/modify must carry the ${id} affordance`);
    assert.equal(item.command, id);
    assert.equal(item.labelKey, 'draw.kind.align');
    assert.equal(item.action, undefined, `${id} must not be a local shell action`);
  }
  // The measurement panel never dispatches it.
  const measurePanel = RIBBON_TABS.find(tab => tab.id === 'measure').panels.find(panel => panel.id === 'measure');
  assert.equal(measurePanel.items.some(item => item.command === id), false);
  // Router admission is the single enable gate.
  assert.equal(isSupportedCommand(id), true, id);
  assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
  setCommandCatalog([]);
  assert.equal(isSupportedCommand(id), false, id);
  assert.equal(admitCommandText(id, { status: loaded }).kind, 'unsupported', id);
  setCommandCatalog(HOST_NAMES);
  // The catalog keys resolve in both catalogs, including the active-tool chip.
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.align'], 'string');
  assert.equal(typeof en['draw.kind.align'], 'string');
  assert.equal(typeof zh['tool.draw.align'], 'string');
  assert.equal(typeof en['tool.draw.align'], 'string');
  assert.equal(toolLabelKey('draw.align'), 'tool.draw.align');
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
});

test('STRETCH is a modify affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule, readCatalog } = await import('./helpers.mjs');
  const id = 'STRETCH';
  const entry = COMMAND_CATALOG.find(command => command.id === id);
  assert.ok(entry, `${id} must be in the command catalog`);
  assert.equal(entry.labelKey, 'draw.kind.stretch');
  assert.equal(entry.icon, 'stretch');
  assert.equal(entry.group, 'modify');
  // Placed with the other modify actions on the Home and Draw tabs.
  for (const tabId of ['home', 'draw']) {
    const modify = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'modify');
    assert.ok(modify, `${tabId} must keep its modify panel`);
    const item = modify.items.find(candidate => candidate.id === id);
    assert.ok(item, `${tabId}/modify must carry the ${id} affordance`);
    assert.equal(item.command, id);
    assert.equal(item.labelKey, 'draw.kind.stretch');
    assert.equal(item.action, undefined, `${id} must not be a local shell action`);
  }
  // The measurement panel never dispatches it.
  const measurePanel = RIBBON_TABS.find(tab => tab.id === 'measure').panels.find(panel => panel.id === 'measure');
  assert.equal(measurePanel.items.some(item => item.command === id), false);
  // Router admission is the single enable gate.
  assert.equal(isSupportedCommand(id), true, id);
  assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
  setCommandCatalog([]);
  assert.equal(isSupportedCommand(id), false, id);
  assert.equal(admitCommandText(id, { status: loaded }).kind, 'unsupported', id);
  setCommandCatalog(HOST_NAMES);
  // The catalog keys resolve in both catalogs, including the active-tool chip.
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.stretch'], 'string');
  assert.equal(typeof en['draw.kind.stretch'], 'string');
  assert.equal(typeof zh['tool.draw.stretch'], 'string');
  assert.equal(typeof en['tool.draw.stretch'], 'string');
  assert.equal(toolLabelKey('draw.stretch'), 'tool.draw.stretch');
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
});

test('DIMLINEAR and DIMALIGNED are annotate affordances in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule, readCatalog } = await import('./helpers.mjs');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const expected = [
    { id: 'DIMLINEAR', labelKey: 'draw.kind.dimlinear', icon: 'dimlinear', tool: 'tool.draw.dimlinear', toolKey: 'draw.dimlinear' },
    { id: 'DIMALIGNED', labelKey: 'draw.kind.dimaligned', icon: 'dimaligned', tool: 'tool.draw.dimaligned', toolKey: 'draw.dimaligned' },
  ];
  const annotate = RIBBON_TABS.find(tab => tab.id === 'annotate');
  assert.ok(annotate, 'the annotate tab must exist');
  const panel = annotate.panels.find(entry => entry.id === 'annotation');
  assert.ok(panel, 'the annotate tab must keep its annotation panel');
  // The dimension pair leads the annotation panel, linear before aligned, so a
  // CAD user finds dimension creation where AutoCAD's Annotate tab puts it.
  assert.deepEqual(panel.items.slice(0, 2).map(item => item.id), ['DIMLINEAR', 'DIMALIGNED']);
  for (const { id, labelKey, icon, tool, toolKey } of expected) {
    // The catalog entry mirrors the draw/measure creation commands (`draw.kind.*`).
    const entry = COMMAND_CATALOG.find(command => command.id === id);
    assert.ok(entry, `${id} must be in the command catalog`);
    assert.equal(entry.labelKey, labelKey);
    assert.equal(entry.icon, icon);
    assert.equal(entry.group, 'annotate');
    // Ribbon entry: the annotation panel, wired as a router command (never a
    // local shell action), rendered as a large creation button.
    const item = panel.items.find(candidate => candidate.id === id);
    assert.ok(item, `annotate/annotation must carry the ${id} affordance`);
    assert.equal(item.command, id);
    assert.equal(item.labelKey, labelKey);
    assert.equal(item.icon, icon);
    assert.equal(item.large, true);
    assert.equal(item.action, undefined, `${id} must not be a local shell action`);
    // No other tab's panel dispatches it (it is an annotate affordance only).
    for (const tab of RIBBON_TABS) {
      for (const other of tab.panels) {
        if (other === panel) continue;
        assert.equal(other.items.some(candidate => candidate.command === id), false, `${tab.id}/${other.id} must not dispatch ${id}`);
      }
    }
    // Router admission is the single enable gate: admitted when the host
    // vocabulary carries it, unsupported (never locally enabled) when it does not.
    assert.equal(isSupportedCommand(id), true, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
    setCommandCatalog([]);
    assert.equal(isSupportedCommand(id), false, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'unsupported', id);
    setCommandCatalog(HOST_NAMES);
    // The label keys resolve in both catalogs, and the active-tool chip key is
    // what the host key maps onto.
    assert.equal(typeof zh[labelKey], 'string');
    assert.equal(typeof en[labelKey], 'string');
    assert.equal(typeof zh[tool], 'string');
    assert.equal(typeof en[tool], 'string');
    assert.equal(toolLabelKey(toolKey), tool);
  }
  // The renderer dispatches the selector through the normal command funnel.
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

// Chrome catalog entries the router does not accept: they stay **visible but
// permanently disabled** (never deleted, never locally enabled) — the same
// contract the grid-snap affordance holds. This is the audited dead set; a
// future edit that adds or restores an affordance must move its id out of this
// list.
//
// Remaining dead affordances, and why:
//  - GRID, ORTHO: unimplemented drafting aids (the router has no selector).
//  - SNAP: AutoCAD's grid snap (F9), deliberately absent from the router; it is
//    a different feature from object snap (`OSNAP`) and must never alias it.
//  - LAYER, PROPERTIES: panel affordances deferred to a later phase.
// The four measurement kinds (DIST/MEASUREPOLYLINE/MEASUREANGLE/AREA) and the
// fixed zoom steps (ZOOMIN/ZOOMOUT) were fixed: they are router-admitted
// selectors now and moved into HOST_NAMES. `PLINE` is now the polyline **draw**
// selector (a router-admitted `COMMAND_TABLE` entry → `BeginDraw(Polyline)`), so
// it is a real affordance, not reserved. `ANGLE` (not an AutoCAD command) is
// deliberately **not** in this set: it is an unbound name, not a dead
// affordance. `ORBIT` is **gesture-only** (router-admitted, click-disabled
// with an explicit reason) and is asserted separately via GESTURE_ONLY_COMMANDS,
// not lumped into this router-refused set.
const ROUTER_REFUSED_AFFORDANCES = ['GRID', 'LAYER', 'ORTHO', 'PROPERTIES', 'SNAP'];

test('the command catalog lists exactly the router-refused dead affordances, and ERASE is not one', () => {
  const refused = COMMAND_CATALOG
    .filter(command => !HOST_EXCLUDED_COMMANDS.includes(command.id)
      && !GESTURE_ONLY_COMMANDS.includes(command.id)
      && !isSupportedCommand(command.id))
    .map(command => command.id)
    .sort();
  assert.deepEqual(refused, [...ROUTER_REFUSED_AFFORDANCES].sort());
  for (const id of ROUTER_REFUSED_AFFORDANCES) {
    assert.equal(isSupportedCommand(id), false, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'unsupported', id);
  }
  assert.equal(ROUTER_REFUSED_AFFORDANCES.includes('ERASE'), false, 'ERASE must be reachable');
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

test('the four measurement buttons dispatch their own admitted kind selectors', async () => {
  const { readModule } = await import('./helpers.mjs');
  const measure = RIBBON_TABS.find(tab => tab.id === 'measure');
  const panel = measure.panels.find(candidate => candidate.id === 'measure');
  assert.ok(panel, 'the Measure tab keeps its measurement panel');
  // Each button dispatches its own selector; none collapses onto
  // `MEASURE`/Distance. The internal ids keep the familiar words, but the
  // **dispatched selectors** are explicit: `PLINE` stays reserved for the P3
  // polyline draw command and `ANGLE` is deliberately unbound.
  const dispatched = panel.items.map(item => item.command);
  assert.deepEqual(dispatched, ['DIST', 'MEASUREPOLYLINE', 'MEASUREANGLE', 'AREA']);
  const idToSelector = Object.fromEntries(panel.items.map(item => [item.id, item.command]));
  assert.deepEqual(idToSelector, {
    DIST: 'DIST',
    PLINE: 'MEASUREPOLYLINE',
    ANGLE: 'MEASUREANGLE',
    AREA: 'AREA',
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
  // Distinct labels, so four different measurement kinds are presented.
  assert.equal(new Set(panel.items.map(item => item.labelKey)).size, 4);
  // The renderer dispatches each selector through the single command funnel.
  const ribbon = await readModule('components/Ribbon.jsx');
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

test('ORBIT stays visible but gesture-disabled with an explicit orbit reason', async () => {
  const { readModule, readCatalog } = await import('./helpers.mjs');
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
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(ribbon.includes('unavailableReasonKey(item.command)'), 'the ribbon must use the explicit reason key');
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
  // A router-refused display id (`LAYER`, a later-phase panel) stays unsupported.
  const layer = filterCommandSuggestions('LAYER');
  assert.equal(layer.find(suggestion => suggestion.id === 'LAYER').supported, false);
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
  // Interactive tools the router accepts are admitted; excluded/unknown names are not.
  assert.equal(admitCommandText('LINE', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('CONFIRM', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('NEW', { status: loaded }).kind, 'unsupported');
  assert.equal(admitCommandText('SAVE', { status: loaded }).kind, 'unsupported');
  assert.equal(admitCommandText('PLOT', { status: loaded }).kind, 'unsupported');
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

test('a host tool selector maps to its catalog key', () => {
  assert.equal(toolLabelKey('draw.line'), 'tool.draw.line');
  assert.equal(toolLabelKey('measure.distance'), 'tool.measure.distance');
  assert.equal(toolLabelKey('  draw.arc '), 'tool.draw.arc');
  assert.equal(toolLabelKey(''), null);
  assert.equal(toolLabelKey(null), null);
  assert.equal(toolLabelKey(undefined), null);
});

test('every ribbon command id and label key exists in the shared catalog', async () => {
  const { readCatalog } = await import('./helpers.mjs');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const ids = ribbonCommandIds();
  assert.ok(ids.length >= 15, `ribbon should reference many commands, saw ${ids.length}`);
  const known = new Set(COMMAND_CATALOG.map(command => command.id));
  for (const id of ids) assert.ok(known.has(id), `ribbon command ${id} is not in the catalog`);
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
