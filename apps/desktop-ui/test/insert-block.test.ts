// INSERT ("Insert block") user-facing surface contract (2026-11-01).
//
// INSERT places a reference to a block that **already exists** in the drawing.
// It is a **point-then-name** capture: one pick sets the insertion point, then
// the block name is the terminal text step on `cad_set_tool_text` (the host
// commits after the text). v1 has **no block list or palette to pick from** and
// no scale/rotation/attribute controls, so the copy says the list is missing
// instead of implying a picker, and nothing here offers a variant popover.
//
// Nothing here infers a step from the tool key: the text mode is the host's
// `awaiting_text` flag alone, and the one pick prompt comes from the real
// captured count. Every user-visible string is a catalog key.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  INSERT_BLOCK_PICK_PROMPT_KEY,
  INSERT_BLOCK_TEXT_PROMPT_KEY,
  TEXT_STEP_PROMPT_KEY,
  textStepPromptKey,
} from '../src/tools/text.ts';
import { PRESELECT_PROMPTS, TOOL_READY_PROMPTS, TOOL_STEP_PROMPTS, toolStepPromptKey } from '../src/cad/cad.ts';
import {
  admitCommandText,
  filterCommandSuggestions,
  isSupportedCommand,
  setCommandCatalog,
  toolLabelKey,
} from '../src/commands/commands.ts';
import { CAPABILITY_GATES, GESTURE_ONLY_COMMANDS } from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };

// The frozen surface names, written out here so a rename cannot pass silently.
// They are the Rust lane's machine names: selector `INSERT`, tool key
// `draw.insertblock`, catalog label `draw.kind.insertblock`.
const INSERT = Object.freeze({
  id: 'INSERT',
  labelKey: 'draw.kind.insertblock',
  icon: 'copy',
  toolKey: 'draw.insertblock',
  toolLabelKey: 'tool.draw.insertblock',
  pick: 'tool.prompt.insertblock.pick_point',
  text: 'tool.prompt.insertblock.type_name',
});

const TOOL = points => ({ key: INSERT.toolKey, points, ready: false });
const AWAITING = points => ({ ...TOOL(points), awaiting_text: true });

const drawPanel = tabId => RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'draw');

test('INSERT is one small draw-panel button on both tabs, with no variant popover', () => {
  for (const tabId of ['home', 'draw']) {
    const items = drawPanel(tabId).items;
    const ids = items.map(item => item.id);
    const at = ids.indexOf(INSERT.id);
    assert.ok(at !== -1, `${tabId}/draw must carry the INSERT affordance`);
    // INSERT sits at the tail of the creation run, directly after SPLINE, with
    // the block **definition** command BLOCK beside it before OFFSET, so no
    // shipped row moves and the HATCH/HATCHPOINT tail keeps its slot.
    assert.equal(ids[at - 1], 'SPLINE', `${tabId}: INSERT follows SPLINE`);
    assert.equal(ids[at + 1], 'BLOCK', `${tabId}: INSERT precedes BLOCK`);
    assert.equal(ids[at + 2], 'OFFSET', `${tabId}: BLOCK precedes OFFSET`);
    assert.deepEqual(
      ids.slice(-3),
      ['HATCH', 'HATCHPOINT', 'REVCLOUD'],
      `${tabId}: the HATCH pair and REVCLOUD close the draw group`,
    );
    assert.equal(items.filter(item => item.id === INSERT.id).length, 1, `${tabId}: one INSERT button`);

    const item = items[at];
    assert.equal(item.command, INSERT.id);
    assert.equal(item.labelKey, INSERT.labelKey);
    assert.equal(item.icon, INSERT.icon);
    assert.equal(item.action, undefined, 'INSERT must dispatch through the router');
    assert.equal(item.standardView, undefined);
    assert.equal(item.large, undefined, `${tabId}: INSERT is a small draw button, not a primary tile`);
    // No block picker, no scale/rotation/attribute controls: a caret would imply
    // choices the command does not have.
    assert.equal(item.variants, undefined, `${tabId}: INSERT has no variant popover`);
    assert.ok(ribbonCommandIds().includes(INSERT.id), `${tabId}: the ribbon reaches INSERT`);
  }
  // No other panel smuggles the command in.
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      if (panel.id === 'draw') continue;
      assert.equal(panel.items.some(item => item.command === INSERT.id), false, `${tab.id}/${panel.id}`);
    }
  }
});

test('INSERT is catalogued as a draw command enabled only by router admission', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === INSERT.id);
  assert.ok(entry, 'INSERT must be in the command catalog');
  assert.deepEqual(
    { ...entry },
    { id: INSERT.id, labelKey: INSERT.labelKey, icon: INSERT.icon, group: 'draw' },
  );
  // The listing ends the draw creation run with it, before the modify family.
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.equal(order.indexOf(INSERT.id), order.indexOf('SPLINE') + 1);
  assert.ok(order.indexOf(INSERT.id) < order.indexOf('MOVE'));
  assert.equal(COMMAND_CATALOG.filter(command => command.id === INSERT.id).length, 1);

  // Router admission is the single gate — the same as every other draw command.
  assert.equal(isSupportedCommand(INSERT.id), true);
  assert.equal(isSupportedCommand('insert'), true);
  assert.equal(admitCommandText(INSERT.id, { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText(INSERT.id, { status: null }).kind, 'unavailable');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand(INSERT.id), false);
  assert.equal(admitCommandText(INSERT.id, { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  // Neither capability-gated nor gesture-only.
  assert.equal(CAPABILITY_GATES[INSERT.id], undefined);
  assert.equal(GESTURE_ONLY_COMMANDS.includes(INSERT.id), false);
  // The command line offers it with the same admission state and catalog label.
  const suggestion = filterCommandSuggestions(INSERT.id).find(candidate => candidate.id === INSERT.id);
  assert.ok(suggestion);
  assert.equal(suggestion.supported, true);
  assert.equal(suggestion.labelKey, INSERT.labelKey);
  // The active-tool chip key is what the host's `tool.key` maps onto.
  assert.equal(toolLabelKey(INSERT.toolKey), INSERT.toolLabelKey);
});

test('one pick, then the block name: the point step and the terminal text step', () => {
  // Exactly one point-index prompt: the insertion point. A second entry would
  // claim a pick the command does not take (there is no scale base, rotation
  // point or attribute prompt in v1), and an empty list would report "ready"
  // before anything was picked.
  assert.deepEqual(TOOL_STEP_PROMPTS[INSERT.toolKey], [INSERT.pick]);
  assert.equal(TOOL_STEP_PROMPTS[INSERT.toolKey].length, 1);
  assert.equal(INSERT_BLOCK_PICK_PROMPT_KEY, INSERT.pick);
  assert.equal(toolStepPromptKey(TOOL(0)), INSERT.pick);
  // A malformed/absent count is read as "no points yet" — never a later step.
  assert.equal(toolStepPromptKey({ key: INSERT.toolKey }), INSERT.pick);
  assert.equal(toolStepPromptKey({ key: INSERT.toolKey, points: '1' }), INSERT.pick);
  assert.equal(toolStepPromptKey({ key: INSERT.toolKey, points: -1 }), INSERT.pick);

  // The text step is **not** a pick: it is named from the host's `awaiting_text`
  // flag alone, whatever the captured count says.
  assert.equal(toolStepPromptKey(AWAITING(0)), INSERT.text);
  assert.equal(toolStepPromptKey(AWAITING(1)), INSERT.text);
  assert.equal(toolStepPromptKey(AWAITING(7)), INSERT.text);
  assert.equal(textStepPromptKey({ key: INSERT.toolKey }), INSERT.text);
  assert.equal(INSERT_BLOCK_TEXT_PROMPT_KEY, INSERT.text);
  assert.equal(TOOL_STEP_PROMPTS[INSERT.toolKey].includes(INSERT.text), false);

  // Between the pick and the flag the next step is the name, so the chip must not
  // borrow the shared dimension copy or claim a block was found/accepted.
  assert.equal(TOOL_READY_PROMPTS[INSERT.toolKey], INSERT.text);
  assert.equal(toolStepPromptKey(TOOL(1)), INSERT.text);
  assert.equal(toolStepPromptKey(TOOL(9)), INSERT.text);
  for (const tool of [TOOL(1), TOOL(9), AWAITING(0), AWAITING(1)]) {
    assert.notEqual(toolStepPromptKey(tool), 'tool.prompt.dim.ready');
    assert.notEqual(toolStepPromptKey(tool), 'tool.prompt.dim.center_ready');
  }

  // It takes a point, so it is **not** a pre-selection tool: a preselection entry
  // would hide the pick behind "select objects first".
  assert.equal(PRESELECT_PROMPTS[INSERT.toolKey], undefined);

  // The neighbouring text-carrying tools keep their own derivations.
  assert.equal(textStepPromptKey({ key: 'draw.text' }), TEXT_STEP_PROMPT_KEY);
  assert.equal(textStepPromptKey({ key: 'draw.mtext' }), 'tool.prompt.mtext.type_text');
  assert.equal(toolStepPromptKey({ key: 'draw.line', points: 1 }), null);
});

test('both locales carry the INSERT copy, and it states the missing block list', () => {
  for (const [name, catalog] of [['zh-CN', zh], ['en', en]]) {
    for (const key of [INSERT.labelKey, INSERT.toolLabelKey, INSERT.pick, INSERT.text]) {
      assert.equal(typeof catalog[key], 'string', `${name} missing ${key}`);
      assert.ok(catalog[key].trim().length > 0, `${name} has empty ${key}`);
    }
    // The catalog label and the active-tool chip name the same thing.
    assert.equal(catalog[INSERT.labelKey], catalog[INSERT.toolLabelKey], name);
  }
  // The pick prompt names a real point, the text prompt names a **block name**;
  // neither may be the shared text/dimension sentence, which would describe a
  // different step.
  assert.equal(en[INSERT.pick].includes('insertion point'), true, en[INSERT.pick]);
  assert.equal(zh[INSERT.pick].includes('插入点'), true, zh[INSERT.pick]);
  assert.notEqual(en[INSERT.text], en[TEXT_STEP_PROMPT_KEY]);
  assert.notEqual(zh[INSERT.text], zh[TEXT_STEP_PROMPT_KEY]);
  // Grounded, literal wording: the missing block list is spelled out in both
  // languages rather than implied away.
  assert.equal(en[INSERT.text].includes('block'), true, en[INSERT.text]);
  assert.equal(en[INSERT.text].includes('not implemented'), true, en[INSERT.text]);
  assert.equal(zh[INSERT.text].includes('块'), true, zh[INSERT.text]);
  assert.equal(zh[INSERT.text].includes('尚未实现'), true, zh[INSERT.text]);
  // The labels stay the short command names, not the prompt sentences.
  assert.notEqual(en[INSERT.labelKey], en[INSERT.text]);
  assert.notEqual(zh[INSERT.labelKey], zh[INSERT.text]);
});
