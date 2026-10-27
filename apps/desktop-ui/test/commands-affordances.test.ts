// Ribbon affordance contracts: every draw/modify/annotate button must be the
// router selector it claims to be — visible, admitted only through the loaded
// host vocabulary, and dispatching through the one command funnel.
//
// Split out of `commands.test.mjs` to keep both files under the 1000-line code
// cap (`scripts/check-file-size.py`); these are the same contracts, moved, not a
// second copy of them.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
  toolLabelKey,
} from '../src/commands/commands.ts';
import { GESTURE_ONLY_COMMANDS } from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES } from './helpers.ts';

setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };

test('ERASE is a modify affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule } = await import('./helpers.ts');
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
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('EXPLODE is a modify affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule, readCatalog } = await import('./helpers.ts');
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
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
});

test('PLINE is a draw affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule } = await import('./helpers.ts');
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
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('RECTANG is a draw affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule } = await import('./helpers.ts');
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
  const { readCatalog } = await import('./helpers.ts');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.rectang'], 'string');
  assert.equal(typeof en['draw.kind.rectang'], 'string');
  assert.equal(typeof zh['tool.draw.rectang'], 'string');
  assert.equal(typeof en['tool.draw.rectang'], 'string');
  // The renderer dispatches the RECTANG selector through the normal command funnel.
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('ELLIPSE is a draw affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule } = await import('./helpers.ts');
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
  const { readCatalog } = await import('./helpers.ts');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.ellipse'], 'string');
  assert.equal(typeof en['draw.kind.ellipse'], 'string');
  assert.equal(typeof zh['tool.draw.ellipse'], 'string');
  assert.equal(typeof en['tool.draw.ellipse'], 'string');
  assert.equal(toolLabelKey('draw.ellipse'), 'tool.draw.ellipse');
  // The renderer dispatches the ELLIPSE selector through the normal command funnel.
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('POLYGON is a draw affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule } = await import('./helpers.ts');
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
  const { readCatalog } = await import('./helpers.ts');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.polygon'], 'string');
  assert.equal(typeof en['draw.kind.polygon'], 'string');
  assert.equal(typeof zh['tool.draw.polygon'], 'string');
  assert.equal(typeof en['tool.draw.polygon'], 'string');
  assert.equal(toolLabelKey('draw.polygon'), 'tool.draw.polygon');
  // The renderer dispatches the POLYGON selector through the normal command funnel.
  const ribbon = await readModule('components/Ribbon.tsx');
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
  const { readCatalog } = await import('./helpers.ts');
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
  const { readCatalog } = await import('./helpers.ts');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.spline'], 'string');
  assert.equal(typeof en['draw.kind.spline'], 'string');
  assert.equal(typeof zh['tool.draw.spline'], 'string');
  assert.equal(typeof en['tool.draw.spline'], 'string');
  assert.equal(toolLabelKey('draw.spline'), 'tool.draw.spline');
});

test('CHAMFER and FILLET are modify affordances in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule, readCatalog } = await import('./helpers.ts');
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
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
});

test('ARRAYRECT and ARRAYPOLAR are modify affordances in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule, readCatalog } = await import('./helpers.ts');
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
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
});

test('ALIGN is a modify affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule, readCatalog } = await import('./helpers.ts');
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
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
});

test('STRETCH is a modify affordance in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule, readCatalog } = await import('./helpers.ts');
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
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
});

test('DIMLINEAR and DIMALIGNED are annotate affordances in the ribbon and catalog, enabled only by router admission', async () => {
  const { readModule, readCatalog } = await import('./helpers.ts');
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
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('MATCHPROP sits with the selection-acting modify commands and needs a source and a target', async () => {
  const { readCatalog, readModule } = await import('./helpers.ts');
  const id = 'MATCHPROP';
  // The catalog entry mirrors the neighbouring edit commands (`draw.kind.*`).
  const entry = COMMAND_CATALOG.find(command => command.id === id);
  assert.ok(entry, `${id} must be in the command catalog`);
  assert.equal(entry.labelKey, 'draw.kind.matchprop');
  assert.equal(entry.icon, 'matchprop');
  assert.equal(entry.group, 'modify');
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.equal(typeof zh['draw.kind.matchprop'], 'string');
  assert.equal(typeof en['draw.kind.matchprop'], 'string');
  // Both catalogs word the requirement its disabled state shows; the copy is the
  // only place the source→targets rule is spelled out for the reader, and it must
  // name the *two* objects the machine rule requires.
  for (const locale of [zh, en]) {
    assert.equal(typeof locale['draw.kind.matchprop.hint'], 'string');
  }
  assert.match(en['draw.kind.matchprop.hint'], /at least two|two selected/i);
  assert.match(zh['draw.kind.matchprop.hint'], /两个/);
  assert.match(en['draw.kind.matchprop.hint'], /\{command\}/, 'the ribbon hint names the item');
  assert.match(en['draw.kind.matchprop.hint'], /first selected/i);
  assert.match(en['draw.kind.matchprop.hint'], /cop(?:y|ies)/i);

  // Placed with the other modify actions on the Home and Draw tabs, directly
  // after ERASE/EXPLODE — the panel's other commands that act on the **current**
  // selection — and wired as a router command (no local shell action).
  for (const tabId of ['home', 'draw']) {
    const modify = RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'modify');
    assert.ok(modify, `${tabId} must keep its modify panel`);
    const item = modify.items.find(candidate => candidate.id === id);
    assert.ok(item, `${tabId}/modify must carry the ${id} affordance`);
    assert.equal(item.command, id);
    assert.equal(item.labelKey, 'draw.kind.matchprop');
    assert.equal(item.icon, 'matchprop');
    assert.equal(item.action, undefined, `${id} must not be a local shell action`);
    assert.equal(item.large, undefined, `${id} is a small modify button, not a primary tile`);
    // No caret: MATCHPROP's Settings sub-prompt (which properties to copy) is not
    // implemented, so a popover would imply choices the command does not have.
    assert.equal(item.variants, undefined, `${id} has no variant popover`);
    assert.equal(modify.items.filter(candidate => candidate.id === id).length, 1, `${tabId}: one ${id} button`);
    const order = modify.items.map(candidate => candidate.id);
    const at = order.indexOf(id);
    // Directly after the selection-acting pair on both tabs (the draw panel puts
    // MOVE/COPY after them, so the shared anchor is EXPLODE, not its successor).
    assert.equal(order[at - 1], 'EXPLODE', `${tabId}: ${id} follows EXPLODE`);
    assert.equal(order[at - 2], 'ERASE', `${tabId}: ERASE precedes EXPLODE`);
  }
  // It is a modify action only: no draw/annotate/view panel dispatches it.
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      if (panel.id === 'modify') continue;
      assert.equal(panel.items.some(item => item.command === id), false, `${tab.id}/${panel.id} must not dispatch ${id}`);
    }
  }
  // The glyph really exists (a missing name would silently draw the `dot`), and it
  // is its own mark rather than a reuse of the neighbouring edit glyphs.
  const icons = await readModule('icons.tsx');
  const map = icons.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  const glyph = map[1].match(/\n\s*matchprop:\s*(<><[\s\S]*?<\/>),/);
  assert.ok(glyph, "icons.tsx must define 'matchprop'");
  for (const other of ['pencil', 'erase', 'explode']) {
    const neighbour = map[1].match(new RegExp(`\\n\\s*${other}:\\s*(?:<\\/>)?(<><[\\s\\S]*?<\\/>)\\s*,`))
      ?? map[1].match(new RegExp(`\\n\\s*${other}:\\s*(<[\\s\\S]*?),\\n`));
    assert.ok(neighbour, `expected the ${other} glyph`);
    assert.notEqual(glyph[1], neighbour[1], `${id} must not reuse the ${other} glyph`);
  }
  // The one filled shape in the set is the paint the brush carries; the bristle
  // face it fills is what keeps the glyph legible at the ribbon's 16px.
  assert.match(glyph[1], /fill="currentColor"/, 'the bristle face is filled');

  // Router admission is the single enable gate, and the vocabulary alone is not
  // enough: MATCHPROP reads the current selection, so the selection requirement
  // decides too. The machine rule and the copy are pinned together here — the
  // sentence names the two objects its `count` requires.
  const { SELECTION_REQUIREMENTS } = await import('../src/commands/gating.ts');
  const { selectionRequirementMet, unavailableReasonKey } = await import('../src/commands/commands.ts');
  assert.deepEqual(SELECTION_REQUIREMENTS[id], { count: 2, reasonKey: 'draw.kind.matchprop.hint' });
  assert.equal(isSupportedCommand(id), true, id);
  const doc = loaded;
  const selection = count => ({ ...doc, selection: { count, empty: count === 0 } });
  assert.equal(selectionRequirementMet(id, selection(2)), true);
  assert.equal(selectionRequirementMet(id, selection(0)), false);
  assert.equal(admitCommandText(id, { status: selection(3) }).kind, 'admitted', id);
  assert.equal(admitCommandText(id, { status: selection(2) }).kind, 'admitted', 'source + one target is enough');
  // One object is a source with nowhere to go; none is nothing at all. Both are
  // refused *before* IPC with the sentence that says what is missing — the same
  // key the disabled ribbon button shows, so button and command line agree.
  for (const count of [0, 1]) {
    const admission = admitCommandText(id, { status: selection(count) });
    assert.equal(admission.kind, 'unavailable', `selection of ${count}`);
    assert.equal(admission.reasonKey, 'draw.kind.matchprop.hint', `selection of ${count}`);
    assert.equal(unavailableReasonKey(id, { status: selection(count) }), 'draw.kind.matchprop.hint');
  }
  // The status-less and loading paths keep their existing answers.
  assert.equal(admitCommandText(id, { status: null }).kind, 'unavailable', id);
  assert.equal(admitCommandText(id, { status: { ...selection(3), loading: true } }).kind, 'unavailable', id);
  assert.equal(unavailableReasonKey(id, { status: { ...selection(3), loading: true } }), 'linux.busy');
  assert.equal(unavailableReasonKey(id, { status: selection(1), pending: true }), 'linux.busy');
  // No drawing: there is no selection block to read, so the honest sentence is
  // the catalog's own "no drawing is open" — never "select more objects".
  const noDocument = { ...doc, documentLabel: null, entityCount: null, selection: { count: 0, empty: true } };
  assert.equal(admitCommandText(id, { status: noDocument }).reasonKey, 'plot.no_document');
  assert.equal(unavailableReasonKey(id, { status: noDocument }), 'plot.no_document');
  // Without the host vocabulary the refusal is the generic one, not the selection
  // sentence: "not supported" and "select more objects" are different facts.
  setCommandCatalog([]);
  assert.equal(isSupportedCommand(id), false, id);
  assert.equal(admitCommandText(id, { status: selection(3) }).kind, 'unsupported', id);
  setCommandCatalog(HOST_NAMES);
  // The neighbour contract is untouched: ERASE also acts on the selection but has
  // no *count* precondition, so it stays admitted on an empty selection and the
  // host answers at run time (`draw.error.selection_required`).
  assert.equal(admitCommandText('ERASE', { status: selection(0) }).kind, 'admitted');
});


