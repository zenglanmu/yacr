// BOUNDARY (AutoCAD 2026) user-facing surface contract.
//
// BOUNDARY is the **internal-point polyline** selector: one pick inside a closed
// boundary (circle, full ellipse or closed polyline), then the host reuses
// HATCHPOINT's discovery and creates one closed, non-associative polyline per loop
// (including nested islands) on the active layer. It is a canonical selector of
// its own — never an alias of HATCH/HATCHPOINT — and `BO`/`BPOLY` are
// deliberately not admitted. The small draw-panel button sits directly after
// HATCHPOINT; the `-BOUNDARY` options are NOT IMPLEMENTED and the copy says so.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  TOOL_READY_PROMPTS,
  TOOL_STEP_PROMPTS,
} from '../src/cad/cad.ts';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
  toolLabelKey,
} from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };

const BOUNDARY = {
  id: 'BOUNDARY',
  labelKey: 'draw.kind.boundary',
  icon: 'polyline',
  toolKey: 'draw.boundary',
  toolLabelKey: 'tool.draw.boundary',
  pick: 'tool.prompt.boundary.pick_internal',
  ready: 'tool.prompt.boundary.ready',
};

const drawPanel = tabId => {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `${tabId} tab must exist`);
  const panel = tab.panels.find(entry => entry.id === 'draw');
  assert.ok(panel, `${tabId} must keep its draw panel`);
  return panel;
};

test('BOUNDARY sits directly after HATCHPOINT on both draw panels, with no popover', () => {
  for (const tabId of ['home', 'draw']) {
    const items = drawPanel(tabId).items;
    const at = items.findIndex(item => item.id === BOUNDARY.id);
    assert.ok(at !== -1, `${tabId}/draw must carry the BOUNDARY button`);
    assert.equal(items[at - 1].id, 'HATCHPOINT', `${tabId}: BOUNDARY follows HATCHPOINT`);
    assert.equal(items.filter(item => item.id === BOUNDARY.id).length, 1, `${tabId}: one BOUNDARY button`);

    const item = items[at];
    assert.equal(item.command, BOUNDARY.id);
    assert.equal(item.labelKey, BOUNDARY.labelKey);
    assert.equal(item.icon, BOUNDARY.icon);
    // The `-BOUNDARY` options are not implemented: the hint names that boundary.
    assert.equal(item.hintKey, 'draw.kind.boundary.hint');
    assert.equal(item.large, undefined, `${tabId}: BOUNDARY is not a primary draw button`);
    assert.equal(item.variants, undefined, `${tabId}: there is no options popover`);
    assert.equal(item.action, undefined, 'BOUNDARY must be a router command, never a shell action');
  }
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      if (panel.id === 'draw') continue;
      assert.equal(panel.items.some(item => item.command === BOUNDARY.id), false, `${tab.id}/${panel.id}`);
    }
  }
  assert.ok(new Set(ribbonCommandIds()).has(BOUNDARY.id));
});

test('BOUNDARY is a catalogued draw command admitted only by the router', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === BOUNDARY.id);
  assert.ok(entry, 'BOUNDARY must be in the command catalog');
  assert.equal(entry.labelKey, BOUNDARY.labelKey);
  assert.equal(entry.icon, BOUNDARY.icon);
  assert.equal(entry.group, 'draw');
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.equal(order.indexOf(BOUNDARY.id), order.indexOf('HATCHPOINT') + 1);
  assert.ok(order.indexOf(BOUNDARY.id) < order.indexOf('MOVE'), 'the HATCH family stays together');

  assert.equal(isSupportedCommand('boundary'), true);
  assert.equal(admitCommandText('BOUNDARY', { status: loaded }).kind, 'admitted');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('BOUNDARY'), false);
  assert.equal(admitCommandText('BOUNDARY', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);

  assert.equal(toolLabelKey(BOUNDARY.toolKey), BOUNDARY.toolLabelKey);
  assert.equal(zh[BOUNDARY.toolLabelKey], '边界');
  assert.equal(typeof en[BOUNDARY.toolLabelKey], 'string');
  // BO/BPOLY are not aliases: only the canonical selector is admitted.
  assert.equal(admitCommandText('BO', { status: loaded }).kind, 'unsupported');
  assert.equal(admitCommandText('BPOLY', { status: loaded }).kind, 'unsupported');
});

test('the chip names the one internal pick, then the captured state without claiming a boundary', () => {
  assert.deepEqual(TOOL_STEP_PROMPTS[BOUNDARY.toolKey], [BOUNDARY.pick]);
  assert.equal(TOOL_READY_PROMPTS[BOUNDARY.toolKey], BOUNDARY.ready);
  for (const catalog of [zh, en]) {
    assert.equal(typeof catalog['draw.kind.boundary'], 'string');
    assert.equal(typeof catalog['draw.kind.boundary.hint'], 'string');
    assert.equal(typeof catalog[BOUNDARY.pick], 'string');
    assert.equal(typeof catalog[BOUNDARY.ready], 'string');
  }
  // The hint must name the unimplemented `-BOUNDARY` options rather than imply
  // that a boundary set / region output / object-type choice exists.
  assert.match(en['draw.kind.boundary.hint'], /-BOUNDARY/);
  assert.match(en['draw.kind.boundary.hint'], /not implemented/i);
});
