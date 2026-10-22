// Active-tool parameter contract (ADR 0016 decision A): the group renders only
// for an active tool and only for parameters with a projected value; a numeric
// step dispatches exactly one `cad_set_tool_parameter` with `{parameter, number}`;
// a flag toggle dispatches `{parameter, flag}`; bounds are enforced per parameter
// (including the fill-angle rule) by disabling a step, never clamping it; the
// bounded pending delta composes and clears as before without ever moving the
// rendered number; and the removed `cad_set_polygon_sides` name appears nowhere.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  reconcileToolParameterPending,
  setToolParameterFlag,
  setToolParameterNumber,
  stepToolParameterPending,
  TOOL_PARAMETER_COMMAND,
  TOOL_PARAMETER_PENDING_CAP,
  TOOL_PARAMETER_SPECS,
  TOOL_PARAMETERS,
  toolParameterViews,
} from '../src/cad.mjs';
import { readAppSource, readCatalog, readModule } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

// Admission is the shell's command gate; every control here uses the same one.
const admitted = () => true;
const denied = () => false;
const view = (status, gate = admitted, pending = null) => toolParameterViews(status, gate, pending);
const first = (status, gate = admitted, pending = null) => view(status, gate, pending)[0];

const polygon = sides => ({ tool: { key: 'draw.polygon', points: 1, ready: false, params: { sides } } });
const fillet = radius => ({ tool: { key: 'draw.fillet', points: 0, ready: false, params: { filletRadius: radius } } });
const fillAngle = degrees => ({ tool: { key: 'draw.arraypolar', points: 0, ready: false, params: { arrayFillAngle: degrees } } });
const align = flag => ({ tool: { key: 'draw.align', points: 0, ready: false, params: { alignScale: flag } } });
const pending = (key, base, delta) => ({ key, base, delta });

test('the parameter group renders only for an active tool with a projected value', () => {
  const v = first(polygon(8));
  assert.deepEqual(v, {
    kind: 'number', key: 'polygonSides', value: 8, delta: 0, effective: 8, text: '8',
    labelKey: 'status.tool_parameter.polygon_sides',
    hintKey: 'status.tool_parameter.polygon_sides.hint',
    icon: 'polygon', disabled: false,
    decrement: 7, increment: 9, canDecrement: true, canIncrement: true,
  });
  // No tool, no map entry, or a parameter with no projected value renders
  // nothing at all — never a default.
  for (const status of [
    null,
    undefined,
    {},
    { tool: null },
    { tool: 'draw.polygon' },
    { tool: { key: 'draw.line' } },                                  // no map row
    { tool: { key: 'draw.polygon' } },                               // no params block
    { tool: { key: 'draw.polygon', params: null } },
    { tool: { key: 'draw.polygon', params: { sides: null } } },      // absent value
    { tool: { key: 'draw.polygon', params: { sides: '8' } } },       // wrong kind
    { tool: { key: 'draw.polygon', params: { sides: 4.5 } } },       // integer rule
    { tool: { key: 'draw.polygon', params: { sides: 2 } } },         // out of range
    { tool: { key: 'draw.polygon', params: { sides: 1025 } } },
    { tool: { key: 'draw.fillet', params: {} } },                    // later tool, no projection yet
  ]) {
    assert.deepEqual(view(status), []);
  }
});

test('the per-tool map and the spec table stay consistent', () => {
  // POLYGON and HATCH are reachable today; HATCH's row is its five parameters.
  assert.deepEqual(TOOL_PARAMETERS['draw.polygon'], { command: 'POLYGON', keys: ['polygonSides'] });
  assert.deepEqual(TOOL_PARAMETERS['draw.hatch'], {
    command: 'HATCH',
    keys: ['hatchPatternType', 'hatchSpacing', 'hatchAngle', 'hatchDouble', 'hatchStyle'],
  });
  // Every key a tool maps to has a spec, and every spec is reachable through the
  // map — so a later phase only adds a row (the specs already exist).
  const mapped = new Set();
  for (const entry of Object.values(TOOL_PARAMETERS)) {
    for (const key of entry.keys) {
      assert.ok(TOOL_PARAMETER_SPECS[key], `no spec for ${key}`);
      mapped.add(key);
    }
  }
  assert.deepEqual([...mapped].sort(), Object.keys(TOOL_PARAMETER_SPECS).sort());
  // The spec keys are the frozen machine keys the wire carries (ADR 0016 A).
  assert.deepEqual(Object.keys(TOOL_PARAMETER_SPECS).sort(), [
    'alignScale', 'arrayColumnSpacing', 'arrayColumns', 'arrayFillAngle', 'arrayItems',
    'arrayRotateItems', 'arrayRowSpacing', 'arrayRows', 'chamferDistanceA',
    'chamferDistanceB', 'filletRadius', 'hatchAngle', 'hatchDouble',
    'hatchPatternType', 'hatchSpacing', 'hatchStyle', 'polygonSides',
  ]);
  assert.equal(TOOL_PARAMETER_SPECS.alignScale.kind, 'flag');
  assert.equal(TOOL_PARAMETER_SPECS.arrayRotateItems.kind, 'flag');
  assert.equal(TOOL_PARAMETER_SPECS.hatchDouble.kind, 'flag');
});

test('a numeric step dispatches exactly one cad_set_tool_parameter with {parameter, number}', async () => {
  const calls = [];
  const result = await setToolParameterNumber(async (...args) => { calls.push(args); return { accepted: true }; }, 'polygonSides', 9);
  assert.deepEqual(calls, [['cad_set_tool_parameter', { parameter: 'polygonSides', number: 9 }]]);
  assert.equal(TOOL_PARAMETER_COMMAND, 'cad_set_tool_parameter');
  assert.deepEqual(result, { kind: 'accepted' });
  // Acceptance is queue admission, not completion: no value/status is returned.
  assert.equal('completed' in result, false);
  assert.equal('number' in result, false);
});

test('a flag toggle dispatches cad_set_tool_parameter with {parameter, flag}', async () => {
  const calls = [];
  const result = await setToolParameterFlag(async (...args) => { calls.push(args); return { accepted: true }; }, 'alignScale', true);
  assert.deepEqual(calls, [['cad_set_tool_parameter', { parameter: 'alignScale', flag: true }]]);
  assert.deepEqual(result, { kind: 'accepted' });
  // A flag is a real boolean, and the view offers the *opposite* of the status.
  const on = first(align(true));
  assert.equal(on.kind, 'flag');
  assert.equal(on.value, true);
  assert.equal(on.next, false);
  assert.equal(first(align(false)).next, true);
});

test('an unoffered value or a wrong kind is refused before IPC', async () => {
  // polygonSides is an integer 3..=1024; anything else never reaches IPC.
  for (const number of [2, 1025, 0, -1, 4.5, NaN, Infinity, '8', null, undefined]) {
    const result = await setToolParameterNumber(() => assert.fail('unoffered value reached IPC'), 'polygonSides', number);
    assert.equal(result.kind, 'refused');
    assert.ok(result.error);
  }
  // A flag parameter refuses a number, and an unknown key is refused too.
  assert.equal((await setToolParameterNumber(() => assert.fail('flag key reached IPC'), 'alignScale', 1)).kind, 'refused');
  assert.equal((await setToolParameterNumber(() => assert.fail('unknown key reached IPC'), 'nope', 1)).kind, 'refused');
  assert.equal((await setToolParameterFlag(() => assert.fail('number key reached IPC'), 'polygonSides', true)).kind, 'refused');
  assert.equal((await setToolParameterFlag(() => assert.fail('non-boolean reached IPC'), 'alignScale', 1)).kind, 'refused');
  // A real IPC failure is reported honestly, never as a silent success.
  for (const invoke of [
    async () => { throw Error('viewer mode'); },
    async () => ({ accepted: false }),
    async () => null,
    async () => undefined,
  ]) {
    assert.equal((await setToolParameterNumber(invoke, 'polygonSides', 6)).kind, 'failed');
    assert.equal((await setToolParameterFlag(invoke, 'alignScale', true)).kind, 'failed');
  }
});

test('numeric bounds are enforced per parameter and a step is disabled, never clamped', () => {
  // polygonSides: integer 3..=1024.
  const low = first(polygon(3));
  assert.equal(low.canDecrement, false);
  assert.equal(low.canIncrement, true);
  assert.equal(low.decrement, 2); // the raw target is not clamped, just not offered
  const high = first(polygon(1024));
  assert.equal(high.canIncrement, false);
  assert.equal(high.increment, 1025);
  // filletRadius: finite >= 0, no upper bound.
  assert.equal(first(fillet(0)).canDecrement, false);
  assert.equal(first(fillet(0)).canIncrement, true);
  assert.deepEqual(view(fillet(-1)), []); // a negative radius is not renderable
  // arrayRows: integer >= 1 (no upper bound).
  const rowsOne = toolParameterViews({ tool: { key: 'draw.arrayrect', params: { arrayRows: 1 } } }, admitted)[0];
  assert.equal(rowsOne.canDecrement, false);
  assert.equal(rowsOne.canIncrement, true);
});

test('the fill-angle stepper offers the full-circle default and never offers 0 or beyond a turn', () => {
  // 345 + 15 = 360 is a full turn and IS an offered target (0 < |angle| <= 360):
  // the `+` step is enabled so a user can land back on the default.
  const nearTop = first(fillAngle(345));
  assert.equal(nearTop.canIncrement, true);
  assert.equal(nearTop.increment, 360);
  assert.equal(nearTop.canDecrement, true);
  // 15 - 15 = 0 is refused: the `-` step is disabled (raw target kept, never
  // silently rewritten).
  const nearZero = first(fillAngle(15));
  assert.equal(nearZero.canDecrement, false);
  assert.equal(nearZero.decrement, 0);
  assert.equal(nearZero.canIncrement, true);
  // The documented default (a full turn) is renderable and can step back down; a
  // step past +360 is out of range and disabled.
  const full = first(fillAngle(360));
  assert.equal(full.value, 360);
  assert.equal(full.canDecrement, true);
  assert.equal(full.increment, 375);
  assert.equal(full.canIncrement, false, 'a target beyond +360 is not offered');
  // -360 is also an offered target (the reversed full circle).
  const negativeFull = first(fillAngle(-360));
  assert.equal(negativeFull.value, -360);
  assert.equal(negativeFull.decrement, -375);
  assert.equal(negativeFull.canDecrement, false, 'a target beyond -360 is not offered');
  assert.equal(negativeFull.canIncrement, true);
  // A partial turn steps normally in both directions.
  const partial = first(fillAngle(90));
  assert.equal(partial.increment, 105);
  assert.equal(partial.decrement, 75);
  assert.equal(partial.canIncrement, true);
  assert.equal(partial.canDecrement, true);
  // A non-finite projection renders nothing.
  assert.deepEqual(view(fillAngle(NaN)), []);
  assert.deepEqual(view(fillAngle(Infinity)), []);
});

test('the control is disabled until the owning command is admitted', () => {
  assert.equal(first(polygon(8)).disabled, false);
  assert.equal(first(polygon(8), denied).disabled, true);
  // The gate is asked for the owning tool's command, never a hard-coded id.
  const asked = [];
  toolParameterViews(polygon(8), command => { asked.push(command); return true; });
  assert.deepEqual(asked, ['POLYGON']);
});

test('two rapid steps compose while the rendered number still follows the status', () => {
  const status = polygon(8);
  // Step 1: the view still renders 8 and offers 9.
  const v1 = first(status);
  assert.equal(v1.value, 8);
  assert.equal(v1.text, '8');
  assert.equal(v1.increment, 9);
  const p1 = stepToolParameterPending(null, status, 'polygonSides', v1.increment);
  assert.deepEqual(p1, pending('polygonSides', 8, 1));
  // Step 2 (host has not echoed yet): the rendered number is *still* 8, but the
  // effective target is 9, so the next step composes to 10.
  const v2 = first(status, admitted, p1);
  assert.equal(v2.value, 8);
  assert.equal(v2.text, '8');
  assert.equal(v2.effective, 9);
  assert.equal(v2.increment, 10);
  const p2 = stepToolParameterPending(p1, status, 'polygonSides', v2.increment);
  assert.deepEqual(p2, pending('polygonSides', 8, 2));
  // The two dispatched targets were 9 then 10 — never 9 then 9, and the rendered
  // number never moved off the status value.
  assert.deepEqual([v1.increment, v2.increment], [9, 10]);
  assert.deepEqual([v1.value, v2.value], [8, 8]);
});

test('the pending delta resets when the status value changes (host caught up)', () => {
  const p = pending('polygonSides', 8, 2);
  // Still the same value: keep accumulating (same reference, so no re-render).
  assert.equal(reconcileToolParameterPending(p, polygon(8)), p);
  // The host echoed: the delta is dropped and the next step builds on the new value.
  assert.equal(reconcileToolParameterPending(p, polygon(10)), null);
  const v = first(polygon(10));
  assert.equal(v.value, 10);
  assert.equal(v.increment, 11);
});

test('the pending delta resets on tool switch, cancel, an absent value and a host error', () => {
  const p = pending('polygonSides', 8, 2);
  for (const status of [
    { tool: { key: 'draw.line' } },                                // tool switched
    { tool: { key: 'draw.polygon', params: { sides: null } } },    // value absent
    { tool: { key: 'draw.polygon', params: null } },
    { tool: { key: 'draw.polygon' } },
    { tool: { key: 'draw.polygon', params: { sides: 2 } } },       // out of range
    { tool: null },                                                // cancelled
    null,
  ]) {
    assert.equal(reconcileToolParameterPending(p, status), null);
  }
  // An in-queue refusal (queue full, Viewer edge) surfaces through `status.error`
  // without changing the value; a stuck delta would compose the next click up to
  // the cap away from the real value, so the error must drop it.
  assert.equal(reconcileToolParameterPending(p, { ...polygon(8), error: 'cad_set_tool_parameter: queue full' }), null);
  // An absent or empty error string is not a real error: the delta is kept.
  assert.equal(reconcileToolParameterPending(p, polygon(8)), p);
  assert.equal(reconcileToolParameterPending(p, { ...polygon(8), error: '' }), p);
  assert.equal(reconcileToolParameterPending(p, { ...polygon(8), error: null }), p);
});

test('folding a step refuses a pending that no longer matches the status', () => {
  const status = polygon(8);
  // A pending measured against a different base (the host already echoed) is
  // stale: the step is refused rather than composed off the wrong base.
  assert.equal(stepToolParameterPending(pending('polygonSides', 7, 1), status, 'polygonSides', 9), null);
  // A pending for a different parameter is likewise refused.
  assert.equal(stepToolParameterPending(pending('filletRadius', 8, 1), status, 'polygonSides', 9), null);
  // A matching pending folds normally.
  assert.deepEqual(stepToolParameterPending(pending('polygonSides', 8, 1), status, 'polygonSides', 10), pending('polygonSides', 8, 2));
});

test('the pending delta is capped and stops at the cap and the range bounds', () => {
  assert.equal(TOOL_PARAMETER_PENDING_CAP, 10);
  // At the positive cap the `+` step is no longer offered.
  const atCapUp = first(polygon(3), admitted, pending('polygonSides', 3, TOOL_PARAMETER_PENDING_CAP));
  assert.equal(atCapUp.effective, 13);
  assert.equal(atCapUp.canIncrement, false);
  assert.equal(atCapUp.canDecrement, true);
  // At the negative cap the `-` step is no longer offered.
  const atCapDown = first(polygon(1024), admitted, pending('polygonSides', 1024, -TOOL_PARAMETER_PENDING_CAP));
  assert.equal(atCapDown.effective, 1014);
  assert.equal(atCapDown.canDecrement, false);
  assert.equal(atCapDown.canIncrement, true);
  // The reducer refuses to grow past the cap (defensive: the view already disables).
  assert.equal(stepToolParameterPending(pending('polygonSides', 3, TOOL_PARAMETER_PENDING_CAP), polygon(3), 'polygonSides', 14), null);
  assert.deepEqual(stepToolParameterPending(pending('polygonSides', 3, 5), polygon(3), 'polygonSides', 4), pending('polygonSides', 3, 1));
  // A step that lands back on the status value collapses to "no pending".
  assert.equal(stepToolParameterPending(pending('polygonSides', 8, -1), polygon(8), 'polygonSides', 8), null);
});

test('the button disabled state follows the effective target, not the stale status', () => {
  // Status 1023 with one pending step: the effective target is already 1024, so
  // `+` is disabled and `-` still works — even though the rendered number is 1023.
  const v = first(polygon(1023), admitted, pending('polygonSides', 1023, 1));
  assert.equal(v.value, 1023);
  assert.equal(v.effective, 1024);
  assert.equal(v.canIncrement, false);
  assert.equal(v.canDecrement, true);
  assert.equal(v.increment, 1025); // raw target, not silently clamped to 1024
});

test('the status bar renders the group from status and never mirrors it locally', async () => {
  const statusbar = await readModule('components/StatusBar.jsx');
  assert.ok(statusbar.includes('toolParameterViews(shell.status, shell.canCommand, shell.toolParameterPending)'));
  // Only parameters with a real projected value render: the map yields the views.
  assert.ok(statusbar.includes('{toolParameters.map(view =>'));
  assert.ok(statusbar.includes("view.kind === 'flag'"));
  // The rendered number is the host value; a step dispatches the effective target
  // through the parameter key.
  assert.ok(statusbar.includes('onStep={shell.stepToolParameter}'));
  assert.ok(statusbar.includes('onToggle={shell.toggleToolParameter}'));
  assert.ok(statusbar.includes('{view.text}'));
  assert.ok(statusbar.includes('onStep(view.key, view.decrement)'));
  assert.ok(statusbar.includes('onStep(view.key, view.increment)'));
  // Bounded and admission-gated: the buttons disable at the effective edges.
  assert.ok(statusbar.includes('const stepDown = view.disabled || !view.canDecrement;'));
  assert.ok(statusbar.includes('const stepUp = view.disabled || !view.canIncrement;'));
  // No local mirror of the value and no direct IPC in the view component.
  assert.ok(!statusbar.includes('useState'));
  assert.ok(!statusbar.includes('invoke('));
  const app = await readAppSource();
  // The pending delta is owned by the shell, reconciled on every status and
  // folded in before the one IPC; a refusal clears it and reaches the banner.
  assert.ok(app.includes('toolParameterPending,'));
  assert.ok(app.includes('reconcileToolParameterPending(prev, status)'));
  assert.ok(app.includes('stepToolParameterPending(prev, status, key, target)'));
  assert.ok(app.includes('setToolParameterNumber(invoke, key, target)'));
  assert.ok(app.includes('setToolParameterFlag(invoke, key, next)'));
  assert.ok(app.includes('setToolParameterPending(null)'));
  assert.ok(app.includes("setError(t('status.tool_parameter.failed', { error: result.error }))"));
  assert.ok(app.includes('invoke(TOOL_PARAMETER_COMMAND, { parameter: key, number })'));
  assert.ok(app.includes('invoke(TOOL_PARAMETER_COMMAND, { parameter: key, flag })'));
});

test('the removed cad_set_polygon_sides name appears nowhere in the UI source', async () => {
  const source = await readAppSource();
  assert.ok(!source.includes('cad_set_polygon_sides'), 'the removed IPC must not reappear');
  assert.ok(!source.includes('POLYGON_SIDES'), 'the removed constants must not reappear');
  assert.ok(!source.includes('polygonSidesView'), 'the polygon-only view must not reappear');
  assert.ok(!source.includes('stepPolygonPending'));
  assert.ok(!source.includes('reconcilePolygonPending'));
  assert.ok(!source.includes('polygonSidesPending'));
  // The one general channel is what remains.
  assert.ok(source.includes("export const TOOL_PARAMETER_COMMAND = 'cad_set_tool_parameter';"));
});

test('the tool-parameter catalog keys exist in both locales with matching placeholders', () => {
  const keys = [
    'status.tool_parameter.decrease',
    'status.tool_parameter.increase',
    'status.tool_parameter.failed',
  ];
  for (const spec of Object.values(TOOL_PARAMETER_SPECS)) keys.push(spec.labelKey, spec.hintKey);
  const placeholder = /(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}/g;
  for (const key of keys) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
    const a = [...zh[key].matchAll(placeholder)].map(match => match[1]).sort();
    const b = [...en[key].matchAll(placeholder)].map(match => match[1]).sort();
    assert.deepEqual(a, b, `placeholders differ for ${key}`);
  }
  // The generic failure copy interpolates the host's refusal string.
  assert.deepEqual([...en['status.tool_parameter.failed'].matchAll(placeholder)].map(m => m[1]), ['error']);
  // The polygon label keeps the real copy and its hint keeps the real range.
  assert.match(zh['status.tool_parameter.polygon_sides'], /边数/);
  assert.match(en['status.tool_parameter.polygon_sides'], /sides/i);
  for (const locale of [zh, en]) {
    assert.match(locale['status.tool_parameter.polygon_sides.hint'], /3/);
    assert.match(locale['status.tool_parameter.polygon_sides.hint'], /1024/);
  }
  // The now-unused polygon-specific keys are gone from both catalogs.
  for (const gone of ['status.polygon_sides', 'status.polygon_sides.decrease', 'status.polygon_sides.increase', 'status.polygon_sides.hint', 'status.polygon_sides.failed']) {
    assert.equal(gone in zh, false, `stale ${gone} still in zh-CN`);
    assert.equal(gone in en, false, `stale ${gone} still in en`);
  }
});
