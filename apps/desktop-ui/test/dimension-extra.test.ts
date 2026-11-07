// Dimension extras surface contract (AutoCAD 2026), DIMROTATED.
//
// DIMROTATED uses the linear pair's three-pick capture and adds one typed
// parameter (`dimRotation`, degrees); DIMSPACE is a selection-based command that
// re-spaces existing linear dimensions. Both are router selectors: the ribbon,
// the catalog, the command line, the prompt chip and the parameter strip must all
// agree with the host's vocabulary and parameter projections, never with a local
// mirror.
import test from 'node:test';
import assert from 'node:assert/strict';
import { TOOL_STEP_PROMPTS, toolStepPromptKey } from '../src/cad/cad.ts';
import {
  admitCommandText,
  filterCommandSuggestions,
  isSupportedCommand,
  setCommandCatalog,
  toolLabelKey,
} from '../src/commands/commands.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { SELECTION_REQUIREMENTS } from '../src/commands/gating.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import {
  TOOL_PARAMETER_SPECS,
  TOOL_PARAMETERS,
} from '../src/tools/tool-parameter-specs.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

// The router vocabulary slice these commands belong to. Admission — not a local
// flag — is what enables their affordances.
const HOST_NAMES = [
  'DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER', 'DIMANGULAR',
  'DIMORDINATE', 'DIMBASELINE', 'DIMCONTINUE', 'DIMROTATED',
];
setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };

function annotationPanel() {
  const annotate = RIBBON_TABS.find(tab => tab.id === 'annotate');
  assert.ok(annotate, 'the annotate tab must exist');
  const panel = annotate.panels.find(entry => entry.id === 'annotation');
  assert.ok(panel, 'the annotate tab must keep its annotation panel');
  return panel;
}

test('DIMROTATED is an annotate affordance appended after the shipped dimension rows', () => {
  const panel = annotationPanel();
  // The nine shipped dimension buttons keep their slots; the two new rows are
  // appended after DIMCONTINUE so no already-shipped button moves.
  const ids = panel.items.map(item => item.id);
  assert.ok(ids.indexOf('DIMROTATED') > ids.indexOf('DIMCONTINUE'));
  for (const id of ['DIMROTATED']) {
    const item = panel.items.find(candidate => candidate.id === id);
    assert.ok(item, `annotate/annotation must carry the ${id} affordance`);
    assert.equal(item.command, id);
    assert.equal(item.labelKey, `draw.kind.${id.toLowerCase()}`);
    assert.equal(item.large, true, `${id} reads as part of the dimension set`);
    assert.equal(item.action, undefined, `${id} must be a router command`);
    assert.equal(item.variants, undefined, `${id} must not imply an options menu`);
    const entry = COMMAND_CATALOG.find(command => command.id === id);
    assert.ok(entry, `${id} must be in the command catalog`);
    assert.equal(entry.labelKey, item.labelKey);
    assert.equal(entry.group, 'annotate');
    // Only the annotate panel dispatches it.
    for (const tab of RIBBON_TABS) {
      for (const other of tab.panels) {
        if (other === panel) continue;
        assert.equal(
          other.items.some(candidate => candidate.command === id),
          false,
          `${tab.id}/${other.id} must not dispatch ${id}`,
        );
      }
    }
  }
  const ribbonIds = new Set(ribbonCommandIds());
  assert.ok(ribbonIds.has('DIMROTATED'));
});

test('DIMROTATED is admitted only through the loaded router vocabulary', async () => {
  for (const id of ['DIMROTATED']) {
    assert.equal(isSupportedCommand(id), true, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'admitted', id);
    const suggestion = filterCommandSuggestions(id).find(candidate => candidate.id === id);
    assert.ok(suggestion, `${id} must be a command-line suggestion`);
    assert.equal(suggestion.supported, true, id);
    setCommandCatalog([]);
    assert.equal(isSupportedCommand(id), false, id);
    assert.equal(admitCommandText(id, { status: loaded }).kind, 'unsupported', id);
    setCommandCatalog(HOST_NAMES);
  }
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(
    ribbon.includes('if (item.command) return shell.runCommand(item.command);'),
    'ribbon items dispatch through shell.runCommand, never an action',
  );
});

test('the DIMROTATED glyph exists, is dedicated, and stays apart from the shipped set', async () => {
  const icons = await readModule('icons.tsx');
  const map = icons.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  const defined = new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
  const panel = annotationPanel();
  const used = [];
  for (const id of ['DIMROTATED']) {
    const item = panel.items.find(candidate => candidate.id === id);
    const glyph = id.toLowerCase();
    assert.equal(item.icon, glyph, `${id} must use its dedicated glyph`);
    assert.ok(defined.has(glyph), `${id} icon '${glyph}' must exist in icons.tsx`);
    assert.equal(COMMAND_CATALOG.find(command => command.id === id).icon, item.icon, id);
    used.push(glyph);
  }
  assert.equal(new Set(used).size, used.length);
  // Neither may reuse a shipped dimension glyph.
  const shipped = [
    'dimlinear', 'dimaligned', 'dimradius', 'dimdiameter', 'dimcenter',
    'dimangular', 'dimordinate', 'dimbaseline', 'dimcontinue',
  ];
  for (const glyph of used) assert.equal(shipped.includes(glyph), false, glyph);
});

test('DIMROTATED names the same three picks and never claims the pick sets the angle', () => {
  assert.deepEqual(TOOL_STEP_PROMPTS['draw.dimrotated'], [
    'tool.prompt.dim.origin_first',
    'tool.prompt.dim.origin_second',
    'tool.prompt.dim.location_rotated',
  ]);
  // The origin sentences are shared with DIMLINEAR; the location sentence is its
  // own, because DIMLINEAR's location pick chooses horizontal vs vertical while
  // DIMROTATED's only positions the line (the angle is the parameter chip).
  assert.equal(
    TOOL_STEP_PROMPTS['draw.dimrotated'][0],
    TOOL_STEP_PROMPTS['draw.dimlinear'][0],
  );
  assert.equal(
    TOOL_STEP_PROMPTS['draw.dimrotated'][1],
    TOOL_STEP_PROMPTS['draw.dimlinear'][1],
  );
  assert.notEqual(
    TOOL_STEP_PROMPTS['draw.dimrotated'][2],
    TOOL_STEP_PROMPTS['draw.dimlinear'][2],
  );
  for (let points = 0; points < 3; points += 1) {
    assert.equal(
      toolStepPromptKey({ key: 'draw.dimrotated', points }),
      TOOL_STEP_PROMPTS['draw.dimrotated'][points],
    );
  }
  // Readiness is the host's `ready`, and the shared dimension ready copy is
  // correct (a real DIMENSION entity is created).
  assert.equal(toolStepPromptKey({ key: 'draw.dimrotated', points: 3, ready: true }), 'tool.prompt.dim.ready');
  // A malformed count is read as "no points yet", never a later step.
  for (const malformed of [undefined, '2', -1, 1.5, Number.NaN]) {
    assert.equal(
      toolStepPromptKey({ key: 'draw.dimrotated', points: malformed }),
      'tool.prompt.dim.origin_first',
    );
  }
});

test('DIMROTATED owns the one dimRotation parameter row, mirrored from the host', () => {
  assert.deepEqual(TOOL_PARAMETERS['draw.dimrotated'], {
    command: 'DIMROTATED',
    keys: ['dimRotation'],
  });
  const spec = TOOL_PARAMETER_SPECS.dimRotation;
  assert.ok(spec, 'dimRotation must have a control spec');
  assert.equal(spec.kind, 'number');
  assert.equal(spec.statusKey, 'dimRotation');
  assert.equal(spec.step, 15);
  assert.equal(spec.typed, true, 'an arbitrary angle must be typeable');
  // The host accepts any finite degrees value (no wrap/normalisation).
  for (const value of [0, 30, -30, 90, 360, 390, 1e6]) {
    assert.equal(spec.offered(value), true, `${value} must be offered`);
  }
  for (const value of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
    assert.equal(spec.offered(value), false, `${value} must not be offered`);
  }
  assert.equal(toolLabelKey('draw.dimrotated'), 'tool.draw.dimrotated');
});

test('every new key resolves in both catalogs and the copy states the boundaries', async () => {
  const keys = [
    'draw.kind.dimrotated', 'tool.draw.dimrotated', 'tool.prompt.dim.location_rotated',
    'status.tool_parameter.dim_rotation', 'status.tool_parameter.dim_rotation.hint',
  ];
  for (const key of keys) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
  }
  assert.equal(en['draw.kind.dimrotated'], en['tool.draw.dimrotated']);
  assert.equal(zh['draw.kind.dimrotated'], zh['tool.draw.dimrotated']);
  // The rotation prompt names the chip, and the hint says the angle is not
  // wrapped — the two facts a reader needs before typing a value.
  assert.match(en['tool.prompt.dim.location_rotated'], /angle/i);
  assert.match(zh['tool.prompt.dim.location_rotated'], /角度/);
  assert.match(en['status.tool_parameter.dim_rotation.hint'], /not wrapped|not normalised/i);
  assert.match(zh['status.tool_parameter.dim_rotation.hint'], /不做回绕/);
});
