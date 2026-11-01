// CIRCLE 2P / CIRCLE 3P / CIRCLE D user-facing surface contract.
//
// The plain `CIRCLE` capture (centre, then radius) stays as it is; this file pins
// the three **definition modes** the host advertises as separate multi-word
// selectors:
//
//   * `CIRCLE 2P` — two diameter endpoints;
//   * `CIRCLE 3P` — three points on the circumference;
//   * `CIRCLE D`  — a centre, then a point whose distance from that centre is the
//     **diameter**, so the radius is half of the picked distance.
//
// Every mode commits exactly one circle and takes no parameter. The copy is the
// only place the D halving is explained, so the assertions below read the real
// sentences rather than a summary of them: a reader must never expect a radius
// pick there. None of the three is a pre-selection tool — the picks are the whole
// input — and every string comes from the shared catalogs.
//
// The ribbon side is a split button on both draw panels (the `FIT` / `AREA`
// popover shape): the plain CIRCLE remains the parent, and the caret lists the
// four definitions in the order above.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  filterCommandSuggestions,
  isSupportedCommand,
  setCommandCatalog,
  splitCommandText,
  toolLabelKey,
} from '../src/commands/commands.ts';
import {
  PRESELECT_PROMPTS,
  TOOL_READY_PROMPTS,
  TOOL_STEP_PROMPTS,
  toolStepPromptKey,
} from '../src/cad/cad.ts';
import { TOOL_PARAMETERS } from '../src/tools/tool-parameter-specs.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const MODES = [
  {
    id: 'CIRCLE 2P',
    labelKey: 'draw.kind.circle2p',
    toolKey: 'draw.circle2p',
    toolLabel: 'tool.draw.circle2p',
    argument: '2P',
    steps: ['tool.prompt.circle2p.first', 'tool.prompt.circle2p.second'],
    ready: 'tool.prompt.circle2p.ready',
  },
  {
    id: 'CIRCLE 3P',
    labelKey: 'draw.kind.circle3p',
    toolKey: 'draw.circle3p',
    toolLabel: 'tool.draw.circle3p',
    argument: '3P',
    steps: ['tool.prompt.circle3p.first', 'tool.prompt.circle3p.second', 'tool.prompt.circle3p.third'],
    ready: 'tool.prompt.circle3p.ready',
  },
  {
    id: 'CIRCLE D',
    labelKey: 'draw.kind.circlediameter',
    toolKey: 'draw.circlediameter',
    toolLabel: 'tool.draw.circlediameter',
    argument: 'D',
    steps: ['tool.prompt.circlediameter.center', 'tool.prompt.circlediameter.diameter'],
    ready: 'tool.prompt.circlediameter.ready',
  },
];

const loaded = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  canOpen: true,
  canNew: true,
  gridEnabled: true,
  snapGridEnabled: false,
  snapSpacing: 10,
  orthoEnabled: false,
  canSave: true,
  canPlot: true,
  canFit: true,
};

function drawPanel(tabId) {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `${tabId} tab must exist`);
  const panel = tab.panels.find(entry => entry.id === 'draw');
  assert.ok(panel, `${tabId} must keep its draw panel`);
  return panel;
}

async function iconNames() {
  const map = (await readModule('icons.tsx')).match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
}

test('the three circle modes are catalogued draw selectors beside CIRCLE', async () => {
  const order = COMMAND_CATALOG.map(command => command.id);
  const glyphs = await iconNames();
  for (const entry of MODES) {
    assert.deepEqual(
      { ...COMMAND_CATALOG.find(command => command.id === entry.id) },
      { id: entry.id, labelKey: entry.labelKey, icon: 'circle', group: 'draw' },
      entry.id,
    );
    // The shared glyph really exists: an unknown name would silently render the
    // `dot` fallback, which would make the row unreadable.
    assert.ok(glyphs.has('circle'), "icons.tsx must define 'circle'");
    // Both locale halves of the row exist (the label and the tool chip name).
    for (const locale of [zh, en]) {
      for (const key of [entry.labelKey, entry.toolLabel]) {
        assert.equal(typeof locale[key], 'string', `missing ${key}`);
        assert.ok(locale[key].trim().length > 0, `empty ${key}`);
      }
    }
  }
  // Listed directly after the plain CIRCLE they refine, so the completion list
  // reads CIRCLE -> CIRCLE 2P -> CIRCLE 3P -> CIRCLE D -> ARC; no shipped row
  // changes its relative order.
  assert.equal(order.indexOf('CIRCLE 2P'), order.indexOf('CIRCLE') + 1);
  assert.equal(order.indexOf('CIRCLE 3P'), order.indexOf('CIRCLE 2P') + 1);
  assert.equal(order.indexOf('CIRCLE D'), order.indexOf('CIRCLE 3P') + 1);
  assert.equal(order.indexOf('ARC'), order.indexOf('CIRCLE D') + 1);
});

test('each mode is admitted as one whole selector, never CIRCLE plus an argument', () => {
  setCommandCatalog(HOST_NAMES);
  for (const entry of MODES) {
    // The local grammar splits the line into a base name and an argument…
    const split = splitCommandText(entry.id);
    assert.equal(split.name, 'CIRCLE', entry.id);
    assert.equal(split.argument, entry.argument, entry.id);
    // …but admission resolves the **whole** line first, exactly like the host
    // router, so the space-separated selector is admitted as itself.
    assert.equal(isSupportedCommand(entry.id), true, `${entry.id} is a router-admitted draw selector`);
    const admission = admitCommandText(entry.id, { status: loaded });
    assert.equal(admission.kind, 'admitted', entry.id);
    assert.equal(admission.command, entry.id, `${entry.id} travels as one command name`);
    assert.equal(admission.reasonKey, null, entry.id);
    // Completion offers the mode: typing the base name lists all four rows.
    const suggestions = filterCommandSuggestions('CIRCLE');
    const row = suggestions.find(suggestion => suggestion.id === entry.id);
    assert.ok(row, `${entry.id} must be suggested`);
    assert.equal(row.supported, true, entry.id);
    assert.equal(row.labelKey, entry.labelKey, entry.id);
  }
  // Without the host vocabulary nothing is admitted: the mode is never inferred
  // from the plain CIRCLE selector.
  setCommandCatalog([]);
  for (const entry of MODES) {
    assert.equal(isSupportedCommand(entry.id), false, entry.id);
    assert.equal(admitCommandText(entry.id, { status: loaded }).kind, 'unsupported', entry.id);
  }
  setCommandCatalog(HOST_NAMES);
});

test('both draw panels offer the CIRCLE split button with the four definitions', () => {
  const expected = ['CIRCLE', 'CIRCLE 2P', 'CIRCLE 3P', 'CIRCLE D'];
  for (const tabId of ['home', 'draw']) {
    const items = drawPanel(tabId).items;
    const item = items.find(candidate => candidate.id === 'CIRCLE');
    assert.ok(item, `${tabId}/draw must keep the CIRCLE button`);
    assert.equal(items.filter(candidate => candidate.id === 'CIRCLE').length, 1, `${tabId}: one CIRCLE button`);
    // The parent is unchanged: the plain centre-then-radius command, still a large
    // primary tile.
    assert.equal(item.command, 'CIRCLE', `${tabId}: the parent keeps its own selector`);
    assert.equal(item.labelKey, 'draw.kind.circle', tabId);
    assert.equal(item.icon, 'circle', tabId);
    assert.equal(item.large, true, `${tabId}: CIRCLE stays a primary draw tile`);
    assert.equal(item.action, undefined, `${tabId}: CIRCLE must be a router command`);
    // The caret lists the four definition modes in listing order, each a router
    // command of its own with a label both catalogs carry.
    assert.deepEqual((item.variants ?? []).map(variant => variant.id), expected, tabId);
    for (const variant of item.variants ?? []) {
      assert.equal(variant.command, variant.id, `${tabId}: ${variant.id} dispatches itself`);
      assert.equal(variant.icon, 'circle', `${tabId}: ${variant.id} shares the circle glyph`);
      assert.equal(variant.action, undefined, `${tabId}: ${variant.id} is not a shell action`);
      assert.equal(typeof zh[variant.labelKey], 'string', `${tabId}: zh ${variant.labelKey}`);
      assert.equal(typeof en[variant.labelKey], 'string', `${tabId}: en ${variant.labelKey}`);
    }
    // The first row repeats the parent's own selector, the way FIT and AREA open
    // their menus with the button's plain action.
    assert.equal(item.variants[0].command, 'CIRCLE', tabId);
  }
  // The modes live only in the draw panels' popover: no other panel dispatches
  // one, and every one of them reaches the ribbon dispatcher.
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      if (panel.id === 'draw') continue;
      for (const entry of MODES) {
        assert.equal(panel.items.some(candidate => candidate.command === entry.id), false, `${tab.id}/${panel.id}/${entry.id}`);
      }
    }
  }
  const dispatched = new Set(ribbonCommandIds());
  for (const entry of MODES) assert.ok(dispatched.has(entry.id), `${entry.id} reaches the ribbon dispatcher`);
});

test('the chip names every pick in order, then the captured definition', () => {
  for (const entry of MODES) {
    // An ordered list: one sentence per pick, and the ready state is not
    // positional so it lives in the parallel ready table.
    assert.deepEqual(TOOL_STEP_PROMPTS[entry.toolKey], entry.steps, entry.toolKey);
    assert.equal(TOOL_READY_PROMPTS[entry.toolKey], entry.ready, entry.toolKey);
    // A pick tool, deliberately absent from the zero-pick pre-selection table.
    assert.equal(PRESELECT_PROMPTS[entry.toolKey], undefined, entry.toolKey);
    // No parameter row: the picks are the whole input.
    assert.equal(TOOL_PARAMETERS[entry.toolKey], undefined, entry.toolKey);
    // The active-tool chip names the mode, not the plain "Circle".
    assert.equal(toolLabelKey(entry.toolKey), entry.toolLabel, entry.toolKey);

    // Every count before the complement names that pick; the host's `ready` flag
    // never moves a pick tool forward (the count decides).
    for (let points = 0; points < entry.steps.length; points += 1) {
      assert.equal(toolStepPromptKey({ key: entry.toolKey, points }), entry.steps[points], `${entry.toolKey} @${points}`);
      assert.equal(
        toolStepPromptKey({ key: entry.toolKey, points, ready: true }),
        entry.steps[points],
        `${entry.toolKey} @${points} ready`,
      );
    }
    // The exact complement is "definition captured, Enter creates".
    assert.equal(toolStepPromptKey({ key: entry.toolKey, points: entry.steps.length }), entry.ready, entry.toolKey);
    assert.equal(
      toolStepPromptKey({ key: entry.toolKey, points: entry.steps.length + 3 }),
      entry.ready,
      `${entry.toolKey} never invents a later pick`,
    );
    // A malformed count is read as "no points yet", never as a later step.
    assert.equal(toolStepPromptKey({ key: entry.toolKey, points: '1' }), entry.steps[0], entry.toolKey);
    // Nothing here is dimensioned, so the dimension ready copy would be false.
    assert.notEqual(entry.ready, 'tool.prompt.dim.ready', entry.toolKey);
    assert.notEqual(entry.ready, 'tool.prompt.dim.center_ready', entry.toolKey);
  }
});

test('both locales word the definitions and say the D pick is the diameter', () => {
  for (const entry of MODES) {
    for (const key of [entry.labelKey, entry.toolLabel, entry.ready, ...entry.steps]) {
      for (const locale of [zh, en]) {
        assert.equal(typeof locale[key], 'string', `missing ${key}`);
        assert.ok(locale[key].trim().length > 0, `empty ${key}`);
      }
    }
  }

  // English: the two-endpoint mode asks for the diameter's endpoints…
  assert.equal(en['tool.prompt.circle2p.first'], 'Specify the first diameter endpoint');
  assert.equal(en['tool.prompt.circle2p.second'], 'Specify the second diameter endpoint');
  // …the three-point mode asks for points *on the circle*…
  assert.equal(en['tool.prompt.circle3p.first'], 'Specify the first point on the circle');
  assert.equal(en['tool.prompt.circle3p.second'], 'Specify the second point on the circle');
  assert.equal(en['tool.prompt.circle3p.third'], 'Specify the third point on the circle');
  // …and the D mode states the halving outright: the picked distance is the
  // diameter and the radius is half of it. This is the one sentence a reader has
  // before the first pick, so it may not read as a radius prompt.
  assert.equal(en['tool.prompt.circlediameter.center'], 'Specify the circle centre');
  assert.match(en['tool.prompt.circlediameter.diameter'], /diameter/i);
  assert.match(en['tool.prompt.circlediameter.diameter'], /radius is half/i);
  assert.doesNotMatch(en['tool.prompt.circlediameter.diameter'], /^Specify (?:the )?radius\b/i);
  // The labels name the definition method as well, so the popover row is honest
  // before the capture starts.
  assert.match(en['draw.kind.circle2p'], /2P/i);
  assert.match(en['draw.kind.circle2p'], /diameter/i);
  assert.match(en['draw.kind.circle3p'], /3P/i);
  assert.match(en['draw.kind.circlediameter'], /diameter/i);
  assert.match(en['tool.draw.circlediameter'], /diameter/i);

  // Chinese: the same three definitions, grounded and literal.
  assert.equal(zh['tool.prompt.circlediameter.center'], '指定圆心');
  assert.match(zh['tool.prompt.circlediameter.diameter'], /直径/);
  assert.match(zh['tool.prompt.circlediameter.diameter'], /半径/);
  assert.match(zh['tool.prompt.circlediameter.diameter'], /一半/);
  assert.match(zh['tool.prompt.circle2p.first'], /直径/);
  assert.match(zh['tool.prompt.circle3p.first'], /圆/);
  assert.match(zh['draw.kind.circlediameter'], /直径/);
  assert.match(zh['tool.draw.circlediameter'], /直径/);

  // The three ready sentences are their own captured state, never the shared
  // dimension copy.
  assert.equal(new Set(MODES.map(entry => en[entry.ready])).size, MODES.length);
  assert.equal(new Set(MODES.map(entry => zh[entry.ready])).size, MODES.length);
  for (const entry of MODES) {
    assert.match(en[entry.ready], /press Enter to create/i);
    assert.match(zh[entry.ready], /按回车创建/);
  }
});
