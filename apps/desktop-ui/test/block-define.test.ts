// BLOCK ("Create block") user-facing surface contract (2026-11-01).
//
// BLOCK turns the **current selection** into a new block definition and replaces
// it with a block reference placed at the picked base point. The flow is
// selection already made → pick base point → type a new block name (the terminal
// text step on `cad_set_tool_text`). There is no block picker, no attribute or
// dynamic-block option, and no retain/delete choice — the selected objects move
// into the new block — so the copy says exactly that and nothing offers a variant
// popover.
//
// Nothing here infers a step from the tool key: the text mode is the host's
// `awaiting_text` flag alone, and the one pick prompt comes from the real
// captured count. Every user-visible string is a catalog key; `HOST_NAMES` is the
// local mirror of the router vocabulary, not proof the Rust command exists.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  BLOCK_PICK_PROMPT_KEY,
  BLOCK_TEXT_PROMPT_KEY,
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
// They are the Rust lane's machine names: selector `BLOCK`, tool key
// `draw.blockdefine`, catalog label `draw.kind.blockdefine`.
const BLOCK = Object.freeze({
  id: 'BLOCK',
  labelKey: 'draw.kind.blockdefine',
  icon: 'copy',
  toolKey: 'draw.blockdefine',
  toolLabelKey: 'tool.draw.blockdefine',
  pick: 'tool.prompt.blockdefine.pick_base',
  text: 'tool.prompt.blockdefine.type_name',
});

const TOOL = points => ({ key: BLOCK.toolKey, points, ready: false });
const AWAITING = points => ({ ...TOOL(points), awaiting_text: true });

const drawPanel = tabId => RIBBON_TABS.find(tab => tab.id === tabId).panels.find(panel => panel.id === 'draw');

test('BLOCK is one small draw-panel button beside INSERT on both tabs, with no variant popover', () => {
  for (const tabId of ['home', 'draw']) {
    const items = drawPanel(tabId).items;
    const ids = items.map(item => item.id);
    const at = ids.indexOf(BLOCK.id);
    assert.ok(at !== -1, `${tabId}/draw must carry the BLOCK affordance`);
    // It sits directly beside INSERT and before OFFSET, so no shipped row moves.
    assert.equal(ids[at - 1], 'INSERT', `${tabId}: BLOCK follows INSERT`);
    assert.equal(ids[at + 1], 'OFFSET', `${tabId}: BLOCK precedes OFFSET`);
    assert.deepEqual(
      ids.slice(-3),
      ['HATCH', 'HATCHPOINT', 'REVCLOUD'],
      `${tabId}: the HATCH pair and REVCLOUD close the draw group`,
    );
    assert.equal(items.filter(item => item.id === BLOCK.id).length, 1, `${tabId}: one BLOCK button`);

    const item = items[at];
    assert.equal(item.command, BLOCK.id);
    assert.equal(item.labelKey, BLOCK.labelKey);
    assert.equal(item.icon, BLOCK.icon);
    assert.equal(item.action, undefined, 'BLOCK must dispatch through the router');
    assert.equal(item.standardView, undefined);
    assert.equal(item.large, undefined, `${tabId}: BLOCK is a small draw button, not a primary tile`);
    // No attributes, no dynamic-block options, no retain/delete choice: a caret
    // would imply choices the command does not have.
    assert.equal(item.variants, undefined, `${tabId}: BLOCK has no variant popover`);
    assert.ok(ribbonCommandIds().includes(BLOCK.id), `${tabId}: the ribbon reaches BLOCK`);
  }
  // No other panel smuggles the command in.
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      if (panel.id === 'draw') continue;
      assert.equal(panel.items.some(item => item.command === BLOCK.id), false, `${tab.id}/${panel.id}`);
    }
  }
});

test('BLOCK is catalogued as a draw command enabled only by router admission', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === BLOCK.id);
  assert.ok(entry, 'BLOCK must be in the command catalog');
  assert.deepEqual(
    { ...entry },
    { id: BLOCK.id, labelKey: BLOCK.labelKey, icon: BLOCK.icon, group: 'draw' },
  );
  // It sits directly after INSERT in the listing, before the modify family.
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.equal(order.indexOf(BLOCK.id), order.indexOf('INSERT') + 1);
  assert.ok(order.indexOf(BLOCK.id) < order.indexOf('MOVE'));
  assert.equal(COMMAND_CATALOG.filter(command => command.id === BLOCK.id).length, 1);

  // Router admission is the single gate — the same as every other draw command.
  assert.equal(isSupportedCommand(BLOCK.id), true);
  assert.equal(isSupportedCommand('block'), true);
  assert.equal(admitCommandText(BLOCK.id, { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText(BLOCK.id, { status: null }).kind, 'unavailable');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand(BLOCK.id), false);
  assert.equal(admitCommandText(BLOCK.id, { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  // Neither capability-gated nor gesture-only.
  assert.equal(CAPABILITY_GATES[BLOCK.id], undefined);
  assert.equal(GESTURE_ONLY_COMMANDS.includes(BLOCK.id), false);
  // The command line offers it with the same admission state and catalog label.
  const suggestion = filterCommandSuggestions(BLOCK.id).find(candidate => candidate.id === BLOCK.id);
  assert.ok(suggestion);
  assert.equal(suggestion.supported, true);
  assert.equal(suggestion.labelKey, BLOCK.labelKey);
  // The active-tool chip key is what the host's `tool.key` maps onto.
  assert.equal(toolLabelKey(BLOCK.toolKey), BLOCK.toolLabelKey);
});

test('the selection plus one base point, then the new block name', () => {
  // Exactly one point-index prompt: the base point. A second entry would claim a
  // pick the command does not take (there is no attribute prompt in v1), and an
  // empty list would report "ready" before anything was picked.
  assert.deepEqual(TOOL_STEP_PROMPTS[BLOCK.toolKey], [BLOCK.pick]);
  assert.equal(TOOL_STEP_PROMPTS[BLOCK.toolKey].length, 1);
  assert.equal(BLOCK_PICK_PROMPT_KEY, BLOCK.pick);
  assert.equal(toolStepPromptKey(TOOL(0)), BLOCK.pick);
  // A malformed/absent count is read as "no points yet" — never a later step.
  assert.equal(toolStepPromptKey({ key: BLOCK.toolKey }), BLOCK.pick);
  assert.equal(toolStepPromptKey({ key: BLOCK.toolKey, points: '1' }), BLOCK.pick);
  assert.equal(toolStepPromptKey({ key: BLOCK.toolKey, points: -1 }), BLOCK.pick);

  // The text step is **not** a pick: it is named from the host's `awaiting_text`
  // flag alone, whatever the captured count says.
  assert.equal(toolStepPromptKey(AWAITING(0)), BLOCK.text);
  assert.equal(toolStepPromptKey(AWAITING(1)), BLOCK.text);
  assert.equal(toolStepPromptKey(AWAITING(7)), BLOCK.text);
  assert.equal(textStepPromptKey({ key: BLOCK.toolKey }), BLOCK.text);
  assert.equal(BLOCK_TEXT_PROMPT_KEY, BLOCK.text);
  assert.equal(TOOL_STEP_PROMPTS[BLOCK.toolKey].includes(BLOCK.text), false);

  // Between the pick and the flag the next step is the name, so the chip names
  // that step instead of borrowing the shared dimension copy — and it never
  // claims a block was created before the name exists.
  assert.equal(TOOL_READY_PROMPTS[BLOCK.toolKey], BLOCK.text);
  assert.equal(toolStepPromptKey(TOOL(1)), BLOCK.text);
  assert.equal(toolStepPromptKey(TOOL(9)), BLOCK.text);
  for (const tool of [TOOL(1), TOOL(9), AWAITING(0), AWAITING(1)]) {
    assert.notEqual(toolStepPromptKey(tool), 'tool.prompt.dim.ready');
    assert.notEqual(toolStepPromptKey(tool), 'tool.prompt.dim.center_ready');
  }

  // It takes a point, so it is **not** a pre-selection tool: a preselection entry
  // would hide the pick behind "select objects first".
  assert.equal(PRESELECT_PROMPTS[BLOCK.toolKey], undefined);

  // The neighbouring text-carrying tools keep their own derivations.
  assert.equal(textStepPromptKey({ key: 'draw.insertblock' }), INSERT_BLOCK_TEXT_PROMPT_KEY);
  assert.notEqual(INSERT_BLOCK_TEXT_PROMPT_KEY, BLOCK.text);
  assert.equal(textStepPromptKey({ key: 'draw.text' }), TEXT_STEP_PROMPT_KEY);
  assert.equal(textStepPromptKey({ key: 'draw.mtext' }), 'tool.prompt.mtext.type_text');
  assert.equal(toolStepPromptKey({ key: 'draw.line', points: 1 }), null);
});

test('both locales carry the BLOCK copy, and it states the v1 boundaries', () => {
  for (const [name, catalog] of [['zh-CN', zh], ['en', en]]) {
    for (const key of [BLOCK.labelKey, BLOCK.toolLabelKey, BLOCK.pick, BLOCK.text]) {
      assert.equal(typeof catalog[key], 'string', `${name} missing ${key}`);
      assert.ok(catalog[key].trim().length > 0, `${name} has empty ${key}`);
    }
    // The catalog label and the active-tool chip name the same thing.
    assert.equal(catalog[BLOCK.labelKey], catalog[BLOCK.toolLabelKey], name);
  }
  // The pick prompt names the base point and says the selected objects move into
  // the new block; the text prompt names a **new block name** and spells out the
  // missing attributes/dynamic-block/retain-delete choices.
  assert.equal(en[BLOCK.pick].includes('base point'), true, en[BLOCK.pick]);
  assert.equal(en[BLOCK.pick].includes('move into the new block'), true, en[BLOCK.pick]);
  assert.equal(zh[BLOCK.pick].includes('基点'), true, zh[BLOCK.pick]);
  assert.equal(zh[BLOCK.pick].includes('移入新块'), true, zh[BLOCK.pick]);

  assert.notEqual(en[BLOCK.text], en[TEXT_STEP_PROMPT_KEY]);
  assert.notEqual(zh[BLOCK.text], zh[TEXT_STEP_PROMPT_KEY]);
  assert.equal(en[BLOCK.text].includes('new block'), true, en[BLOCK.text]);
  assert.equal(en[BLOCK.text].includes('attributes'), true, en[BLOCK.text]);
  assert.equal(en[BLOCK.text].includes('dynamic-block'), true, en[BLOCK.text]);
  assert.equal(en[BLOCK.text].includes('retain/delete'), true, en[BLOCK.text]);
  assert.equal(en[BLOCK.text].includes('not implemented'), true, en[BLOCK.text]);
  assert.equal(zh[BLOCK.text].includes('新块'), true, zh[BLOCK.text]);
  assert.equal(zh[BLOCK.text].includes('属性'), true, zh[BLOCK.text]);
  assert.equal(zh[BLOCK.text].includes('动态块'), true, zh[BLOCK.text]);
  assert.equal(zh[BLOCK.text].includes('尚未实现'), true, zh[BLOCK.text]);
  // The labels stay the short command names, not the prompt sentences.
  assert.notEqual(en[BLOCK.labelKey], en[BLOCK.text]);
  assert.notEqual(zh[BLOCK.labelKey], zh[BLOCK.text]);
});
