// HATCH user-facing surface contract (ADR 0018 decision D, P6b; gradient fill
// channel 2026-10-27).
//
// HATCH is a **pre-selection** tool: the user selects the closed boundary
// objects first (circle, full ellipse or closed polyline) and then runs the
// command. Its point arity is the zero-point exact complement, so the chip has
// no point step: it asks for the boundary while the host reports the tool as not
// ready, and says the boundary is ready once the host's own `ready` flag turns
// true. Ten parameters are numeric/flag codes on the wire; three closed sets
// (pattern type, style, gradient kind) carry `choices` so a control can only reach
// implemented codes, and two of them are read **by name**: the predefined pattern
// (three transcribed entries, still the host's 1-based index on the wire) and the
// gradient kind (Linear / Spherical). The two gradient stops are integer ACI
// indices shown as colour swatches — the wire stays a number.
//
// Nothing here may imply the unimplemented forms — internal-point boundary
// selection, custom pattern names/`.pat` files, associativity. LINE/ARC boundary
// edges **are** stitched into closed loops (as exact bulge arcs) and the copy says
// so. The **three** predefined patterns the host transcribed (ANSI31, ANSI32,
// ANSI37) and the two-colour gradient `LINEAR`/`SPHERICAL` *are* implemented and
// are named by the catalogs; the allow-list checks at the bottom keep every other
// pattern name out and keep the domain's `CYLINDER` spelling out. Every
// user-visible string comes from the shared catalogs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { PRESELECT_PROMPTS, TOOL_READY_PROMPTS, TOOL_STEP_PROMPTS, toolStepPromptKey } from '../src/cad/cad.ts';
import { setToolParameterFlag, setToolParameterNumber, TOOL_PARAMETER_SPECS, TOOL_PARAMETERS, toolParameterViews } from '../src/tools/tool-parameters.ts';
import {
  admitCommandText,
  CAPABILITY_GATES,
  COMMAND_CATALOG,
  filterCommandSuggestions,
  GESTURE_ONLY_COMMANDS,
  isSupportedCommand,
  setCommandCatalog,
  toolLabelKey,
} from '../src/commands/commands.ts';
import { RIBBON_TABS, ribbonCommandIds } from '../src/ribbon/ribbon-model.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

// The router's real vocabulary carries HATCH; admission — not a local flag — is
// what enables the button.
const HOST_NAMES = ['HATCH'];
setCommandCatalog(HOST_NAMES);

const loaded = { documentLabel: 'fixture.dwg', entityCount: 3, loading: false, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: true, canPlot: true, canFit: true };

const HATCH = {
  id: 'HATCH',
  labelKey: 'draw.kind.hatch',
  icon: 'hatch',
  toolKey: 'draw.hatch',
  toolLabelKey: 'tool.draw.hatch',
  idle: 'tool.prompt.hatch.select_boundary',
  ready: 'tool.prompt.hatch.ready',
  params: [
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
  ],
};

// The host projects these ten values **only** while HATCH is active; the defaults
// are the host's own (`0` solid, pattern index `1` = ANSI31, gradient kind `0`
// Linear, gradient angle `0`, gradient stops `1` red / `5` blue, spacing `1`,
// angle `0`, double false, style `0` Normal).
const HATCH_PARAMS = Object.freeze({
  hatchPatternType: 0,
  hatchPattern: 1,
  hatchGradientKind: 0,
  hatchGradientAngle: 0,
  hatchGradientColorA: 1,
  hatchGradientColorB: 5,
  hatchSpacing: 1,
  hatchAngle: 0,
  hatchDouble: false,
  hatchStyle: 0,
});

const hatchStatus = ({ params = {}, ready = false, points = 0 } = {}) => ({
  tool: {
    key: HATCH.toolKey,
    points,
    ready,
    params: { ...HATCH_PARAMS, ...params },
  },
});

const parameterStatus = params => hatchStatus({ params });

const views = (status, gate = () => true) => toolParameterViews(status, gate);
const viewOf = (status, key) => views(status).find(view => view.key === key);

function drawPanel(tabId) {
  const tab = RIBBON_TABS.find(candidate => candidate.id === tabId);
  assert.ok(tab, `${tabId} tab must exist`);
  const panel = tab.panels.find(entry => entry.id === 'draw');
  assert.ok(panel, `${tabId} must keep its draw panel`);
  return panel;
}

test('HATCH closes the draw command group on both draw panels, as a router command', async () => {
  for (const tabId of ['home', 'draw']) {
    const panel = drawPanel(tabId);
    // HATCH still closes the draw group. MTEXT (2026-10-26) and MLEADER
    // (ADR 0019 P7c) are appended after TEXT — their annotation neighbours — so
    // they join the tail without moving HATCH:
    // `OFFSET` → `TEXT` → `MTEXT` → `MLEADER` → `HATCH` is the fixed order, and
    // HATCH stays last.
    assert.deepEqual(
      panel.items.slice(-5).map(item => item.id),
      ['OFFSET', 'TEXT', 'MTEXT', 'MLEADER', 'HATCH'],
      tabId,
    );
    const item = panel.items.at(-1);
    assert.equal(item.command, 'HATCH');
    assert.equal(item.labelKey, HATCH.labelKey);
    assert.equal(item.icon, HATCH.icon);
    // A small button, like the other secondary draw tools: no variant popover,
    // so nothing can offer the unimplemented pattern-name/gradient forms.
    assert.equal(item.large, undefined, `${tabId}: HATCH is not a primary draw button`);
    assert.equal(item.variants, undefined);
    assert.equal(item.action, undefined, 'HATCH must be a router command, never a shell action');
  }
  // The annotate dimension family is untouched: its seven commands still lead
  // the annotation panel in the same fixed order and HATCH is not among them.
  const annotate = RIBBON_TABS.find(tab => tab.id === 'annotate');
  const annotation = annotate.panels.find(panel => panel.id === 'annotation');
  assert.deepEqual(annotation.items.slice(0, 7).map(item => item.id), [
    'DIMLINEAR', 'DIMALIGNED', 'DIMRADIUS', 'DIMDIAMETER', 'DIMCENTER', 'DIMANGULAR', 'DIMORDINATE',
  ]);
  assert.equal(annotation.items.some(item => item.id === 'HATCH'), false);
  // No other panel dispatches HATCH, and the ribbon reaches it for the catalogue
  // honesty sweep.
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      if (panel.id === 'draw') continue;
      assert.equal(panel.items.some(item => item.command === 'HATCH'), false, `${tab.id}/${panel.id}`);
    }
  }
  assert.ok(new Set(ribbonCommandIds()).has('HATCH'));
  // The renderer dispatches through the one command funnel (Enter/Escape and
  // admission behave exactly like the other draw commands).
  const ribbon = await readModule('components/Ribbon.tsx');
  assert.ok(ribbon.includes('if (item.command) return shell.runCommand(item.command);'));
});

test('HATCH is a catalogued draw command whose enable state comes only from router admission', () => {
  const entry = COMMAND_CATALOG.find(command => command.id === 'HATCH');
  assert.ok(entry, 'HATCH must be in the command catalog');
  assert.equal(entry.labelKey, HATCH.labelKey);
  assert.equal(entry.icon, HATCH.icon);
  assert.equal(entry.group, 'draw');
  // It sits with the draw family in the AutoCAD-style listing, after SPLINE.
  const order = COMMAND_CATALOG.map(command => command.id);
  assert.ok(order.indexOf('HATCH') > order.indexOf('SPLINE'));
  assert.ok(order.indexOf('HATCH') < order.indexOf('MOVE'));
  // The active-tool chip key is what the host's `tool.key` maps onto.
  assert.equal(toolLabelKey(HATCH.toolKey), HATCH.toolLabelKey);

  // Router admission is the single gate.
  assert.equal(isSupportedCommand('hatch'), true);
  assert.equal(admitCommandText('HATCH', { status: loaded }).kind, 'admitted');
  assert.equal(admitCommandText('HATCH', { status: null }).kind, 'unavailable');
  assert.equal(admitCommandText('HATCH', { status: { ...loaded, loading: true } }).kind, 'unavailable');
  setCommandCatalog([]);
  assert.equal(isSupportedCommand('HATCH'), false);
  assert.equal(admitCommandText('HATCH', { status: loaded }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
  // It is neither capability-gated nor gesture-only: only the host vocabulary
  // gates it.
  assert.equal(CAPABILITY_GATES.HATCH, undefined);
  assert.equal(GESTURE_ONLY_COMMANDS.includes('HATCH'), false);
  // The command line autocompletes it with the same admission state.
  const suggestion = filterCommandSuggestions('HATCH').find(candidate => candidate.id === 'HATCH');
  assert.ok(suggestion);
  assert.equal(suggestion.supported, true);
  assert.equal(suggestion.labelKey, HATCH.labelKey);
});

test('a HATCH tool has exactly ten parameter spec rows in the strip order', () => {
  assert.deepEqual(TOOL_PARAMETERS['draw.hatch'], {
    command: 'HATCH',
    keys: HATCH.params,
  });
  const expected = {
    hatchPatternType: { kind: 'number', statusKey: 'hatchPatternType', step: 1, choices: [0, 1, 2, 3] },
    hatchPattern: { kind: 'number', statusKey: 'hatchPattern', step: 1, choices: [1, 2, 3] },
    hatchGradientKind: { kind: 'number', statusKey: 'hatchGradientKind', step: 1, choices: [0, 1] },
    hatchGradientAngle: { kind: 'number', statusKey: 'hatchGradientAngle', step: 15 },
    // A colour stop is a bare integer ACI index on the wire, shown as a swatch. Its
    // offered set starts at 1: ACI 0 is ByBlock, not a colour.
    hatchGradientColorA: { kind: 'number', statusKey: 'hatchGradientColorA', step: 1, swatch: true },
    hatchGradientColorB: { kind: 'number', statusKey: 'hatchGradientColorB', step: 1, swatch: true },
    hatchSpacing: { kind: 'number', statusKey: 'hatchSpacing', step: 0.5 },
    hatchAngle: { kind: 'number', statusKey: 'hatchAngle', step: 15 },
    hatchDouble: { kind: 'flag', statusKey: 'hatchDouble' },
    hatchStyle: { kind: 'number', statusKey: 'hatchStyle', step: 1, choices: [0, 1, 2] },
  };
  for (const key of HATCH.params) {
    const spec = TOOL_PARAMETER_SPECS[key];
    assert.ok(spec, `no spec for ${key}`);
    assert.equal(spec.kind, expected[key].kind, key);
    assert.equal(spec.statusKey, expected[key].statusKey, key);
    assert.equal(spec.step, expected[key].step, key);
    assert.equal(spec.swatch, expected[key].swatch, key);
    assert.equal(spec.labelKey, `status.tool_parameter.${key.replace(/[A-Z]/g, c => `_${c.toLowerCase()}`)}`, key);
    assert.equal(spec.hintKey, `${spec.labelKey}.hint`, key);
    if (expected[key].choices) assert.deepEqual([...spec.choices], expected[key].choices, key);
    else assert.equal(spec.choices, undefined, `${key} is not a closed set`);
  }
  // The views render in the strip order, each with its own label from the shared
  // catalogs — no hand-written row wording.
  const rendered = views(hatchStatus());
  assert.deepEqual(rendered.map(view => view.key), HATCH.params);
  for (const view of rendered) {
    assert.equal(typeof zh[view.labelKey], 'string', view.labelKey);
    assert.equal(typeof en[view.labelKey], 'string', view.labelKey);
    assert.equal(typeof zh[view.hintKey], 'string', view.hintKey);
    assert.equal(typeof en[view.hintKey], 'string', view.hintKey);
    assert.equal(view.disabled, false);
  }
  // Exactly the two colour stops are swatches; no other row paints a colour.
  assert.deepEqual(
    rendered.filter(view => view.kind === 'number' && view.swatch).map(view => view.key),
    ['hatchGradientColorA', 'hatchGradientColorB'],
  );
  // The group is gated by HATCH's own command, never a hard-coded id.
  const asked = [];
  views(hatchStatus(), command => { asked.push(command); return true; });
  assert.deepEqual(asked, ['HATCH']);
  assert.equal(views(hatchStatus(), () => false)[0].disabled, true);
});

test('the closed sets stop at the implemented codes and never offer another', () => {
  // Pattern type: 0 solid, 1 user-defined, 2 predefined, 3 gradient. From the
  // default solid, `-` is disabled and `+` reaches user-defined; … ; from gradient
  // `+` is disabled (the raw target stays 4 but is never offered, so 4 cannot be
  // sent).
  const solid = viewOf(hatchStatus(), 'hatchPatternType');
  assert.equal(solid.value, 0);
  assert.equal(solid.canDecrement, false);
  assert.equal(solid.increment, 1);
  assert.equal(solid.canIncrement, true);
  const userDefined = viewOf(parameterStatus({ hatchPatternType: 1 }), 'hatchPatternType');
  assert.equal(userDefined.value, 1);
  assert.equal(userDefined.decrement, 0);
  assert.equal(userDefined.canDecrement, true);
  assert.equal(userDefined.increment, 2);
  assert.equal(userDefined.canIncrement, true, 'the predefined code 2 is reachable');
  const predefined = viewOf(parameterStatus({ hatchPatternType: 2 }), 'hatchPatternType');
  assert.equal(predefined.value, 2);
  assert.equal(predefined.decrement, 1);
  assert.equal(predefined.canDecrement, true);
  assert.equal(predefined.increment, 3);
  assert.equal(predefined.canIncrement, true, 'the gradient code 3 is reachable');
  const gradient = viewOf(parameterStatus({ hatchPatternType: 3 }), 'hatchPatternType');
  assert.equal(gradient.value, 3);
  assert.equal(gradient.canDecrement, true);
  assert.equal(gradient.increment, 4);
  assert.equal(gradient.canIncrement, false, 'the pattern-type set ends at gradient (3)');
  // Predefined pattern index: 1 ANSI31, 2 ANSI32, 3 ANSI37 — the host table
  // length is the upper bound, so no offered value can be out of range.
  const ansi31 = viewOf(hatchStatus(), 'hatchPattern');
  assert.equal(ansi31.value, 1);
  assert.equal(ansi31.canDecrement, false);
  assert.equal(ansi31.increment, 2);
  assert.equal(ansi31.canIncrement, true);
  const ansi37 = viewOf(parameterStatus({ hatchPattern: 3 }), 'hatchPattern');
  assert.equal(ansi37.value, 3);
  assert.equal(ansi37.decrement, 2);
  assert.equal(ansi37.canDecrement, true);
  assert.equal(ansi37.increment, 4);
  assert.equal(ansi37.canIncrement, false, 'there is no fourth predefined pattern');
  // Style: 0 Normal, 1 Outer, 2 Ignore — both ends of the set are bounded.
  const normal = viewOf(hatchStatus(), 'hatchStyle');
  assert.equal(normal.canDecrement, false);
  assert.equal(normal.canIncrement, true);
  const outer = viewOf(parameterStatus({ hatchStyle: 1 }), 'hatchStyle');
  assert.equal(outer.canDecrement, true);
  assert.equal(outer.canIncrement, true);
  const ignore = viewOf(parameterStatus({ hatchStyle: 2 }), 'hatchStyle');
  assert.equal(ignore.canDecrement, true);
  assert.equal(ignore.canIncrement, false, 'the style set ends at Ignore');
  // A projected code outside any closed set renders nothing (the host contract
  // would be broken), rather than showing an unexplained code in a control.
  assert.equal(viewOf(parameterStatus({ hatchPatternType: 4 }), 'hatchPatternType'), undefined);
  assert.equal(viewOf(parameterStatus({ hatchPatternType: 2.5 }), 'hatchPatternType'), undefined);
  assert.equal(viewOf(parameterStatus({ hatchPattern: 0 }), 'hatchPattern'), undefined);
  assert.equal(viewOf(parameterStatus({ hatchPattern: 4 }), 'hatchPattern'), undefined);
  assert.equal(viewOf(parameterStatus({ hatchStyle: 3 }), 'hatchStyle'), undefined);
  assert.equal(viewOf(parameterStatus({ hatchGradientKind: 2 }), 'hatchGradientKind'), undefined);
});

test('spacing, angle and double are their own labelled controls with real bounds', () => {
  const spacing = viewOf(hatchStatus(), 'hatchSpacing');
  assert.equal(spacing.kind, 'number');
  assert.equal(spacing.value, 1);
  assert.equal(spacing.text, '1');
  assert.equal(spacing.decrement, 0.5);
  assert.equal(spacing.increment, 1.5);
  assert.equal(spacing.canDecrement, true);
  assert.equal(spacing.canIncrement, true);
  // A spacing must be > 0; 0 is never offered even though the raw target is 0.
  const half = viewOf(parameterStatus({ hatchSpacing: 0.5 }), 'hatchSpacing');
  assert.equal(half.decrement, 0);
  assert.equal(half.canDecrement, false);
  assert.equal(half.canIncrement, true);
  assert.equal(viewOf(parameterStatus({ hatchSpacing: 0 }), 'hatchSpacing'), undefined);
  assert.equal(viewOf(parameterStatus({ hatchSpacing: -2 }), 'hatchSpacing'), undefined);

  // Angle is in degrees, stepping by 15; the host accepts any finite value with
  // no implicit wrap, so the stepper has no artificial ceiling.
  const angle = viewOf(hatchStatus(), 'hatchAngle');
  assert.equal(angle.value, 0);
  assert.equal(angle.decrement, -15);
  assert.equal(angle.increment, 15);
  assert.equal(angle.canDecrement, true);
  assert.equal(angle.canIncrement, true);
  assert.equal(viewOf(parameterStatus({ hatchAngle: 45 }), 'hatchAngle').value, 45);
  assert.equal(viewOf(parameterStatus({ hatchAngle: NaN }), 'hatchAngle'), undefined);
  assert.equal(viewOf(parameterStatus({ hatchAngle: Infinity }), 'hatchAngle'), undefined);

  // Double is a plain flag; the toggle offers the opposite of the host value.
  const off = viewOf(hatchStatus(), 'hatchDouble');
  assert.equal(off.kind, 'flag');
  assert.equal(off.value, false);
  assert.equal(off.next, true);
  const on = viewOf(parameterStatus({ hatchDouble: true }), 'hatchDouble');
  assert.equal(on.value, true);
  assert.equal(on.next, false);
  // HATCH projects these ten and nothing else: no other parameter belongs to it.
  assert.deepEqual(views(hatchStatus()).map(view => view.key), HATCH.params);
});

test('HATCH parameter writes use the one channel and refuse codes outside the closed sets', async () => {
  for (const [key, accepted, refused] of [
    ['hatchPatternType', [0, 1, 2, 3], [-1, 4, 0.5, '1', null]],
    ['hatchPattern', [1, 2, 3], [0, -1, 4, 1.5, '1', null]],
    ['hatchStyle', [0, 1, 2], [-1, 3, 4, 1.5, '0', null]],
    ['hatchGradientKind', [0, 1], [-1, 2, 0.5, '0', null]],
    ['hatchSpacing', [0.5, 1, 2.5, 1e9], [0, -1, NaN, Infinity, '1', null]],
    ['hatchAngle', [0, 45, -270, 1e6], [NaN, Infinity, -Infinity, '45', null]],
    // Degrees, any finite value: the host converts to radians once and wraps
    // nothing, so a value beyond a turn is still a value it accepts.
    ['hatchGradientAngle', [0, 45, -270, 1e6], [NaN, Infinity, -Infinity, '45', null]],
    // ACI indices 1..=255. `0` is ByBlock (no colour) and the command refuses it by
    // name, so the UI never offers or sends it; a string spelling of a colour
    // (`aci:5`, `#ff0000`) has no numeric channel to travel on and is refused.
    ['hatchGradientColorA', [1, 5, 255], [0, -1, 256, 1.5, 'aci:5', '#ff0000', null]],
    ['hatchGradientColorB', [1, 7, 255], [0, -1, 256, 1.5, '5', null]],
  ]) {
    for (const number of accepted) {
      const calls = [];
      const result = await setToolParameterNumber(
        async (...args) => { calls.push(args); return { accepted: true }; },
        key,
        number,
      );
      assert.deepEqual(result, { kind: 'accepted' }, `${key} ${number}`);
      assert.deepEqual(calls, [['cad_set_tool_parameter', { parameter: key, number }]], `${key} ${number}`);
    }
    for (const number of refused) {
      const result = await setToolParameterNumber(
        () => assert.fail(`${key} ${number} reached IPC`),
        key,
        number,
      );
      assert.equal(result.kind, 'refused', `${key} ${number}`);
      assert.ok(result.error);
    }
  }
  const calls = [];
  assert.deepEqual(
    await setToolParameterFlag(async (...args) => { calls.push(args); return { accepted: true }; }, 'hatchDouble', true),
    { kind: 'accepted' },
  );
  assert.deepEqual(calls, [['cad_set_tool_parameter', { parameter: 'hatchDouble', flag: true }]]);
  assert.equal((await setToolParameterFlag(() => assert.fail('wrong kind reached IPC'), 'hatchDouble', 1)).kind, 'refused');
  assert.equal((await setToolParameterFlag(() => assert.fail('number key reached IPC'), 'hatchSpacing', true)).kind, 'refused');
});

test('the pre-selection chip asks for the boundary, then reports it ready from the host flag', async () => {
  // HATCH has no pick steps: an entry in the point-index table would have to be
  // empty (which would claim readiness immediately) or invent picks.
  assert.equal(TOOL_STEP_PROMPTS[HATCH.toolKey], undefined);
  assert.deepEqual(PRESELECT_PROMPTS[HATCH.toolKey], { idle: HATCH.idle, ready: HATCH.ready });
  // Idle: no selection yet. The point count is always 0 for this tool and must
  // not be read as "the capture is complete".
  assert.equal(toolStepPromptKey(hatchStatus({ ready: false }).tool), HATCH.idle);
  assert.equal(toolStepPromptKey({ key: HATCH.toolKey, points: 0 }), HATCH.idle);
  assert.equal(toolStepPromptKey({ key: HATCH.toolKey, points: 3, ready: false }), HATCH.idle);
  // A malformed/absent readiness never claims the selection is sufficient.
  assert.equal(toolStepPromptKey({ key: HATCH.toolKey, ready: 'true' }), HATCH.idle);
  // Ready: the host reports a non-empty selection that can be confirmed.
  assert.equal(toolStepPromptKey(hatchStatus({ ready: true }).tool), HATCH.ready);
  assert.equal(toolStepPromptKey({ key: HATCH.toolKey, points: 0, ready: true }), HATCH.ready);
  // It never borrows a dimension key: the shared dimension ready copy and the
  // centre-mark copy both claim something HATCH does not produce.
  assert.equal(TOOL_READY_PROMPTS[HATCH.toolKey], undefined);
  for (const key of [HATCH.idle, HATCH.ready]) {
    assert.notEqual(key, 'tool.prompt.dim.ready');
    assert.notEqual(key, 'tool.prompt.dim.center_ready');
    assert.equal(TOOL_STEP_PROMPTS['draw.dimlinear']?.includes(key), false);
  }
  // The other tools keep their derivations untouched.
  assert.equal(toolStepPromptKey({ key: 'draw.line', points: 0 }), null);
  assert.equal(toolStepPromptKey({ key: 'draw.dimlinear', points: 0 }), 'tool.prompt.dim.origin_first');
  assert.equal(toolStepPromptKey({ key: 'draw.dimlinear', points: 3 }), 'tool.prompt.dim.ready');
  assert.equal(toolStepPromptKey(null), null);
  // The status bar renders that key next to the tool name, so the chip really
  // carries the copy and not just a test-only value.
  const statusbar = await readModule('components/StatusBar.tsx');
  assert.ok(statusbar.includes('const toolStepKey = tool ? toolStepPromptKey(tool) : null;'));
  assert.ok(statusbar.includes('{t(toolStepKey)}'));
});

test('the HATCH copy exists in both locales and states the honest boundary', () => {
  const keys = new Set([
    HATCH.labelKey,
    HATCH.toolLabelKey,
    HATCH.idle,
    HATCH.ready,
    ...Object.values(TOOL_PARAMETER_SPECS)
      .filter(spec => spec.labelKey.startsWith('status.tool_parameter.hatch_'))
      .flatMap(spec => [spec.labelKey, spec.hintKey]),
    // The names the two choosers offer are copy too, so the allow-list below
    // covers them: a pattern name that is not one of the three implemented ones, or
    // a gradient kind that is not Linear/Spherical, can never reach a segment.
    ...Object.values(TOOL_PARAMETER_SPECS.hatchPattern.choiceLabelKeys),
    ...Object.values(TOOL_PARAMETER_SPECS.hatchGradientKind.choiceLabelKeys),
    // The colour swatch's trigger name (it is the row's own label, parameterised).
    'status.tool_parameter.choose_color',
  ]);
  for (const key of keys) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
  }
  // The button, the command line and the active-tool chip name the same tool.
  assert.equal(en['draw.kind.hatch'], en['tool.draw.hatch']);
  assert.equal(zh['draw.kind.hatch'], zh['tool.draw.hatch']);
  assert.match(en['draw.kind.hatch'], /hatch/i);
  assert.match(zh['draw.kind.hatch'], /填充/);

  // The idle prompt asks for a boundary and names the accepted kinds: closed
  // shapes plus LINE/ARC edges that stitch into a closed loop. It must never
  // imply internal-point boundary detection.
  assert.match(en[HATCH.idle], /boundary/i);
  assert.match(en[HATCH.idle], /circle/i);
  assert.match(en[HATCH.idle], /ellipse/i);
  assert.match(en[HATCH.idle], /polyline/i);
  assert.match(en[HATCH.idle], /LINE\/ARC/i);
  assert.match(en[HATCH.idle], /closed loop/i);
  assert.match(zh[HATCH.idle], /边界/);
  assert.match(zh[HATCH.idle], /圆/);
  assert.match(zh[HATCH.idle], /椭圆/);
  assert.match(zh[HATCH.idle], /多段线/);
  assert.match(zh[HATCH.idle], /直线\/圆弧/);
  assert.match(zh[HATCH.idle], /闭合环/);
  for (const [locale, pattern] of [[en, /internal point|pick inside/i], [zh, /内部点/]]) {
    assert.doesNotMatch(locale[HATCH.idle], pattern);
  }
  // The ready prompt names the boundary and Enter, not a dimension.
  assert.match(en[HATCH.ready], /boundary/i);
  assert.match(en[HATCH.ready], /Enter/);
  assert.match(zh[HATCH.ready], /边界/);
  assert.match(zh[HATCH.ready], /回车/);
  for (const locale of [zh, en]) {
    assert.doesNotMatch(locale[HATCH.ready], /dimension|标注/i);
  }

  // Pattern type names its four codes and no longer claims predefined patterns or
  // gradients are missing.
  for (const code of ['0', '1', '2', '3']) {
    assert.match(en['status.tool_parameter.hatch_pattern_type.hint'], new RegExp(`\\b${code}\\b`));
    assert.match(zh['status.tool_parameter.hatch_pattern_type.hint'], new RegExp(code));
  }
  for (const locale of [zh, en]) {
    assert.doesNotMatch(
      locale['status.tool_parameter.hatch_pattern_type.hint'],
      /not supported|unsupported|not implemented|不支持|尚未支持/i,
      'predefined patterns and gradients are implemented now',
    );
    assert.match(locale['status.tool_parameter.hatch_pattern_type.hint'], /3/);
  }
  // The predefined pattern row names all three implemented patterns in both
  // locales (it is a name chooser now), and the pattern-type hint names the
  // implemented set too.
  const IMPLEMENTED_PATTERNS = ['ANSI31', 'ANSI32', 'ANSI37'];
  for (const locale of [zh, en]) {
    for (const key of ['status.tool_parameter.hatch_pattern.hint', 'status.tool_parameter.hatch_pattern_type.hint']) {
      for (const name of IMPLEMENTED_PATTERNS) {
        assert.ok(locale[key].includes(name), `${key} must name ${name}`);
      }
    }
  }
  // Style names all three codes and their human names.
  for (const code of ['0', '1', '2']) {
    assert.match(en['status.tool_parameter.hatch_style.hint'], new RegExp(code));
    assert.match(zh['status.tool_parameter.hatch_style.hint'], new RegExp(code));
  }
  for (const name of [/Normal/, /Outer/, /Ignore/]) {
    assert.match(en['status.tool_parameter.hatch_style.hint'], name);
  }
  for (const name of [/普通/, /外部/, /忽略/]) {
    assert.match(zh['status.tool_parameter.hatch_style.hint'], name);
  }
  // Spacing is a length above zero (and the predefined pattern scale); the
  // angle is in degrees; double is the perpendicular second family.
  assert.match(en['status.tool_parameter.hatch_spacing.hint'], /greater than 0/);
  assert.match(zh['status.tool_parameter.hatch_spacing.hint'], /大于 0/);
  assert.match(en['status.tool_parameter.hatch_spacing.hint'], /HPSCALE/i);
  assert.match(zh['status.tool_parameter.hatch_spacing.hint'], /HPSCALE/);
  assert.match(en['status.tool_parameter.hatch_angle.hint'], /degrees/i);
  assert.match(zh['status.tool_parameter.hatch_angle.hint'], /度/);
  assert.match(en['status.tool_parameter.hatch_double.hint'], /perpendicular/i);
  assert.match(zh['status.tool_parameter.hatch_double.hint'], /垂直/);

  // The gradient rows state what they really write: an ACI palette index (not a
  // true colour), a degrees angle, and the two implemented kinds. The Spherical
  // note is deliberately modest — on an ordinary rectangular/circular boundary the
  // ramp is baked over the whole boundary extent, so it is not "radial".
  for (const locale of [zh, en]) {
    for (const key of ['status.tool_parameter.hatch_gradient_color_a.hint', 'status.tool_parameter.hatch_gradient_color_b.hint']) {
      assert.match(locale[key], /ACI/, `${key} must say the value is an ACI index`);
      assert.match(locale[key], /1[–-]255/, `${key} must name the real index range`);
      assert.match(locale[key], /ByBlock/, `${key} must name the refused placeholder`);
      assert.match(locale[key], /3/, `${key} must say which pattern type uses it`);
    }
    assert.match(locale['status.tool_parameter.hatch_gradient_angle.hint'], /3/);
    assert.match(locale['status.tool_parameter.hatch_gradient_kind.hint'], /3/);
    // The gradient rows are pattern-type-3 rows only, and the pattern-only rows say
    // the same about themselves.
    for (const key of ['status.tool_parameter.hatch_spacing.hint', 'status.tool_parameter.hatch_angle.hint', 'status.tool_parameter.hatch_double.hint']) {
      assert.match(locale[key], /gradient|渐变/i, `${key} must say a gradient does not use it`);
    }
  }
  // "palette index, not a true colour" — the honest limit, in both languages.
  assert.match(en['status.tool_parameter.hatch_gradient_color_a.hint'], /not a true colour/i);
  assert.match(en['status.tool_parameter.hatch_gradient_color_a.hint'], /palette index/i);
  assert.match(zh['status.tool_parameter.hatch_gradient_color_a.hint'], /不是真彩色/);
  assert.match(zh['status.tool_parameter.hatch_gradient_color_a.hint'], /调色板序号/);
  // The Spherical caveat: still offered, but never sold as a true radial ramp.
  assert.match(en['status.tool_parameter.hatch_gradient_kind.hint'], /Spherical/);
  assert.match(en['status.tool_parameter.hatch_gradient_kind.hint'], /Linear/);
  assert.match(en['status.tool_parameter.hatch_gradient_kind.hint'], /flat or elliptical/i);
  assert.match(zh['status.tool_parameter.hatch_gradient_kind.hint'], /球面/);
  assert.match(zh['status.tool_parameter.hatch_gradient_kind.hint'], /线性/);
  assert.match(zh['status.tool_parameter.hatch_gradient_kind.hint'], /椭圆/);
  // The gradient angle is a degrees value (the host converts once).
  assert.match(en['status.tool_parameter.hatch_gradient_angle.hint'], /degrees/i);
  assert.match(zh['status.tool_parameter.hatch_gradient_angle.hint'], /度/);

  // Positive allow-list: the implemented predefined set is exactly
  // {ANSI31, ANSI32, ANSI37}. Any other pattern name is unimplemented and must not
  // appear; associativity and `.pat` loading must not be claimed. Gradients **are**
  // implemented now, so the ban is narrower: only the two gradient rows and the
  // pattern-type hint may mention one, and no row may imply a gradient carries a
  // pattern name.
  const GRADIENT_KEYS = new Set([
    'status.tool_parameter.hatch_pattern_type.hint',
    'status.tool_parameter.hatch_gradient_kind',
    'status.tool_parameter.hatch_gradient_kind.hint',
    'status.tool_parameter.hatch_gradient_kind.linear',
    'status.tool_parameter.hatch_gradient_kind.spherical',
    'status.tool_parameter.hatch_gradient_angle',
    'status.tool_parameter.hatch_gradient_angle.hint',
    'status.tool_parameter.hatch_gradient_color_a',
    'status.tool_parameter.hatch_gradient_color_a.hint',
    'status.tool_parameter.hatch_gradient_color_b',
    'status.tool_parameter.hatch_gradient_color_b.hint',
    'status.tool_parameter.hatch_spacing.hint',
    'status.tool_parameter.hatch_angle.hint',
    'status.tool_parameter.hatch_double.hint',
  ]);
  for (const key of keys) {
    for (const locale of [zh, en]) {
      const text = locale[key];
      const named = text.match(/ANSI\d+/g) ?? [];
      for (const name of named) {
        assert.ok(
          IMPLEMENTED_PATTERNS.includes(name),
          `${key} in ${locale === zh ? 'zh-CN' : 'en'} names unimplemented pattern ${name}`,
        );
      }
      if (!GRADIENT_KEYS.has(key)) {
        assert.doesNotMatch(text, /gradient|渐变/i, `${key} must not claim a gradient`);
      }
      assert.doesNotMatch(text, /associat|关联/i, `${key} must not claim associativity`);
      // A `.pat` mention is only allowed as a refusal, never as a load claim.
      for (const mention of text.match(/[^.]*\.pat[^.]*/gi) ?? []) {
        assert.match(
          mention,
          /not supported|unsupported|not implemented|不支持|尚未支持/i,
          `${key} must not claim .pat loading: ${mention}`,
        );
      }
    }
  }
  // The domain's third gradient spelling is never offered: `CYLINDER` bakes as
  // spherical, so naming it would be a second name for one behaviour.
  for (const locale of [zh, en]) {
    for (const key of GRADIENT_KEYS) {
      assert.doesNotMatch(locale[key], /CYLINDER|圆柱/i, `${key} must not offer CYLINDER`);
    }
  }
});

test('the predefined pattern is a named chooser, not a raw index stepper', async () => {
  // The row's own shape: three named segments in the host's code order, the chosen
  // one marked, nothing invented.
  const row = views(parameterStatus({ hatchPatternType: 2 })).find(view => view.key === 'hatchPattern');
  assert.deepEqual(row.choices.map(choice => choice.labelKey), [
    'status.tool_parameter.hatch_pattern.ansi31',
    'status.tool_parameter.hatch_pattern.ansi32',
    'status.tool_parameter.hatch_pattern.ansi37',
  ]);
  assert.deepEqual(row.choices.map(choice => choice.value), [1, 2, 3]);
  for (const choice of row.choices) {
    assert.equal(typeof zh[choice.labelKey], 'string', choice.labelKey);
    assert.equal(typeof en[choice.labelKey], 'string', choice.labelKey);
  }
  // Under a type the index does not apply to (solid, user-defined, gradient), the
  // row is still there (the names stay discoverable) but every segment is inert —
  // the pattern-type control is what gates it, so the two rows can never disagree.
  for (const type of [0, 1, 3]) {
    const inert = views(parameterStatus({ hatchPatternType: type })).find(view => view.key === 'hatchPattern');
    assert.ok(inert, `the row stays visible under type ${type}`);
    assert.equal(inert.inactive, true);
    assert.deepEqual(inert.choices.map(choice => choice.disabled), [true, true, true]);
  }

  // The status bar routes a named row to the chooser and every other numeric row
  // to the stepper — one funnel, no duplicated controls.
  const statusbar = await readModule('components/StatusBar.tsx');
  assert.ok(statusbar.includes('if (view.choices) return <ParameterChoices'));
  assert.ok(statusbar.includes('<ParameterStepper'));
  assert.ok(statusbar.includes('onChoose={shell.stepToolParameter}'));
  assert.ok(statusbar.includes('onStep={shell.stepToolParameter}'));
  // The chooser dispatches the **code**, never the name: the wire stays the
  // host's integer index and no catalog string can leak into a parameter write.
  const controls = await readModule('components/ParameterControls.tsx');
  assert.ok(controls.includes('onChoose(view.key, choice.value)'));
  assert.ok(!controls.includes('onChoose(view.key, choice.labelKey)'));
  assert.ok(!controls.includes("onChoose(view.key, t("));
  // …and it owns no value of its own: no local state, no IPC.
  assert.ok(!controls.includes('invoke('));
  // The chosen segment is exposed and announced, and an inert row says why.
  assert.ok(controls.includes('aria-pressed={choice.selected}'));
  assert.ok(controls.includes('disabled={choice.disabled}'));
  assert.ok(controls.includes("role=\"group\""));
  assert.ok(controls.includes('aria-label={name}'));
  assert.ok(controls.includes('t(TOOL_PARAMETER_INACTIVE_KEY)'));
  // The names come from the catalog on the segment itself, not from a title only.
  assert.ok(controls.includes('{t(choice.labelKey)}'));
});

test('the hatch glyph is a dedicated boundary-with-fill icon, not a neighbour glyph', async () => {
  const icons = await readModule('icons.tsx');
  const map = icons.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.jsx');
  const defined = new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
  assert.ok(defined.has(HATCH.icon), `icons.jsx must define '${HATCH.icon}'`);
  // Ribbon and catalog agree on the dedicated name (an unknown name would make
  // <Icon> silently fall back to a dot).
  for (const tabId of ['home', 'draw']) {
    const item = drawPanel(tabId).items.find(candidate => candidate.id === HATCH.id);
    assert.equal(item.icon, HATCH.icon, tabId);
  }
  assert.equal(COMMAND_CATALOG.find(command => command.id === HATCH.id).icon, HATCH.icon);
  // `hatch` belongs to HATCH alone: no other catalogued command reuses it.
  assert.equal(COMMAND_CATALOG.filter(command => command.icon === HATCH.icon).length, 1);
  // Every draw-panel glyph is a real name, so no draw button falls back to a dot.
  for (const tabId of ['home', 'draw']) {
    for (const item of drawPanel(tabId).items) {
      assert.ok(defined.has(item.icon), `${item.id} icon '${item.icon}' must exist`);
    }
  }
  // Shape: one closed boundary plus three diagonal fill strokes, all inside the
  // loop. No dimension line, tick or arrowhead.
  const glyph = map[1].match(/\n\s*hatch:\s*(<><[\s\S]*?<\/>),/);
  assert.ok(glyph, 'expected a hatch glyph');
  assert.equal((glyph[1].match(/<rect/g) ?? []).length, 1, 'the boundary is one closed rect');
  const diagonals = [...glyph[1].matchAll(/<path d="M([\d.]+) ([\d.]+) ([\d.]+) ([\d.]+)"/g)];
  assert.equal(diagonals.length, 3, 'hatch = three parallel diagonal fill lines');
  for (const [, x1, y1, x2, y2] of diagonals) {
    assert.notEqual(x1, x2, 'each stroke is diagonal in x');
    assert.notEqual(y1, y2, 'each stroke is diagonal in y');
    for (const [x, y] of [[x1, y1], [x2, y2]]) {
      assert.ok(Number(x) >= 4 && Number(x) <= 20, `stroke x ${x} must stay inside the boundary`);
      assert.ok(Number(y) >= 4 && Number(y) <= 20, `stroke y ${y} must stay inside the boundary`);
    }
  }
  // The loop is rounded, which keeps it apart from `rectang` (a sharp rectangle
  // with corner grips); the absence of vertex dots keeps it apart from `polygon`.
  assert.match(glyph[1], /rx="1.5"/);
  assert.equal((glyph[1].match(/<circle/g) ?? []).length, 0);
  for (const neighbour of ['polygon', 'rectang', 'dimlinear', 'dimaligned', 'dimradius', 'dimdiameter', 'dimcenter', 'dimangular', 'dimordinate']) {
    assert.notEqual(HATCH.icon, neighbour);
  }
});

