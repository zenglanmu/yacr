// Baseline / continue dimension surface contract (ADR 0017 decision C, P5f):
// DIMBASELINE and DIMCONTINUE both start by picking an **existing** linear
// dimension as the base, then accept an open-ended run of measured points and
// commit the batch with Enter — never the linear pair's fixed three picks.
//
// So the ribbon entry, the command catalog, the router selector, the two glyphs
// and the per-step status-bar prompt must all exist, and the step must be derived
// from the host's real `tool.key` + captured count + `ready`, never from a local
// mirror and never from a hard-coded number of points. Every user-visible string
// comes from the shared catalogs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { OPEN_ENDED_PROMPTS } from '../src/tools/tool-open-ended.ts';
import { TOOL_READY_PROMPTS, TOOL_STEP_PROMPTS, toolStepPromptKey } from '../src/cad/cad.ts';
import {
  admitCommandText,
  filterCommandSuggestions,
  isSupportedCommand,
  setCommandCatalog,
  toolLabelKey,
} from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

// Mirrors the host vocabulary slice these commands belong to: the router accepts
// both (`cad_ui_core::session::admissible_names()`), so admission — not a local
// flag — is what enables their ribbon buttons.
const HOST_NAMES = [
  'DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER', 'DIMANGULAR', 'DIMORDINATE',
  'DIMBASELINE', 'DIMCONTINUE',
];
setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };

// The one shared pick and the per-command repeatable point step. Both commands
// commit real DIMENSION entities, so both keep the shared dimension ready copy.
const BASE = 'tool.prompt.dim.select_base';
const READY = 'tool.prompt.dim.ready';
const TOOLS = [
  { id: 'DIMBASELINE', toolKey: 'draw.dimbaseline', point: 'tool.prompt.dim.baseline_point' },
  { id: 'DIMCONTINUE', toolKey: 'draw.dimcontinue', point: 'tool.prompt.dim.continue_point' },
];

// The whole dimension family, in the one fixed order the panel ships: the seven
// creation commands, then the two that consume an existing dimension.
const FAMILY = [
  'DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER', 'DIMANGULAR', 'DIMORDINATE',
  'DIMBASELINE', 'DIMCONTINUE',
];

function annotationPanel() {
  const annotate = RIBBON_TABS.find(tab => tab.id === 'annotate');
  assert.ok(annotate, 'the annotate tab must exist');
  const panel = annotate.panels.find(entry => entry.id === 'annotation');
  assert.ok(panel, 'the annotate tab must keep its annotation panel');
  return panel;
}

test('baseline and continue close the dimension family without moving a shipped button', () => {
  const panel = annotationPanel();
  // The family stays one contiguous block in a fixed order: the two new commands
  // are appended after the seven creation commands, so every existing button keeps
  // its slot and the panel never reorders between sessions.
  assert.deepEqual(panel.items.slice(0, FAMILY.length).map(item => item.id), FAMILY);
  for (const { id } of TOOLS) {
    const item = panel.items.find(candidate => candidate.id === id);
    assert.ok(item, `annotate/annotation must carry the ${id} affordance`);
    // A router command (never a local shell action), rendered as a large tile so
    // it reads as part of the same dimension set.
    assert.equal(item.command, id);
    assert.equal(item.labelKey, `draw.kind.${id.toLowerCase()}`);
    assert.equal(item.large, true);
    assert.equal(item.action, undefined, `${id} must not be a local shell action`);
    // No variant caret: there is no `Options` sub-prompt for these commands.
    assert.equal(item.variants, undefined, `${id} must not imply an options menu`);
  }
  // No other tab's panel dispatches them: they are annotate-only affordances.
  for (const tab of RIBBON_TABS) {
    for (const other of tab.panels) {
      if (other === panel) continue;
      for (const { id } of TOOLS) {
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
  for (const { id } of TOOLS) assert.ok(ids.has(id), `the ribbon must reference ${id}`);
});

test('each new command has an annotate catalog row enabled only by router admission', async () => {
  for (const { id, toolKey } of TOOLS) {
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
    // The command line lists and admits them like their neighbours.
    const suggestion = filterCommandSuggestions(id).find(candidate => candidate.id === id);
    assert.ok(suggestion, `${id} must be a command-line suggestion`);
    assert.equal(suggestion.supported, true, id);
  }
  // The renderer dispatches the selector through the normal command funnel, so
  // confirm/cancel/active state/focus behave exactly like the shipped family.
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('the two glyphs exist, are dedicated, and stay apart from the shipped set', async () => {
  const icons = await readModule('icons.tsx');
  const map = icons.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.jsx');
  const defined = new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
  const panel = annotationPanel();
  const dedicated = { DIMBASELINE: 'dimbaseline', DIMCONTINUE: 'dimcontinue' };
  const used = [];
  for (const { id } of TOOLS) {
    const item = panel.items.find(candidate => candidate.id === id);
    assert.ok(item.icon, `${id} must carry an icon`);
    assert.equal(item.icon, dedicated[id], `${id} must use its dedicated dimension glyph`);
    // An unknown name makes <Icon> silently fall back to a dot, so the name must
    // really exist in the set, and the catalog must reuse the same name.
    assert.ok(defined.has(item.icon), `${id} icon '${item.icon}' must exist in icons.jsx`);
    assert.equal(COMMAND_CATALOG.find(command => command.id === id).icon, item.icon, id);
    used.push(item.icon);
  }
  assert.equal(new Set(used).size, used.length, 'the two new tools must be visually distinguishable');

  // Both glyphs are drawn in the family's 24px / 1.6px stroke language: the shared
  // <Icon> component owns the stroke width, and a glyph that hard-coded its own
  // width or fill would drift from its neighbours.
  const icon = icons.match(/strokeWidth="([\d.]+)"/);
  assert.ok(icon, 'expected the shared stroke width on <Icon>');
  assert.equal(icon[1], '1.6');
  for (const { id, icon: glyph } of [
    { id: 'DIMBASELINE', icon: 'dimbaseline' },
    { id: 'DIMCONTINUE', icon: 'dimcontinue' },
  ]) {
    const body = map[1].match(new RegExp(`\\n\\s*${glyph}:\\s*(<><[\\s\\S]*?</>),`));
    assert.ok(body, `expected a ${glyph} glyph`);
    assert.doesNotMatch(body[1], /stroke-width|fill=|stroke="/, `${id} must inherit the shared stroke language`);
    // Only strokes: no circled point or boxed target, which would read as a
    // radial/hatch glyph instead of a dimension row.
    assert.doesNotMatch(body[1], /<circle|<rect|<ellipse/, `${id} must draw dimension strokes only`);
  }

  // Baseline = one shared origin extension line plus two stacked rows, each with
  // an arrowhead pointing back into that line: the second row is what keeps it
  // off `dimlinear` (one row, two extension lines, arrowheads at both ends).
  const baseline = map[1].match(/\n\s*dimbaseline:\s*(<><[\s\S]*?<\/>),/);
  assert.ok(baseline, 'expected a dimbaseline glyph');
  assert.equal((baseline[1].match(/<path /g) ?? []).length, 4, 'dimbaseline = one extension line, two rows, two arrowheads');
  assert.equal((baseline[1].match(/M4 [\d.]+h/g) ?? []).length, 2, 'dimbaseline draws two rows from the shared origin');
  assert.equal((baseline[1].match(/M4 4\.5v15/g) ?? []).length, 1, 'dimbaseline shares one extension line, never two');
  assert.equal((baseline[1].match(/M7 /g) ?? []).length, 2, 'each row carries its own arrowhead into the shared line');
  // The rows really are stacked rows of different length, so the mark reads as a
  // stack rather than one measured span.
  const rowLengths = [...baseline[1].matchAll(/M4 [\d.]+h([\d.]+)/g)].map(match => Number(match[1]));
  assert.equal(rowLengths.length, 2);
  assert.notEqual(rowLengths[0], rowLengths[1], 'the rows end at different points, like a real baseline stack');

  // Continue = three extension lines, one collinear row cut by them, and an
  // outward arrowhead at each end. The joint's extension line runs past the two
  // outer ends, which is what separates a chained pair from one symmetric span.
  const continueGlyph = map[1].match(/\n\s*dimcontinue:\s*(<><[\s\S]*?<\/>),/);
  assert.ok(continueGlyph, 'expected a dimcontinue glyph');
  assert.equal((continueGlyph[1].match(/<path /g) ?? []).length, 4, 'dimcontinue = three extension lines, one row, two arrowheads');
  assert.equal((continueGlyph[1].match(/M4 12h16/g) ?? []).length, 1, 'dimcontinue keeps one collinear row, never stacked rows');
  const chain = continueGlyph[1].match(/d="M4 7v10M(\d+(?:\.\d+)?) 4\.5v15M(\d+(?:\.\d+)?) 7v10"/);
  assert.ok(chain, 'dimcontinue draws three extension lines with the joint between the two ends');
  assert.ok(
    Number(chain[1]) > 4 && Number(chain[1]) < Number(chain[2]),
    'the joint sits between the outer ends, so the row is cut into two segments',
  );
  assert.equal((continueGlyph[1].match(/l-?3 2\.3/g) ?? []).length, 2, 'an outward arrowhead on each end of the row');

  // The shipped seven keep their own glyphs, so the family stays nine distinct marks.
  const familyIcons = FAMILY.map(id => panel.items.find(item => item.id === id).icon);
  assert.equal(new Set(familyIcons).size, familyIcons.length, 'the dimension family glyphs must stay mutually distinct');
  assert.notEqual(panel.items.find(item => item.id === 'DIMBASELINE').icon, 'dimlinear');
  assert.notEqual(panel.items.find(item => item.id === 'DIMCONTINUE').icon, 'dimlinear');
  assert.notEqual(panel.items.find(item => item.id === 'DIMCONTINUE').icon, 'dimbaseline');
});

test('the base pick comes first, then the point step repeats while the host accepts points', async () => {
  for (const { toolKey, point } of TOOLS) {
    // Before the base is picked the chip names that pick. The base pick is an
    // entity-pick step counted as the first captured point (the same shape
    // DIMRADIUS/DIMCENTER already have), so "no points yet" means "no base yet".
    assert.equal(toolStepPromptKey({ key: toolKey, points: 0 }), BASE, toolKey);
    // After the base pick the point step repeats for every further point. A
    // fixed-arity list would have run off its end here and claimed readiness.
    for (const points of [1, 2, 3, 5, 12]) {
      assert.equal(toolStepPromptKey({ key: toolKey, points }), point, `${toolKey} @${points}`);
    }
    // Readiness is the host's `ready`, never a local point count.
    assert.equal(toolStepPromptKey({ key: toolKey, points: 4, ready: true }), READY, toolKey);
    // A capture holding only the base pick is refused by the command (zero
    // measured points), so even a `ready: true` there must not claim a dimension
    // is defined.
    assert.equal(toolStepPromptKey({ key: toolKey, points: 1, ready: true }), point, toolKey);
    // A malformed or absent count is read as "no points yet", never a later step.
    for (const malformed of [undefined, '2', -1, 1.5, Number.NaN]) {
      assert.equal(toolStepPromptKey({ key: toolKey, points: malformed }), BASE, `${toolKey} ${malformed}`);
    }
  }
  // The shared base pick is one step; the measured-point step is per command.
  assert.equal(TOOLS[0].point, 'tool.prompt.dim.baseline_point');
  assert.equal(TOOLS[1].point, 'tool.prompt.dim.continue_point');
  assert.notEqual(TOOLS[0].point, TOOLS[1].point);
  // The status bar resolves the chip from this one function and swaps the numeric
  // count for the open-ended copy while the host reports `openEnded`, so no fixed
  // complement is ever implied.
  const statusbar = await readModule('components/StatusBar.tsx');
  assert.ok(statusbar.includes('const toolStepKey = tool ? toolStepPromptKey(tool) : null;'));
  assert.ok(statusbar.includes('const toolOpenEnded = tool?.openEnded === true;'));
});

test('the fixed-arity prompt table cannot express these tools and does not pretend to', () => {
  // The ordered list is indexed by the captured count, so a two-entry list would
  // claim "ready" after the first free point. Neither tool may live there.
  for (const { toolKey } of TOOLS) {
    assert.equal(TOOL_STEP_PROMPTS[toolKey], undefined, `${toolKey} must not be a fixed-arity list`);
    // Neither overrides the ready copy: both commit real DIMENSION entities, so
    // the shared "dimension defined" copy is correct for them.
    assert.equal(TOOL_READY_PROMPTS[toolKey], undefined, `${toolKey} must fall through to the shared ready copy`);
    const entry = OPEN_ENDED_PROMPTS[toolKey];
    assert.ok(entry, `${toolKey} must have an open-ended prompt entry`);
    assert.equal(entry.select, BASE, toolKey);
    assert.equal(Object.keys(entry).sort().join(','), 'point,select', `${toolKey} names both states only`);
  }
  assert.deepEqual(Object.keys(OPEN_ENDED_PROMPTS).sort(), ['draw.dimbaseline', 'draw.dimcontinue']);
  // The ready copy really does claim a dimension — that is why it is shared and
  // why neither tool may inherit the centre-mark (plain geometry) copy.
  assert.match(en[READY], /dimension/i);
  assert.match(zh[READY], /标注/);
  assert.notEqual(toolStepPromptKey({ key: TOOLS[0].toolKey, points: 3, ready: true }), 'tool.prompt.dim.center_ready');
});

test('every new key resolves in both catalogs and the copy names the base pick and the Enter finish', async () => {
  for (const { id, toolKey, point } of TOOLS) {
    for (const key of [BASE, point, `draw.kind.${id.toLowerCase()}`, `tool.${toolKey}`]) {
      for (const locale of [zh, en]) {
        assert.equal(typeof locale[key], 'string', `missing ${key}`);
        assert.ok(locale[key].trim().length > 0, `empty ${key}`);
      }
    }
    // The button label and the active-tool chip are the same string a CAD user
    // sees, and the base-pick step really asks for an existing dimension.
    assert.equal(en[`draw.kind.${id.toLowerCase()}`], en[`tool.${toolKey}`], id);
    assert.equal(zh[`draw.kind.${id.toLowerCase()}`], zh[`tool.${toolKey}`], id);
    assert.match(en[BASE], /base/i, id);
    assert.match(en[BASE], /dimension/i, id);
    assert.match(zh[BASE], /基准/, id);
    // v1 accepts a **linear** base only, so the pick step names that kind instead
    // of promising every dimension kind (a pick the command would refuse).
    assert.match(en[BASE], /linear/i, id);
    assert.match(zh[BASE], /线性/, id);
    // The point step names the honest finish gesture and never a fixed point
    // count: the command takes as many points as the user picks.
    assert.match(en[point], /Enter/i, id);
    assert.match(zh[point], /回车/, id);
    assert.doesNotMatch(en[point], /\d/, `${id} must not promise a numeric point count`);
    assert.doesNotMatch(zh[point], /\d/, `${id} must not promise a numeric point count`);
    assert.doesNotMatch(en[BASE], /\d/, `${id} must not promise a numeric point count in the pick step`);
  }
  // The open-ended table references exactly the contract's prompt keys: no
  // invented key, no reuse of another tool's step, and no duplicate of the shared
  // ready copy.
  const table = await readModule('tools/tool-open-ended.ts');
  const referenced = new Set([...table.matchAll(/'(tool\.prompt\.[a-z0-9_.]+)'/g)].map(match => match[1]));
  assert.deepEqual(
    [...referenced].sort(),
    [BASE, 'tool.prompt.dim.baseline_point', 'tool.prompt.dim.continue_point'].sort(),
  );
  // Every one of them resolves in both locales, and the fall-through ready copy
  // is the one the family already ships (never a new duplicate).
  for (const key of referenced) {
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
  }
  assert.equal(referenced.has('tool.prompt.dim.center_ready'), false);
  assert.equal(typeof en[READY], 'string');
  assert.equal(typeof zh[READY], 'string');
});
