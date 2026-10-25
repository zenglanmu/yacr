// Angular / ordinate dimension surface contract (ADR 0017 decision C, P5e):
// DIMANGULAR is the **two-line** capture — two straight-line entity picks, then
// the arc location that selects which of the four angles is measured — and
// DIMORDINATE is a two-point capture (a feature point, then the leader endpoint)
// whose axis the backend derives on its own.
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
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon-model.mjs';
import { readCatalog, readModule } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

// Mirrors the host vocabulary slice these commands belong to: the router accepts
// both (`cad_ui_core::session::admissible_names()`), so admission — not a local
// flag — is what enables their ribbon buttons.
const HOST_NAMES = [
  'DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER', 'DIMANGULAR', 'DIMORDINATE',
];
setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };

// id -> its active-tool chip key (`tool.draw.<id>`), its named picks in order,
// and the ready prompt once the capture is complete. Both commands commit real
// DIMENSION entities, so both keep the shared dimension ready copy.
const ANGULAR_TOOL = {
  id: 'DIMANGULAR',
  toolKey: 'draw.dimangular',
  steps: ['tool.prompt.dim.select_line', 'tool.prompt.dim.select_second_line', 'tool.prompt.dim.location_angular'],
  ready: 'tool.prompt.dim.ready',
};
const ORDINATE_TOOL = {
  id: 'DIMORDINATE',
  toolKey: 'draw.dimordinate',
  steps: ['tool.prompt.dim.feature', 'tool.prompt.dim.leader_endpoint'],
  ready: 'tool.prompt.dim.ready',
};
const ALL_TOOLS = [ANGULAR_TOOL, ORDINATE_TOOL];

// The whole dimension family, in the one fixed order the panel ships.
const FAMILY = ['DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER', 'DIMANGULAR', 'DIMORDINATE'];

function annotationPanel() {
  const annotate = RIBBON_TABS.find(tab => tab.id === 'annotate');
  assert.ok(annotate, 'the annotate tab must exist');
  const panel = annotate.panels.find(entry => entry.id === 'annotation');
  assert.ok(panel, 'the annotate tab must keep its annotation panel');
  return panel;
}

test('the angular and ordinate dimensions extend the one contiguous dimension family', () => {
  const panel = annotationPanel();
  // Linear, aligned, radius, diameter, centre mark, then the angular/ordinate
  // pair: the family is one contiguous block in a fixed order, and the two new
  // commands were appended after the already-shipped five so no existing button
  // moved (the panel never reorders between sessions).
  assert.deepEqual(panel.items.slice(0, FAMILY.length).map(item => item.id), FAMILY);
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
  // The ribbon reaches both commands, so the catalog-honesty sweep sees them too.
  const ids = new Set(ribbonCommandIds());
  for (const { id } of ALL_TOOLS) assert.ok(ids.has(id), `the ribbon must reference ${id}`);
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

test('the step prompt names each angular and ordinate pick from the host captured count', () => {
  for (const { toolKey, steps, ready } of ALL_TOOLS) {
    for (let points = 0; points < steps.length; points += 1) {
      assert.equal(toolStepPromptKey({ key: toolKey, points }), steps[points], `${toolKey} @${points}`);
    }
    // Once the exact complement is captured the capture is ready and waits for
    // Enter, so the ready prompt names that instead of repeating the last pick.
    assert.equal(toolStepPromptKey({ key: toolKey, points: steps.length }), ready, toolKey);
    // An out-of-range count can only mean a ready capture.
    assert.equal(toolStepPromptKey({ key: toolKey, points: 9 }), ready, toolKey);
    // A malformed or absent count is read as "no points yet", never a later step.
    assert.equal(toolStepPromptKey({ key: toolKey }), steps[0], toolKey);
    assert.equal(toolStepPromptKey({ key: toolKey, points: '1' }), steps[0], toolKey);
    assert.equal(toolStepPromptKey({ key: toolKey, points: -1 }), steps[0], toolKey);
  }
  // Two line picks then the arc location; never a third point pick.
  assert.equal(TOOL_STEP_PROMPTS['draw.dimangular'].length, 3);
  // Feature point then leader endpoint.
  assert.equal(TOOL_STEP_PROMPTS['draw.dimordinate'].length, 2);
});

test('both new commands keep the shared dimension ready copy, never the centre-mark copy', () => {
  // DIMANGULAR and DIMORDINATE commit real DIMENSION entities, so the shared
  // ready prompt (which names a defined dimension) is correct for them.
  for (const { toolKey, steps } of ALL_TOOLS) {
    assert.equal(toolStepPromptKey({ key: toolKey, points: steps.length }), 'tool.prompt.dim.ready', toolKey);
    assert.notEqual(toolStepPromptKey({ key: toolKey, points: steps.length }), 'tool.prompt.dim.center_ready', toolKey);
    // Neither is overridden: only DIMCENTER (plain geometry) needs its own copy.
    assert.equal(TOOL_READY_PROMPTS[toolKey], undefined, `${toolKey} must fall through to the shared ready copy`);
  }
  // The shared copy really does claim a dimension, and the centre-mark copy
  // really does not — that contrast is why reusing the latter would be a lie.
  assert.match(en['tool.prompt.dim.ready'], /dimension/i);
  assert.match(zh['tool.prompt.dim.ready'], /标注/);
  assert.doesNotMatch(en['tool.prompt.dim.center_ready'], /dimension/i);
  assert.doesNotMatch(zh['tool.prompt.dim.center_ready'], /标注/);
  // The centre mark is the only override, so the two new tools never inherit it.
  assert.equal(TOOL_READY_PROMPTS['draw.dimcenter'], 'tool.prompt.dim.center_ready');
});

test('the angular prompts describe the two-line capture and never a vertex workflow', () => {
  const [first, second, location] = ANGULAR_TOOL.steps;
  for (const key of ANGULAR_TOOL.steps) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
  }
  // The picks are lines, and the second prompt announces the second pick.
  assert.match(en[first], /line/i);
  assert.match(zh[first], /直线/);
  assert.match(en[second], /second/i);
  assert.match(zh[second], /第二条直线/);
  // Honest boundary: the interactive tool is the two-line form only. The
  // three-point form has no interactive capture, so nothing may ask for a vertex
  // (or any other point) first.
  for (const key of [first, second]) {
    assert.doesNotMatch(en[key], /vertex|point/i, key);
    assert.doesNotMatch(zh[key], /顶点/, key);
    assert.notEqual(key, 'tool.prompt.dim.origin_first');
    assert.notEqual(key, 'tool.prompt.dim.origin_second');
  }
  // The third pick is the dimension arc location.
  assert.match(en[location], /arc/i);
  assert.match(zh[location], /弧/);
  // Honest boundary: the backend's rule is "the sector containing the pick".
  // The copy must not promise a quadrant picker (or any discrete angle chooser)
  // and must not claim the user places the text.
  for (const locale of [zh, en]) {
    assert.doesNotMatch(locale[location], /quadrant|picker|dropdown/i, location);
    assert.doesNotMatch(locale[location], /象限|下拉|选择器/, location);
  }
});

test('the ordinate prompts name the two points and promise no axis control', () => {
  const [feature, leader] = ORDINATE_TOOL.steps;
  for (const key of ORDINATE_TOOL.steps) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
  }
  assert.match(en[feature], /feature/i);
  assert.match(zh[feature], /特征/);
  assert.match(en[leader], /leader/i);
  assert.match(en[leader], /endpoint/i);
  assert.match(zh[leader], /引线/);
  // Both steps are free points, and the interactive tool never forces an axis
  // (the backend applies its own auto rule), so no prompt may offer an axis pick.
  for (const key of ORDINATE_TOOL.steps) {
    assert.doesNotMatch(en[key], /axis|x ordinate|y ordinate/i, key);
    assert.doesNotMatch(zh[key], /坐标轴|X 坐标|Y 坐标/, key);
  }
});

test('every new step-prompt and chip key exists in both catalogs with real copy', () => {
  const keys = new Set(['tool.prompt.dim.ready', 'tool.prompt.dim.center_ready']);
  for (const { steps, ready } of ALL_TOOLS) {
    for (const key of steps) keys.add(key);
    keys.add(ready);
  }
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
  // The ribbon/catalog labels and the active-tool chip names are the same strings
  // a CAD user sees on the button and in the status bar.
  assert.equal(en['draw.kind.dimangular'], en['tool.draw.dimangular']);
  assert.equal(en['draw.kind.dimordinate'], en['tool.draw.dimordinate']);
  assert.equal(zh['draw.kind.dimangular'], zh['tool.draw.dimangular']);
  assert.equal(zh['draw.kind.dimordinate'], zh['tool.draw.dimordinate']);
});

test('the new ribbon items use dedicated dimension glyphs that exist and stay distinct', async () => {
  const icons = await readModule('icons.jsx');
  const map = icons.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.jsx');
  const defined = new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
  const panel = annotationPanel();
  // Each new command carries its own glyph, named for the command.
  const dedicated = { DIMANGULAR: 'dimangular', DIMORDINATE: 'dimordinate' };
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
  assert.equal(new Set(used).size, used.length, 'the two new tools must be visually distinguishable');
  // The whole dimension family uses dedicated glyphs, never a reused placeholder.
  // `angle` is the measure-angle glyph the angular dimension deliberately builds
  // on (rays plus arc), so it must stay a *distinct* icon, not become this one.
  const reused = new Set(['arc', 'circle', 'point', 'angle']);
  const familyIcons = [];
  for (const id of FAMILY) {
    const item = panel.items.find(candidate => candidate.id === id);
    assert.ok(defined.has(item.icon), `${id} icon '${item.icon}' must exist`);
    assert.equal(reused.has(item.icon), false, `${id} must not reuse the '${item.icon}' neighbour glyph`);
    familyIcons.push(item.icon);
  }
  // Seven dedicated glyphs, all mutually distinct.
  assert.equal(new Set(familyIcons).size, familyIcons.length, 'the dimension family glyphs must be distinct');
  // The catalog uses the same dedicated names, so the command-line list matches.
  for (const id of FAMILY) {
    assert.equal(
      COMMAND_CATALOG.find(command => command.id === id).icon,
      panel.items.find(item => item.id === id).icon,
      id,
    );
  }
  // The angular glyph is built from the measure-angle vocabulary plus dimension
  // arrowheads: same two rays, but an arc with arrowheads on it.
  const angularGlyph = map[1].match(/\n\s*dimangular:\s*(<><[\s\S]*?<\/>),/);
  assert.ok(angularGlyph, 'expected a dimangular glyph');
  assert.equal((angularGlyph[1].match(/<path/g) ?? []).length, 5, 'dimangular = two rays, one arc, two arrowhead wings');
  // The ordinate glyph is a feature point plus a leader that ends in a tick.
  const ordinateGlyph = map[1].match(/\n\s*dimordinate:\s*(<><[\s\S]*?<\/>),/);
  assert.ok(ordinateGlyph, 'expected a dimordinate glyph');
  assert.match(ordinateGlyph[1], /<circle/, 'the feature point is a circled point');
  assert.equal((ordinateGlyph[1].match(/<path/g) ?? []).length, 2, 'dimordinate = one leader line and one end tick');
});
