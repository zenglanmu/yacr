// HATCHPOINT ("Hatch by point") user-facing surface contract.
//
// HATCHPOINT is HATCH's **point-pick entry**: the user picks one internal point
// inside a closed boundary (whole circle, full ellipse or closed polyline), the
// host finds the containing boundary with its nested islands and hatches with
// the **same** ten fill parameters HATCH uses. It is deliberately **not** a
// pre-selection tool and has exactly one named pick, so its chip asks for that
// point and then names the captured state — it must never claim a boundary was
// found, because the host has not searched for one yet at that moment.
//
// The button is a small button directly after HATCH on both draw panels, it
// reuses the `hatch` glyph (no second icon asset), and its enable state is
// router admission like every other command. Every user-visible string comes
// from the shared catalogs.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  PRESELECT_PROMPTS,
  TOOL_READY_PROMPTS,
  TOOL_STEP_PROMPTS,
  toolStepPromptKey,
} from '../src/cad/cad.ts';
import { TOOL_PARAMETERS } from '../src/tools/tool-parameter-specs.ts';
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

const HATCH_POINT = {
  id: 'HATCHPOINT',
  labelKey: 'draw.kind.hatchpoint',
  icon: 'hatch',
  toolKey: 'draw.hatchpoint',
  toolLabelKey: 'tool.draw.hatchpoint',
  pick: 'tool.prompt.hatchpoint.pick_internal',
  ready: 'tool.prompt.hatchpoint.ready',
};

const drawPanel = tabId => {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `${tabId} tab must exist`);
  const panel = tab.panels.find(entry => entry.id === 'draw');
  assert.ok(panel, `${tabId} must keep its draw panel`);
  return panel;
};

test('HATCHPOINT sits directly after HATCH on both draw panels, with no popover', () => {
  for (const tabId of ['home', 'draw']) {
    const items = drawPanel(tabId).items;
    const at = items.findIndex(item => item.id === HATCH_POINT.id);
    assert.ok(at !== -1, `${tabId}/draw must carry the HATCHPOINT button`);
    // The existing HATCH button is untouched and still directly before it, so no
    // shipped row moved: the pair reads `HATCH` → `HATCHPOINT`.
    assert.equal(items[at - 1].id, 'HATCH', `${tabId}: HATCHPOINT follows HATCH`);
    assert.equal(items.filter(item => item.id === HATCH_POINT.id).length, 1, `${tabId}: one HATCHPOINT button`);

    const item = items[at];
    assert.equal(item.command, HATCH_POINT.id);
    assert.equal(item.labelKey, HATCH_POINT.labelKey);
    // The same glyph as HATCH: the icon set has no point mark, and a new asset is
    // not part of this command.
    assert.equal(item.icon, HATCH_POINT.icon);
    // A small button like HATCH, with no variant popover: the command has no
    // sub-prompt and no second parameter set (it reuses HATCH's ten rows).
    assert.equal(item.large, undefined, `${tabId}: HATCHPOINT is not a primary draw button`);
    assert.equal(item.variants, undefined);
    assert.equal(item.action, undefined, 'HATCHPOINT must be a router command, never a shell action');
  }
  // No other panel dispatches it.
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      if (panel.id === 'draw') continue;
      assert.equal(panel.items.some(item => item.command === HATCH_POINT.id), false, `${tab.id}/${panel.id}`);
    }
  }
  assert.ok(new Set(ribbonCommandIds()).has(HATCH_POINT.id));
});

test('HATCHPOINT is a catalogued draw command admitted only by the router', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === HATCH_POINT.id);
  assert.ok(entry, 'HATCHPOINT must be in the command catalog');
  assert.equal(entry.labelKey, HATCH_POINT.labelKey);
  assert.equal(entry.icon, HATCH_POINT.icon, 'it reuses the HATCH glyph');
  assert.equal(entry.group, 'draw');
  // It is appended directly after HATCH, so HATCH's own listing slot is unchanged
  // and the draw family stays contiguous.
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.equal(order.indexOf(HATCH_POINT.id), order.indexOf('HATCH') + 1);
  assert.equal(order.indexOf('HATCHPOINT') < order.indexOf('MOVE'), true);

  // Router admission is the single gate, like every other command.
  assert.equal(isSupportedCommand('hatchpoint'), true);
  assert.equal(admitCommandText('HATCHPOINT', { status: loaded }).kind, 'admitted');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('HATCHPOINT'), false);
  assert.equal(admitCommandText('HATCHPOINT', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  // The active-tool chip key is what the host's `tool.key` maps onto.
  assert.equal(toolLabelKey(HATCH_POINT.toolKey), HATCH_POINT.toolLabelKey);
});

test('the chip names the one internal pick, then the captured state without claiming a boundary', () => {
  // Exactly one named step: it is a one-point capture, and the ready prompt is not
  // positional, so it lives in the ready table instead of the step list.
  assert.deepEqual(TOOL_STEP_PROMPTS[HATCH_POINT.toolKey], [HATCH_POINT.pick]);
  assert.equal(TOOL_READY_PROMPTS[HATCH_POINT.toolKey], HATCH_POINT.ready);
  // It takes a pick, so it is deliberately absent from the pre-selection table
  // (which is HATCH's zero-pick shape).
  assert.equal(PRESELECT_PROMPTS[HATCH_POINT.toolKey], undefined);

  // No point yet: ask for the internal point. The host's `ready` flag does not
  // change that — for a pick tool the count decides the step.
  assert.equal(toolStepPromptKey({ key: HATCH_POINT.toolKey, points: 0 }), HATCH_POINT.pick);
  assert.equal(toolStepPromptKey({ key: HATCH_POINT.toolKey, points: 0, ready: true }), HATCH_POINT.pick);
  // The point is captured but the boundary has not been searched yet.
  assert.equal(toolStepPromptKey({ key: HATCH_POINT.toolKey, points: 1 }), HATCH_POINT.ready);
  assert.equal(toolStepPromptKey({ key: HATCH_POINT.toolKey, points: 3 }), HATCH_POINT.ready);
  // A malformed count is read as "no points yet", never as a later step.
  assert.equal(toolStepPromptKey({ key: HATCH_POINT.toolKey, points: '1' }), HATCH_POINT.pick);
  // It never borrows the dimension ready copy (nothing is dimensioned).
  assert.notEqual(HATCH_POINT.ready, 'tool.prompt.dim.ready');
  assert.notEqual(HATCH_POINT.ready, 'tool.prompt.dim.center_ready');
});

test('both locales carry the HATCHPOINT copy and state the honest boundary', () => {
  for (const key of [HATCH_POINT.labelKey, HATCH_POINT.toolLabelKey, HATCH_POINT.pick, HATCH_POINT.ready]) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
  }
  // The button, the command line and the active-tool chip name the same tool.
  assert.equal(en[HATCH_POINT.labelKey], 'Hatch by point');
  assert.equal(en[HATCH_POINT.labelKey], en[HATCH_POINT.toolLabelKey]);
  assert.equal(zh[HATCH_POINT.labelKey], '按点填充');
  assert.equal(zh[HATCH_POINT.labelKey], zh[HATCH_POINT.toolLabelKey]);

  // The pick copy names the accepted boundaries and says which edges this command
  // does **not** find, so it can never imply the HATCH pre-selection sweep.
  assert.match(en[HATCH_POINT.pick], /point inside/i);
  assert.match(en[HATCH_POINT.pick], /circle/i);
  assert.match(en[HATCH_POINT.pick], /ellipse/i);
  assert.match(en[HATCH_POINT.pick], /polyline/i);
  assert.match(en[HATCH_POINT.pick], /LINE\/ARC/i);
  assert.match(en[HATCH_POINT.pick], /not found/i);
  assert.match(zh[HATCH_POINT.pick], /内部/);
  assert.match(zh[HATCH_POINT.pick], /圆/);
  assert.match(zh[HATCH_POINT.pick], /椭圆/);
  assert.match(zh[HATCH_POINT.pick], /多段线/);
  assert.match(zh[HATCH_POINT.pick], /直线\/圆弧/);
  assert.match(zh[HATCH_POINT.pick], /不会/);

  // The ready copy names the captured point and Enter; the host has not searched
  // for a boundary yet, so neither language may claim one was found or is ready.
  assert.match(en[HATCH_POINT.ready], /Internal point captured/);
  assert.match(en[HATCH_POINT.ready], /Enter/);
  assert.doesNotMatch(en[HATCH_POINT.ready], /boundary/i);
  assert.match(zh[HATCH_POINT.ready], /已拾取内部点/);
  assert.match(zh[HATCH_POINT.ready], /回车/);
  assert.doesNotMatch(zh[HATCH_POINT.ready], /边界/);
});

test('HATCHPOINT owns exactly HATCH\'s ten parameter rows under its own command', () => {
  // No new control: the point-pick entry reads the same host parameters, so the
  // two commands can never disagree about the fill they create.
  assert.deepEqual(TOOL_PARAMETERS[HATCH_POINT.toolKey], {
    command: HATCH_POINT.id,
    keys: TOOL_PARAMETERS['draw.hatch'].keys,
  });
  assert.deepEqual(TOOL_PARAMETERS['draw.hatch'].keys, [
    'hatchPatternType',
    'hatchPattern',
    'hatchGradientKind',
    'hatchGradientAngle',
    'hatchGradientColorA',
    'hatchGradientColorB',
    'hatchSpacing',
    'hatchAngle',
    'hatchDouble',
    'hatchStyle',
  ]);
  assert.notEqual(TOOL_PARAMETERS['draw.hatch'].command, TOOL_PARAMETERS[HATCH_POINT.toolKey].command);
});
