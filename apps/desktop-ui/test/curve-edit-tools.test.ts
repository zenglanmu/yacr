// The three single-curve edits added 2026-11-01: BREAK, DIVIDE and LENGTHEN.
//
// What this pins, and why it is one module: the two commands have **different
// capture shapes**, and almost every mistake here is a shape mistake.
//
//   * BREAK (`draw.break`) is a **two-pick** capture. The first click selects the
//     curve *and* is the first break point, so the tool reports one point per
//     click from the start; the second click is the other end of the removed
//     span. It is **not** a pre-selection tool.
//   * DIVIDE (`draw.divide`) takes **no pick at all**: the one preselected curve
//     is the whole input, so its chip is chosen by the host's readiness, never by
//     a point count. LENGTHEN (`draw.lengthen`) takes one pick for the static
//     modes (the end that moves) and two for `DY`; its settings are the shared
//     tool-parameter strip.
//
// The copy assertions only check presence and the honest distinction between the
// two shapes — this suite does not claim the backend implements the edits.
import test from 'node:test';
import assert from 'node:assert/strict';
import { PRESELECT_PROMPTS, TOOL_READY_PROMPTS, TOOL_STEP_PROMPTS, toolStepPromptKey } from '../src/cad/cad.ts';
import { TOOL_PARAMETER_SPECS, TOOL_PARAMETERS } from '../src/tools/tool-parameter-specs.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const BREAK = Object.freeze({
  id: 'BREAK',
  toolKey: 'draw.break',
  labelKey: 'draw.kind.break',
  toolLabelKey: 'tool.draw.break',
  icon: 'trim',
  selectEntity: 'tool.prompt.break.select_entity',
  secondPoint: 'tool.prompt.break.second_point',
  ready: 'tool.prompt.break.ready',
});

const DIVIDE = Object.freeze({
  id: 'DIVIDE',
  toolKey: 'draw.divide',
  labelKey: 'draw.kind.divide',
  toolLabelKey: 'tool.draw.divide',
  icon: 'point',
  idle: 'tool.prompt.divide.select_curve',
  ready: 'tool.prompt.divide.ready',
  param: 'divideSegments',
});

const LENGTHEN = Object.freeze({
  id: 'LENGTHEN',
  toolKey: 'draw.lengthen',
  labelKey: 'draw.kind.lengthen',
  toolLabelKey: 'tool.draw.lengthen',
  icon: 'scale',
  idle: 'tool.prompt.lengthen.select_curve',
  ready: 'tool.prompt.lengthen.ready',
  params: ['lengthenMode', 'lengthenValue'],
});

const panel = (tabId, panelId) => RIBBON_TABS
  .find(tab => tab.id === tabId)
  .panels.find(candidate => candidate.id === panelId);

const tool = (key, points, ready = false) => ({ key, points, ready });

// --- BREAK: two named picks, the first of which is the entity pick -----------

test('BREAK names exactly two picks, and the first one is also the first break point', () => {
  assert.deepEqual(TOOL_STEP_PROMPTS[BREAK.toolKey], [BREAK.selectEntity, BREAK.secondPoint]);
  assert.equal(TOOL_STEP_PROMPTS[BREAK.toolKey].length, 2, 'the list must stay at two entries');
  assert.equal(TOOL_READY_PROMPTS[BREAK.toolKey], BREAK.ready);
  // The two step sentences are different: one asks for the curve, the other for
  // the far end of the span. Reusing one string would hide the second click.
  assert.notEqual(BREAK.selectEntity, BREAK.secondPoint);
  // Point counts map one-to-one onto the list: 0 = curve pick (which already
  // places the first break point), 1 = second point, 2 = span complete.
  assert.equal(toolStepPromptKey(tool(BREAK.toolKey, 0)), BREAK.selectEntity);
  assert.equal(toolStepPromptKey(tool(BREAK.toolKey, 1)), BREAK.secondPoint);
  assert.equal(toolStepPromptKey(tool(BREAK.toolKey, 2, false)), BREAK.ready);
  assert.equal(toolStepPromptKey(tool(BREAK.toolKey, 2, true)), BREAK.ready);
  // A malformed count is treated as "no points yet", never as a later step.
  for (const points of [-1, 1.5, '1', null, undefined]) {
    assert.equal(toolStepPromptKey(tool(BREAK.toolKey, points)), BREAK.selectEntity, String(points));
  }
});

test('BREAK is a pick capture, never a pre-selection tool', () => {
  // A preselection entry would replace its pick prompts with select/ready copy,
  // which is exactly the state BREAK does not have: nothing is selected when it
  // starts, and every click places a break point.
  assert.equal(PRESELECT_PROMPTS[BREAK.toolKey], undefined);
  // Its ready copy is not the shared dimension sentence: nothing is dimensioned.
  assert.notEqual(TOOL_READY_PROMPTS[BREAK.toolKey], 'tool.prompt.dim.ready');
  // The `F` override is its one parameter: on, the first click only selects the
  // object and two more picks define the span; off, the plain two-click form.
  assert.deepEqual(TOOL_PARAMETERS[BREAK.toolKey], {
    command: 'BREAK',
    keys: ['breakFirstPoint'],
  });
});

// --- DIVIDE / LENGTHEN: zero picks, readiness decides the chip ---------------

test('DIVIDE is a pre-selection tool whose chip follows the host readiness only', () => {
  assert.deepEqual(PRESELECT_PROMPTS[DIVIDE.toolKey], { idle: DIVIDE.idle, ready: DIVIDE.ready });
  // It must not also be a step list: an empty list would report "ready" before
  // anything is selected, and a non-empty one would invent a pick.
  assert.equal(TOOL_STEP_PROMPTS[DIVIDE.toolKey], undefined);
  assert.equal(TOOL_READY_PROMPTS[DIVIDE.toolKey], undefined);
  // Readiness must be the literal `true`; anything else never claims the
  // selection is usable.
  assert.equal(toolStepPromptKey(tool(DIVIDE.toolKey, 0, false)), DIVIDE.idle);
  assert.equal(toolStepPromptKey(tool(DIVIDE.toolKey, 0, true)), DIVIDE.ready);
  for (const ready of [undefined, null, 1, 'true']) {
    assert.equal(toolStepPromptKey(tool(DIVIDE.toolKey, 0, ready)), DIVIDE.idle, String(ready));
  }
  // Points are irrelevant here: a stale count never changes the sentence.
  assert.equal(toolStepPromptKey(tool(DIVIDE.toolKey, 3, false)), DIVIDE.idle);
});

test('LENGTHEN names the nearest-endpoint pick for the static modes', () => {
  // Static DE/PER/TOT capture one pick (the end that moves); DY takes two. There
  // is no readiness-only pre-selection chip for LENGTHEN any more.
  assert.equal(
    TOOL_READY_PROMPTS[LENGTHEN.toolKey],
    'tool.prompt.lengthen.dynamic_ready',
  );
  assert.equal(
    toolStepPromptKey(tool(LENGTHEN.toolKey, 0, false)),
    'tool.prompt.lengthen.static_pick',
  );
  assert.equal(
    toolStepPromptKey(tool(LENGTHEN.toolKey, 0, true)),
    'tool.prompt.lengthen.static_pick',
    "readiness never skips the pick",
  );
  assert.equal(
    toolStepPromptKey(tool(LENGTHEN.toolKey, 1, true)),
    'tool.prompt.lengthen.ready',
  );
});

// --- the parameter rows ------------------------------------------------------

test('DIVIDE owns the segment count and its two block options; LENGTHEN owns the mode plus the amount', () => {
  assert.deepEqual(TOOL_PARAMETERS[DIVIDE.toolKey], {
    command: 'DIVIDE',
    keys: [DIVIDE.param, 'divideUseBlock', 'divideAlignBlock'],
  });
  assert.equal(TOOL_PARAMETER_SPECS.divideUseBlock.kind, 'flag');
  assert.equal(TOOL_PARAMETER_SPECS.divideUseBlock.statusKey, 'divideUseBlock');
  assert.equal(
    TOOL_PARAMETER_SPECS.divideUseBlock.labelKey,
    'status.tool_parameter.divide_use_block',
  );
  assert.equal(
    TOOL_PARAMETER_SPECS.divideUseBlock.hintKey,
    'status.tool_parameter.divide_use_block.hint',
  );
  assert.equal(TOOL_PARAMETER_SPECS.divideAlignBlock.kind, 'flag');
  assert.equal(TOOL_PARAMETER_SPECS.divideAlignBlock.statusKey, 'divideAlignBlock');
  assert.equal(
    TOOL_PARAMETER_SPECS.divideAlignBlock.labelKey,
    'status.tool_parameter.divide_align_block',
  );
  assert.equal(
    TOOL_PARAMETER_SPECS.divideAlignBlock.hintKey,
    'status.tool_parameter.divide_align_block.hint',
  );
  assert.equal(TOOL_PARAMETER_SPECS.breakFirstPoint.kind, 'flag');
  assert.equal(TOOL_PARAMETER_SPECS.breakFirstPoint.statusKey, 'breakFirstPoint');
  assert.equal(
    TOOL_PARAMETER_SPECS.breakFirstPoint.labelKey,
    'status.tool_parameter.break_first_point',
  );
  assert.deepEqual(TOOL_PARAMETERS[LENGTHEN.toolKey], { command: 'LENGTHEN', keys: LENGTHEN.params });

  const segments = TOOL_PARAMETER_SPECS.divideSegments;
  assert.equal(segments.kind, 'number');
  assert.equal(segments.statusKey, 'divideSegments');
  assert.equal(segments.step, 1);
  // The host's closed range 2..=1024, integers only — a value outside it is
  // refused, never clamped.
  for (const value of [2, 3, 17, 1024]) assert.equal(segments.offered(value), true, String(value));
  for (const value of [0, 1, 1025, 4.5, NaN, Infinity]) assert.equal(segments.offered(value), false, String(value));
  assert.equal(segments.choices, undefined, 'a count is not a closed set');
  assert.equal(segments.choiceLabelKeys, undefined);
  assert.equal(segments.labelKey, 'status.tool_parameter.divide_segments');
  assert.equal(segments.hintKey, 'status.tool_parameter.divide_segments.hint');

  const mode = TOOL_PARAMETER_SPECS.lengthenMode;
  assert.equal(mode.kind, 'number');
  assert.equal(mode.statusKey, 'lengthenMode');
  assert.equal(mode.step, 1);
  assert.deepEqual([...mode.choices], [0, 1, 2, 3]);
  for (const value of [0, 1, 2, 3]) assert.equal(mode.offered(value), true, String(value));
  for (const value of [-1, 4, 0.5, NaN, Infinity]) assert.equal(mode.offered(value), false, String(value));
  // The three codes are read by name, so a raw 0/1/2 stepper never appears.
  assert.deepEqual({ ...mode.choiceLabelKeys }, {
    0: 'status.tool_parameter.lengthen_mode.delta',
    1: 'status.tool_parameter.lengthen_mode.percent',
    2: 'status.tool_parameter.lengthen_mode.total',
    3: 'status.tool_parameter.lengthen_mode.dynamic',
  });
  assert.equal(mode.labelKey, 'status.tool_parameter.lengthen_mode');
  assert.equal(mode.hintKey, 'status.tool_parameter.lengthen_mode.hint');

  const value = TOOL_PARAMETER_SPECS.lengthenValue;
  assert.equal(value.kind, 'number');
  assert.equal(value.statusKey, 'lengthenValue');
  assert.equal(value.step, 1);
  // Finite and non-zero, both signs: Delta may shorten as well as lengthen.
  for (const number of [-5, -0.5, 0.5, 12]) assert.equal(value.offered(number), true, String(number));
  for (const number of [0, -0, NaN, Infinity, -Infinity]) assert.equal(value.offered(number), false, String(number));
  assert.equal(value.choices, undefined);
  assert.equal(value.labelKey, 'status.tool_parameter.lengthen_value');
  assert.equal(value.hintKey, 'status.tool_parameter.lengthen_value.hint');
});

// --- catalog + ribbon --------------------------------------------------------

test('all three are catalogued as modify commands next to TRIM/EXTEND', () => {
  const order = COMMAND_CATALOG.map(command => command.id);
  for (const entry of [BREAK, DIVIDE, LENGTHEN]) {
    const row = COMMAND_CATALOG.find(command => command.id === entry.id);
    assert.deepEqual({ ...row }, { id: entry.id, labelKey: entry.labelKey, icon: entry.icon, group: 'modify' });
    assert.equal(order.indexOf(entry.id), order.indexOf('EXTEND') + 1 + [BREAK, DIVIDE, LENGTHEN].indexOf(entry), entry.id);
    assert.ok(order.indexOf(entry.id) < order.indexOf('ROTATE'), `${entry.id} stays inside the trim/extend run`);
    assert.ok(order.indexOf(entry.id) < order.indexOf('JOIN'), 'JOIN still closes the modify listing');
  }
});

test('both modify panels carry the three as small router buttons with no popover', () => {
  for (const tabId of ['home', 'draw']) {
    const modify = panel(tabId, 'modify');
    assert.ok(modify, `${tabId} keeps its modify panel`);
    const ids = modify.items.map(item => item.id);
    for (const entry of [BREAK, DIVIDE, LENGTHEN]) {
      const item = modify.items.find(candidate => candidate.id === entry.id);
      assert.ok(item, `${tabId}/modify must carry ${entry.id}`);
      assert.equal(item.command, entry.id, 'the dispatch token is the router name');
      assert.equal(item.labelKey, entry.labelKey);
      assert.equal(item.icon, entry.icon);
      assert.equal(item.large, undefined, `${entry.id} is a small button`);
      assert.equal(item.variants, undefined, `${entry.id} has no settings popover`);
      assert.equal(item.action, undefined, `${entry.id} must not bypass the router`);
      assert.equal(modify.items.filter(candidate => candidate.id === entry.id).length, 1, `${tabId}: one ${entry.id}`);
      // Directly after EXTEND, in the task's own order, so both panels match.
      assert.equal(ids.indexOf(entry.id), ids.indexOf('EXTEND') + 1 + [BREAK, DIVIDE, LENGTHEN].indexOf(entry), entry.id);
    }
    // The shipped JOIN → SELECTSIMILAR tail keeps its relative order; the
    // 2026-11-07 modify additions are appended after it.
    const joinIndex = ids.indexOf('JOIN');
    assert.ok(joinIndex >= 0, `${tabId}: JOIN is present`);
    assert.equal(ids[joinIndex + 1], 'SELECTSIMILAR', `${tabId}: SELECTSIMILAR is appended after JOIN`);
  }
});

test('the borrowed glyphs exist and no panel elsewhere dispatches these commands', async () => {
  const icons = await readModule('icons.tsx');
  const map = icons.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  const defined = new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
  for (const entry of [BREAK, DIVIDE, LENGTHEN]) {
    assert.ok(defined.has(entry.icon), `icons.tsx must define '${entry.icon}'`);
    for (const tab of RIBBON_TABS) {
      for (const each of tab.panels) {
        if (each.id === 'modify') continue;
        assert.equal(each.items.some(item => item.command === entry.id), false, `${tab.id}/${each.id}: ${entry.id}`);
      }
    }
  }
});

// --- copy -------------------------------------------------------------------

test('the catalog carries both locales for every label, prompt and parameter row', () => {
  const keys = [
    BREAK.labelKey, BREAK.toolLabelKey, BREAK.selectEntity, BREAK.secondPoint, BREAK.ready,
    DIVIDE.labelKey, DIVIDE.toolLabelKey, DIVIDE.idle, DIVIDE.ready,
    LENGTHEN.labelKey, LENGTHEN.toolLabelKey, LENGTHEN.idle, LENGTHEN.ready,
    'status.tool_parameter.divide_segments',
    'status.tool_parameter.divide_segments.hint',
    'status.tool_parameter.lengthen_mode',
    'status.tool_parameter.lengthen_mode.hint',
    'status.tool_parameter.lengthen_mode.delta',
    'status.tool_parameter.lengthen_mode.percent',
    'status.tool_parameter.lengthen_mode.total',
    'status.tool_parameter.lengthen_mode.dynamic',
    'status.tool_parameter.lengthen_value',
    'status.tool_parameter.lengthen_value.hint',
    'status.tool_parameter.divide_use_block',
    'status.tool_parameter.divide_use_block.hint',
    'status.tool_parameter.break_first_point',
    'status.tool_parameter.break_first_point.hint',
    'status.tool_parameter.align_pairs',
    'status.tool_parameter.align_pairs.hint',
    'status.tool_parameter.align_pairs.single',
    'status.tool_parameter.align_pairs.two',
    'status.tool_parameter.align_pairs.three',
    'tool.prompt.align.source_1',
    'tool.prompt.align.dest_1',
    'tool.prompt.align.source_2',
    'tool.prompt.align.dest_2',
    'tool.prompt.align.source_3',
    'tool.prompt.align.dest_3',
    'tool.prompt.align.ready',
    'tool.prompt.break.first_point',
    'tool.prompt.lengthen.dynamic_pick',
    'tool.prompt.lengthen.dynamic_new_end',
    'tool.prompt.lengthen.dynamic_ready',
    'tool.prompt.divide.type_block',
  ];
  for (const key of keys) {
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.ok(en[key].trim().length > 0, `en ${key} is empty`);
    assert.ok(zh[key].trim().length > 0, `zh-CN ${key} is empty`);
  }
  // The ribbon label and the active-tool chip name the same command, in both
  // locales — so the chip can never disagree with the button that started it.
  for (const entry of [BREAK, DIVIDE, LENGTHEN]) {
    assert.equal(en[entry.labelKey], en[entry.toolLabelKey], `${entry.id} (en)`);
    assert.equal(zh[entry.labelKey], zh[entry.toolLabelKey], `${entry.id} (zh-CN)`);
  }
  // The three lengthen modes are three distinct names, never one string thrice.
  const modes = ['delta', 'percent', 'total', 'dynamic'].map(name => `status.tool_parameter.lengthen_mode.${name}`);
  assert.equal(new Set(modes.map(key => en[key])).size, 4);
  assert.equal(new Set(modes.map(key => zh[key])).size, 4);
  // No copy names a dimension or a finish verb these commands do not have.
  for (const key of [BREAK.selectEntity, BREAK.secondPoint, BREAK.ready, DIVIDE.idle, DIVIDE.ready, LENGTHEN.idle, LENGTHEN.ready]) {
    assert.equal(/dimension/i.test(en[key]), false, `${key} must not name a dimension`);
  }
});

// --- parameter-selected capture shapes (2026-11-09) --------------------------

test('the parameter-selected shapes name their own picks instead of a fixed list', () => {
  const withParams = (key, points, ready, params) => ({ key, points, ready, params });

  // ALIGN's pair count slices the six-prompt list: 1 pair asks for source/dest,
  // 2 pairs stops before the third pair, and 3 pairs asks for all six.
  assert.equal(
    toolStepPromptKey(withParams('draw.align', 0, false, { alignPairs: 1 })),
    'tool.prompt.align.source_1',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.align', 1, false, { alignPairs: 1 })),
    'tool.prompt.align.dest_1',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.align', 2, true, { alignPairs: 1 })),
    'tool.prompt.align.ready',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.align', 4, false, { alignPairs: 2 })),
    'tool.prompt.align.ready',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.align', 4, false, { alignPairs: 3 })),
    'tool.prompt.align.source_3',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.align', 5, false, { alignPairs: 3 })),
    'tool.prompt.align.dest_3',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.align', 6, true, { alignPairs: 3 })),
    'tool.prompt.align.ready',
  );

  // BREAK's `F` form: the first click only selects the object.
  assert.equal(
    toolStepPromptKey(withParams('draw.break', 0, false, { breakFirstPoint: true })),
    'tool.prompt.break.select_entity',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.break', 1, false, { breakFirstPoint: true })),
    'tool.prompt.break.first_point',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.break', 2, false, { breakFirstPoint: true })),
    'tool.prompt.break.second_point',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.break', 3, true, { breakFirstPoint: true })),
    'tool.prompt.break.ready',
  );
  // Off keeps the plain two-click list (the first pick is the first break point).
  assert.equal(
    toolStepPromptKey(withParams('draw.break', 0, false, { breakFirstPoint: false })),
    'tool.prompt.break.select_entity',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.break', 1, false, { breakFirstPoint: false })),
    'tool.prompt.break.second_point',
  );

  // LENGTHEN's dynamic mode replaces the pre-selection chip with two picks.
  assert.equal(
    toolStepPromptKey(withParams('draw.lengthen', 0, false, { lengthenMode: 3 })),
    'tool.prompt.lengthen.dynamic_pick',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.lengthen', 1, false, { lengthenMode: 3 })),
    'tool.prompt.lengthen.dynamic_new_end',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.lengthen', 2, true, { lengthenMode: 3 })),
    'tool.prompt.lengthen.dynamic_ready',
  );
  // A static mode now takes one pick (the end that moves) and then shows the
  // ready copy.
  assert.equal(
    toolStepPromptKey(withParams('draw.lengthen', 0, false, { lengthenMode: 2 })),
    'tool.prompt.lengthen.static_pick',
  );
  assert.equal(
    toolStepPromptKey(withParams('draw.lengthen', 1, false, { lengthenMode: 2 })),
    'tool.prompt.lengthen.ready',
  );

  // DIVIDE's block option is a text step; the host's `awaiting_text` names it.
  assert.equal(
    toolStepPromptKey(withParams('draw.divide', 0, false, { divideUseBlock: false })),
    'tool.prompt.divide.select_curve',
  );
  assert.equal(
    toolStepPromptKey({ ...withParams('draw.divide', 0, false, { divideUseBlock: true }), awaiting_text: true }),
    'tool.prompt.divide.type_block',
  );
});
