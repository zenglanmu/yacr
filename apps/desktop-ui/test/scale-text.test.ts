// SCALETEXT (2026-11-01): the selection-based text scaler.
//
// It is a **zero-pick**, selection-only tool: one or more Text/MTEXT objects are
// the whole input and the single `scaleTextFactor` row is the only setting, so
// its chip follows the host readiness instead of a captured point count. This
// suite pins the whole UI surface the desktop lane owns — catalogue row, ribbon
// button, prompt pair, parameter spec/key list and both-locale copy — and the
// host's value rule: finite, strictly above 0, and **1.0 is never offered**
// because it is a no-op the host refuses by name.
//
// It does not claim the Rust command exists or works: `HOST_NAMES` is the local
// mirror of the router vocabulary, and the button's real enable state is the
// host's `canCommand`.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  PRESELECT_PROMPTS,
  TOOL_READY_PROMPTS,
  TOOL_STEP_PROMPTS,
  toolStepPromptKey,
} from '../src/cad/cad.ts';
import { setToolParameterNumber, toolParameterViews } from '../src/tools/tool-parameters.ts';
import { TOOL_PARAMETER_SPECS, TOOL_PARAMETERS } from '../src/tools/tool-parameter-specs.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { isSupportedCommand, setCommandCatalog } from '../src/commands/commands.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const ID = 'SCALETEXT';
const TOOL_KEY = 'draw.scaletext';
const LABEL_KEY = 'draw.kind.scaletext';
const TOOL_LABEL_KEY = 'tool.draw.scaletext';
const IDLE = 'tool.prompt.scaletext.select_text';
const READY = 'tool.prompt.scaletext.ready';
const PARAM = 'scaleTextFactor';
const PARAM_LABEL = 'status.tool_parameter.scale_text_factor';
const PARAM_HINT = 'status.tool_parameter.scale_text_factor.hint';

const drawPanel = tabId => RIBBON_TABS
  .find(tab => tab.id === tabId)
  .panels.find(panel => panel.id === 'draw');

const tool = (points, ready = false) => ({ key: TOOL_KEY, points, ready });
const status = value => ({
  tool: { key: TOOL_KEY, points: 0, ready: true, params: { [PARAM]: value } },
});
const rows = (value, gate = () => true) => toolParameterViews(status(value), gate);

// --- catalogue ---------------------------------------------------------------

test('SCALETEXT is a draw-group catalogue row beside the text commands', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === ID);
  assert.ok(entry, 'SCALETEXT must be in the command catalog');
  assert.deepEqual({ ...entry }, {
    id: ID,
    labelKey: LABEL_KEY,
    icon: 'scale',
    group: 'draw',
  });
  // It sits directly after MTEXT (the objects it edits) and before MLEADER, so
  // the text run reads TEXT → MTEXT → SCALETEXT → MLEADER.
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.equal(order.indexOf(ID), order.indexOf('MTEXT') + 1);
  assert.equal(order.indexOf('MLEADER'), order.indexOf(ID) + 1);
  // The shared `scale` glyph is a real icon name, not a made-up mark.
  assert.equal(COMMAND_CATALOG.filter(command => command.id === ID).length, 1);
});

// --- ribbon ------------------------------------------------------------------

test('both draw panels carry SCALETEXT as one small router button with no popover', () => {
  for (const tabId of ['home', 'draw']) {
    const items = drawPanel(tabId).items;
    const at = items.findIndex(item => item.id === ID);
    assert.ok(at !== -1, `${tabId}/draw must carry the SCALETEXT affordance`);
    assert.equal(items[at - 1].id, 'MTEXT', `${tabId}: SCALETEXT sits directly after MTEXT`);
    const item = items[at];
    assert.equal(item.command, ID, 'the dispatch token is the router name');
    assert.equal(item.labelKey, LABEL_KEY);
    assert.equal(item.icon, 'scale');
    assert.equal(item.large, undefined, `${tabId}: SCALETEXT is a small button`);
    assert.equal(item.variants, undefined, `${tabId}: there is no options popover`);
    assert.equal(item.action, undefined, 'SCALETEXT must dispatch through the router');
    assert.equal(item.standardView, undefined);
    assert.equal(items.filter(candidate => candidate.id === ID).length, 1, `${tabId}: one SCALETEXT`);
    // The HATCH pair still closes the draw group.
    assert.deepEqual(
      items.slice(-4).map(candidate => candidate.id),
      ['HATCH', 'HATCHPOINT', 'REVCLOUD', 'GRADIENT'],
      `${tabId}/draw ends with the HATCH pair and REVCLOUD`,
    );
  }
  // No other panel smuggles the command in.
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      if (panel.id === 'draw') continue;
      assert.equal(panel.items.some(item => item.command === ID), false, `${tab.id}/${panel.id}`);
    }
  }
  assert.ok(ribbonCommandIds().includes(ID), 'the ribbon reaches SCALETEXT');
});

// --- prompts -----------------------------------------------------------------

test('SCALETEXT is a pre-selection tool whose chip follows the host readiness only', () => {
  assert.deepEqual(PRESELECT_PROMPTS[TOOL_KEY], { idle: IDLE, ready: READY });
  // It must not also be a step list: an empty list would report "ready" before
  // anything is selected, and a non-empty one would invent a pick it never takes.
  assert.equal(TOOL_STEP_PROMPTS[TOOL_KEY], undefined);
  assert.equal(TOOL_READY_PROMPTS[TOOL_KEY], undefined);
  // Readiness must be the literal `true`; anything else never claims the
  // selection is usable.
  assert.equal(toolStepPromptKey(tool(0, false)), IDLE);
  assert.equal(toolStepPromptKey(tool(0, true)), READY);
  for (const ready of [undefined, null, 1, 'true']) {
    assert.equal(toolStepPromptKey(tool(0, ready)), IDLE, String(ready));
  }
  // Points are irrelevant here: a stale count never changes the sentence.
  assert.equal(toolStepPromptKey(tool(7, false)), IDLE);
  assert.equal(toolStepPromptKey(tool(7, true)), READY);
  // An absent tool never becomes this tool by accident.
  assert.equal(toolStepPromptKey(null), null);
  assert.equal(toolStepPromptKey({ key: 'draw.line', points: 0, ready: true }), null);
});

// --- the parameter -----------------------------------------------------------

test('SCALETEXT owns exactly one factor row and never offers the 1.0 no-op', async () => {
  assert.deepEqual(TOOL_PARAMETERS[TOOL_KEY], { command: ID, keys: [PARAM] });

  const spec = TOOL_PARAMETER_SPECS[PARAM];
  assert.equal(spec.kind, 'number');
  assert.equal(spec.statusKey, PARAM);
  assert.equal(spec.step, 0.1);
  assert.equal(spec.labelKey, PARAM_LABEL);
  assert.equal(spec.hintKey, PARAM_HINT);
  assert.equal(spec.typed, undefined, 'the factor is stepped, not freely typed');
  // The host's rule: finite and strictly above 0, minus the 1.0 no-op it refuses.
  for (const value of [0.1, 0.5, 0.9, 1.1, 1.5, 2, 10]) {
    assert.equal(spec.offered(value), true, String(value));
  }
  for (const refused of [1, 0, -0, -1, NaN, Infinity, -Infinity, '2', null, undefined]) {
    assert.equal(spec.offered(refused), false, String(refused));
  }
  // 1.0 is the default and must still render (the row is the only setting), but
  // no step may land on it as a target.
  const atDefault = rows(1)[0];
  assert.equal(atDefault.key, PARAM);
  assert.equal(atDefault.value, 1);
  assert.equal(atDefault.text, '1');
  assert.equal(atDefault.step, 0.1);
  assert.equal(atDefault.typed, false);
  assert.deepEqual([atDefault.decrement, atDefault.canDecrement], [0.9, true]);
  assert.deepEqual([atDefault.increment, atDefault.canIncrement], [1.1, true]);
  // A projected scale renders at the host's value; the gate is SCALETEXT itself.
  const asked = [];
  const row = rows(1.5, command => { asked.push(command); return true; })[0];
  assert.equal(row.value, 1.5);
  assert.equal(row.text, '1.5');
  assert.deepEqual(asked, [ID]);
  assert.equal(rows(1.5, () => false)[0].disabled, true);
  // Anything the host would refuse renders no row at all, never a guessed value.
  for (const refused of [0, -1, NaN, Infinity, null, '2']) {
    assert.deepEqual(rows(refused), [], String(refused));
  }
  // The write path uses the same predicate, so 1.0 can never reach IPC.
  for (const refused of [1, 0, -1, NaN, Infinity, '1.5', null, undefined]) {
    const result = await setToolParameterNumber(
      () => assert.fail('a refused factor reached IPC'), PARAM, refused,
    );
    assert.equal(result.kind, 'refused', String(refused));
  }
});

// --- router mirror -----------------------------------------------------------

test('the vocabulary mirror carries SCALETEXT, so the button can be admitted', () => {
  setCommandCatalog(HOST_NAMES);
  assert.equal(isSupportedCommand(ID), true);
  setCommandCatalog([]);
  assert.equal(isSupportedCommand(ID), false, 'an empty host vocabulary admits nothing');
  setCommandCatalog(HOST_NAMES);
});

// --- copy --------------------------------------------------------------------

test('the catalogue, prompts and parameter copy exist in both locales', () => {
  const keys = [LABEL_KEY, TOOL_LABEL_KEY, IDLE, READY, PARAM_LABEL, PARAM_HINT];
  for (const key of keys) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
  }
  // The button and the active-tool chip say the same word.
  assert.equal(en[LABEL_KEY], en[TOOL_LABEL_KEY]);
  assert.equal(zh[LABEL_KEY], zh[TOOL_LABEL_KEY]);
  // The task's exact copy, pinned so a paraphrase is a visible change.
  assert.equal(en[LABEL_KEY], 'Scale text');
  assert.equal(zh[LABEL_KEY], '缩放文字');
  assert.equal(en[IDLE], 'Select one or more Text or MText objects to scale');
  assert.equal(zh[IDLE], '选择要缩放的文字或多行文字对象');
  assert.equal(en[READY], 'Text selected; set the scale factor and press Enter to scale');
  assert.equal(zh[READY], '已选择文字；设置缩放比例后按回车');
  // The idle and ready sentences name different states.
  assert.notEqual(en[IDLE], en[READY]);
  assert.notEqual(zh[IDLE], zh[READY]);
  // The hint states the value rule, the effect and the refused no-op.
  for (const locale of [zh, en]) {
    assert.match(locale[PARAM_HINT], /1\.0/);
    assert.match(locale[PARAM_HINT], /0/);
  }
  assert.match(en[PARAM_HINT], /greater than 0/i);
  assert.match(en[PARAM_HINT], /glyph height/i);
  assert.match(en[PARAM_HINT], /wrap width/i);
  assert.match(en[PARAM_HINT], /no-op|no change|refuses/i);
  assert.match(zh[PARAM_HINT], /大于 0/);
  assert.match(zh[PARAM_HINT], /字高/);
  assert.match(zh[PARAM_HINT], /换行宽度/);
  assert.match(zh[PARAM_HINT], /拒绝/);
});
