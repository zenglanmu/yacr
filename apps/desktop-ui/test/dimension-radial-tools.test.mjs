// Radial-dimension / centre-mark surface contract (ADR 0017 P5d):
// DIMRADIUS/DIMDIAMETER are two-pick captures — an arc/circle entity pick, then
// the dimension-line location — and DIMCENTER is a single arc/circle pick that
// produces **plain geometry** (a centre mark or centrelines), not a dimension.
//
// The ribbon entry, the command catalog, the router selector and the per-step
// status-bar prompt must all exist, and the step must stay derived from the
// host's real `tool.key` + captured count — never a local mirror and never a
// fabricated step. Every user-visible string comes from the shared catalogs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { TOOL_READY_PROMPTS, TOOL_STEP_PROMPTS, toolStepPromptKey } from '../src/cad.mjs';
import {
  admitCommandText,
  COMMAND_CATALOG,
  filterCommandSuggestions,
  isSupportedCommand,
  setCommandCatalog,
  toolLabelKey,
} from '../src/commands.mjs';
import { RIBBON_TABS } from '../src/ribbon-model.mjs';
import { readCatalog, readModule } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

// Mirrors the host vocabulary slice these commands belong to: the router accepts
// all three (`cad_ui_core::session::admissible_names()`), so admission — not a
// local flag — is what enables their ribbon buttons.
const HOST_NAMES = ['DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER'];
setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };

// id -> its active-tool chip key (`tool.draw.<id>`), its named picks in order,
// and the ready prompt once the capture is complete. The radial pair defines a
// dimension entity, so it shares the dimension ready prompt; the centre mark is
// plain geometry and gets its own ready copy.
const RADIAL_TOOLS = [
  { id: 'DIMRADIUS', toolKey: 'draw.dimradius', steps: ['tool.prompt.dim.select_arc', 'tool.prompt.dim.location_radius'], ready: 'tool.prompt.dim.ready' },
  { id: 'DIMDIAMETER', toolKey: 'draw.dimdiameter', steps: ['tool.prompt.dim.select_arc', 'tool.prompt.dim.location_diameter'], ready: 'tool.prompt.dim.ready' },
];
const CENTER_TOOL = { id: 'DIMCENTER', toolKey: 'draw.dimcenter', steps: ['tool.prompt.dim.select_arc'], ready: 'tool.prompt.dim.center_ready' };
const ALL_TOOLS = [...RADIAL_TOOLS, CENTER_TOOL];

function annotationPanel() {
  const annotate = RIBBON_TABS.find(tab => tab.id === 'annotate');
  assert.ok(annotate, 'the annotate tab must exist');
  const panel = annotate.panels.find(entry => entry.id === 'annotation');
  assert.ok(panel, 'the annotate tab must keep its annotation panel');
  return panel;
}

test('the radial pair and centre mark sit with the linear pair in one fixed dimension family', () => {
  const panel = annotationPanel();
  // linear, aligned, radius, diameter, centre mark: the whole dimension family
  // leads the annotation panel in a fixed order, so a CAD user finds the radial
  // pair and the centre mark right next to the linear pair (AutoCAD's Annotate
  // tab order) and the panel never reorders between sessions.
  assert.deepEqual(
    panel.items.slice(0, 5).map(item => item.id),
    ['DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER'],
  );
  for (const { id } of ALL_TOOLS) {
    const item = panel.items.find(candidate => candidate.id === id);
    assert.ok(item, `annotate/annotation must carry the ${id} affordance`);
    // A router command (never a local shell action), rendered as a large button.
    assert.equal(item.command, id);
    assert.equal(item.labelKey, `draw.kind.${id.toLowerCase()}`);
    assert.equal(item.large, true);
    assert.equal(item.action, undefined, `${id} must not be a local shell action`);
  }
  // No other tab's panel dispatches them: they are annotate-only affordances.
  for (const tab of RIBBON_TABS) {
    for (const other of tab.panels) {
      if (other === panel) continue;
      for (const { id } of ALL_TOOLS) {
        assert.equal(
          other.items.some(candidate => candidate.command === id),
          false,
          `${tab.id}/${other.id} must not dispatch ${id}`,
        );
      }
    }
  }
});

test('each new command has a catalog entry in the annotate group, enabled only by router admission', async () => {
  for (const { id, toolKey } of ALL_TOOLS) {
    const entry = COMMAND_CATALOG.find(command => command.id === id);
    assert.ok(entry, `${id} must be in the command catalog`);
    assert.equal(entry.labelKey, `draw.kind.${id.toLowerCase()}`);
    assert.equal(entry.group, 'annotate');
    // The active-tool chip key is what the host's `tool.key` maps onto.
    assert.equal(toolLabelKey(toolKey), `tool.${toolKey}`);
    // Router admission is the single enable gate: admitted when the host
    // vocabulary carries it, unsupported (never locally enabled) when it does not.
    assert.equal(isSupportedCommand(id), true, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
    setCommandCatalog([]);
    assert.equal(isSupportedCommand(id), false, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'unsupported', id);
    setCommandCatalog(HOST_NAMES);
  }
  // The renderer dispatches the selector through the normal command funnel, so
  // confirm/cancel/active state/focus behave exactly like the linear pair.
  const ribbon = await readModule('components/Ribbon.jsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('the command line autocompletes and admits the new commands like their neighbours', () => {
  for (const { id } of ALL_TOOLS) {
    const match = filterCommandSuggestions(id).find(suggestion => suggestion.id === id);
    assert.ok(match, `${id} must be a command-line suggestion`);
    assert.equal(match.supported, true, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
  }
});

test('the step prompt names each radial pick from the host captured count', () => {
  for (const { toolKey, steps, ready } of ALL_TOOLS) {
    for (let points = 0; points < steps.length; points += 1) {
      assert.equal(toolStepPromptKey({ key: toolKey, points }), steps[points], `${toolKey} @${points}`);
    }
    // Once the exact complement is captured the capture is ready and waits for
    // Enter, so the ready prompt names that instead of repeating the last pick.
    assert.equal(toolStepPromptKey({ key: toolKey, points: steps.length }), ready, toolKey);
    // An out-of-range count can only mean a ready capture.
    assert.equal(toolStepPromptKey({ key: toolKey, points: 9 }), ready, toolKey);
  }
  // The radial pair defines a dimension entity, so it keeps the shared ready
  // prompt; the centre mark is plain geometry, so its ready copy differs.
  assert.equal(TOOL_STEP_PROMPTS['draw.dimradius'].length, 2);
  assert.equal(TOOL_STEP_PROMPTS['draw.dimdiameter'].length, 2);
  assert.equal(toolStepPromptKey({ key: 'draw.dimradius', points: 2 }), 'tool.prompt.dim.ready');
  assert.equal(toolStepPromptKey({ key: 'draw.dimdiameter', points: 2 }), 'tool.prompt.dim.ready');
  assert.equal(toolStepPromptKey({ key: 'draw.dimcenter', points: 1 }), 'tool.prompt.dim.center_ready');
  assert.notEqual(toolStepPromptKey({ key: 'draw.dimcenter', points: 1 }), 'tool.prompt.dim.ready');
  // The first step is the shared arc/circle entity pick for the whole radial
  // family (and the centre mark).
  for (const { toolKey } of ALL_TOOLS) {
    assert.equal(TOOL_STEP_PROMPTS[toolKey][0], 'tool.prompt.dim.select_arc');
  }
  // The radial pair's second pick is the dimension location, and the radius and
  // diameter prompts are deliberately different copy.
  assert.equal(TOOL_STEP_PROMPTS['draw.dimradius'][1], 'tool.prompt.dim.location_radius');
  assert.equal(TOOL_STEP_PROMPTS['draw.dimdiameter'][1], 'tool.prompt.dim.location_diameter');
  assert.notEqual(TOOL_STEP_PROMPTS['draw.dimradius'][1], TOOL_STEP_PROMPTS['draw.dimdiameter'][1]);
  // DIMCENTER is a single entity pick.
  assert.equal(TOOL_STEP_PROMPTS['draw.dimcenter'].length, 1);
  // A malformed or absent count is read as "no points yet", never a later step.
  assert.equal(toolStepPromptKey({ key: 'draw.dimradius' }), 'tool.prompt.dim.select_arc');
  assert.equal(toolStepPromptKey({ key: 'draw.dimradius', points: '1' }), 'tool.prompt.dim.select_arc');
  assert.equal(toolStepPromptKey({ key: 'draw.dimdiameter', points: -1 }), 'tool.prompt.dim.select_arc');
  assert.equal(toolStepPromptKey({ key: 'draw.dimcenter', points: -1 }), 'tool.prompt.dim.select_arc');
  // A tool with no named steps still renders no prompt at all.
  assert.equal(toolStepPromptKey({ key: 'draw.line', points: 1 }), null);
});

test('the radial location prompt never claims the user positions the text', () => {
  // The location pick only sets the leader direction from the centre; the
  // synthesis derives the text placement, so the copy must not tell the user to
  // place the text.
  for (const key of ['tool.prompt.dim.location_radius', 'tool.prompt.dim.location_diameter']) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
    assert.doesNotMatch(en[key], /text/i, key);
    assert.doesNotMatch(zh[key], /文字|文本/, key);
  }
  // The first step names the real entity pick the backend expects: an arc or a
  // circle (a fillet arc counts).
  assert.match(en['tool.prompt.dim.select_arc'], /arc/i);
  assert.match(en['tool.prompt.dim.select_arc'], /circle/i);
  assert.match(zh['tool.prompt.dim.select_arc'], /圆弧/);
});

test('the centre-mark ready copy never claims a dimension was defined', () => {
  // DIMCENTER emits plain line geometry — a centre mark or centrelines — not a
  // dimension entity, so its ready copy must not claim a dimension was defined
  // (the shared ready prompt does, correctly, for the real dimension commands).
  const key = 'tool.prompt.dim.center_ready';
  for (const locale of [zh, en]) {
    assert.equal(typeof locale[key], 'string', `missing ${key}`);
    assert.ok(locale[key].trim().length > 0, `empty ${key}`);
  }
  assert.doesNotMatch(en[key], /dimension/i, key);
  assert.doesNotMatch(zh[key], /标注/, key);
  // The shared prompt DOES claim a dimension — that contrast is the point.
  assert.match(en['tool.prompt.dim.ready'], /dimension/i);
  assert.match(zh['tool.prompt.dim.ready'], /标注/);
  // It does describe the real output: a centre mark or centrelines to place.
  assert.match(en[key], /center mark/i);
  assert.match(en[key], /cent(er|re)lines/i);
  assert.match(zh[key], /圆心标记/);
  assert.match(zh[key], /中心线/);
  // And it is wired as DIMCENTER's own ready prompt, not the shared one.
  assert.equal(TOOL_READY_PROMPTS['draw.dimcenter'], key);
  assert.notEqual(TOOL_READY_PROMPTS['draw.dimcenter'], 'tool.prompt.dim.ready');
  // The radial pair defines a dimension entity, so it is NOT overridden.
  assert.equal(TOOL_READY_PROMPTS['draw.dimradius'], undefined);
  assert.equal(TOOL_READY_PROMPTS['draw.dimdiameter'], undefined);
});

test('every radial step-prompt key exists in both catalogs with real copy', () => {
  const keys = new Set(['tool.prompt.dim.select_arc', 'tool.prompt.dim.ready']);
  for (const { steps, ready } of ALL_TOOLS) {
    for (const key of steps) keys.add(key);
    keys.add(ready);
  }
  for (const key of keys) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
  }
});

test('every ribbon/catalog/chip key resolves in both catalogs', () => {
  const keys = new Set();
  for (const { id, toolKey } of ALL_TOOLS) {
    keys.add(`draw.kind.${id.toLowerCase()}`);
    keys.add(`tool.${toolKey}`);
  }
  for (const key of keys) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
  }
});

test('the new ribbon items use dedicated dimension glyphs that exist and stay distinct', async () => {
  const icons = await readModule('icons.jsx');
  const map = icons.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.jsx');
  const defined = new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
  const panel = annotationPanel();
  // Each new command carries its own glyph, named for the command.
  const dedicated = { DIMRADIUS: 'dimradius', DIMDIAMETER: 'dimdiameter', DIMCENTER: 'dimcenter' };
  const used = [];
  for (const { id } of ALL_TOOLS) {
    const item = panel.items.find(candidate => candidate.id === id);
    assert.ok(item.icon, `${id} must carry an icon`);
    assert.equal(item.icon, dedicated[id], `${id} must use its dedicated dimension glyph`);
    // An unknown name makes <Icon> silently fall back to a dot, so the name must
    // really exist in the set.
    assert.ok(defined.has(item.icon), `${id} icon '${item.icon}' must exist in icons.jsx`);
    used.push(item.icon);
  }
  // Three different glyphs, so a CAD user can tell the family apart at a glance.
  assert.equal(new Set(used).size, used.length, 'the three new tools must be visually distinguishable');
  // The whole dimension family uses dedicated glyphs, never the reused
  // arc/circle/point placeholders.
  const family = ['DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER'];
  const reused = new Set(['arc', 'circle', 'point']);
  for (const id of family) {
    const item = panel.items.find(candidate => candidate.id === id);
    assert.ok(defined.has(item.icon), `${id} icon '${item.icon}' must exist`);
    assert.equal(reused.has(item.icon), false, `${id} must not reuse the '${item.icon}' placeholder glyph`);
  }
  // The five dedicated glyphs are all present and mutually distinct.
  const familyIcons = family.map(id => panel.items.find(item => item.id === id).icon);
  assert.equal(new Set(familyIcons).size, familyIcons.length, 'the dimension family glyphs must be distinct');
  // The catalog uses the same dedicated names, so the command-line list matches.
  const { COMMAND_CATALOG } = await import('../src/commands.mjs');
  for (const id of family) {
    assert.equal(COMMAND_CATALOG.find(command => command.id === id).icon, panel.items.find(item => item.id === id).icon, id);
  }
});
