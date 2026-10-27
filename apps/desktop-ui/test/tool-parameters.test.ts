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
  NO_TOOL_PARAMETER_DRAFT,
  TOOL_PARAMETER_COMMAND,
  TOOL_PARAMETER_INACTIVE_KEY,
  TOOL_PARAMETER_INVALID_KEY,
  TOOL_PARAMETER_PENDING_CAP,
  TOOL_PARAMETER_SPECS,
  TOOL_PARAMETERS,
  cancelToolParameterDraft,
  commitToolParameterDraft,
  reconcileToolParameterPending,
  setToolParameterFlag,
  setToolParameterNumber,
  stepToolParameterDraft,
  stepToolParameterPending,
  toolParameterViews,
  typeToolParameterDraft,
} from '../src/tools/tool-parameters.ts';
import { readAppSource, readCatalog, readModule } from './helpers.ts';

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
    step: 1, typed: false,
    // A plain row is not a chooser, not a colour swatch and is not scoped to a
    // sibling code: all three facets stay explicitly false/absent rather than being
    // left off the view.
    swatch: false, choices: null, inactive: false,
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
  // POLYGON, HATCH and the two text tools are reachable today; HATCH's row is its
  // six parameters, and TEXT/MTEXT share the one text height.
  assert.deepEqual(TOOL_PARAMETERS['draw.polygon'], { command: 'POLYGON', keys: ['polygonSides'] });
  assert.deepEqual(TOOL_PARAMETERS['draw.hatch'], {
    command: 'HATCH',
    keys: [
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
  });
  // Both text-creation tools own the same row: the host projects one session
  // `textHeight` and both create their glyphs with it (ADR 0019 F.1), so TEXT and
  // MTEXT can never show a different height.
  assert.deepEqual(TOOL_PARAMETERS['draw.text'], { command: 'TEXT', keys: ['textHeight'] });
  assert.deepEqual(TOOL_PARAMETERS['draw.mtext'], { command: 'MTEXT', keys: ['textHeight'] });
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
    'hatchGradientAngle', 'hatchGradientColorA', 'hatchGradientColorB', 'hatchGradientKind',
    'hatchPattern', 'hatchPatternType', 'hatchSpacing', 'hatchStyle', 'polygonSides', 'textHeight',
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

// --- the text height (ADR 0019 decision F.1) --------------------------------
// One session parameter, owned as a single row by both text-creation tools; the
// host's own rule is `finite_greater_than(0.0)` (`cad-app`'s
// `ToolParameter::TextHeight` validation), mirrored here and nowhere tightened.

const textHeight = (key, value) => ({ tool: { key, points: 1, ready: false, params: { textHeight: value } } });

test('the text height is one row on both text tools and comes from cad_status', () => {
  for (const key of ['draw.text', 'draw.mtext']) {
    // One row, in the host's order, on both tools.
    const rendered = toolParameterViews(textHeight(key, 2.5), admitted);
    assert.deepEqual(rendered.map(v => v.key), ['textHeight'], key);
    const v = rendered[0];
    assert.equal(v.kind, 'number');
    // The rendered value is the projected host value, never a local default.
    assert.equal(v.value, 2.5);
    assert.equal(v.text, '2.5');
    assert.equal(v.typed, true, 'a height is a length: it can be typed');
    assert.equal(v.step, 0.5);
    assert.equal(v.labelKey, 'status.tool_parameter.text_height');
    assert.equal(v.hintKey, 'status.tool_parameter.text_height.hint');
    assert.equal(v.decrement, 2);
    assert.equal(v.increment, 3);
    assert.equal(v.canDecrement, true);
    assert.equal(v.canIncrement, true);
    // The gate is the running tool's own command, never a hard-coded id.
    const asked = [];
    toolParameterViews(textHeight(key, 2.5), command => { asked.push(command); return true; });
    assert.deepEqual(asked, [key === 'draw.text' ? 'TEXT' : 'MTEXT'], key);
  }
  // A decimal projection round-trips verbatim, not through the 0.5 step.
  assert.equal(toolParameterViews(textHeight('draw.text', 1.75), admitted)[0].text, '1.75');
  // The host refuses anything not finite and strictly positive, so 0, a negative
  // and a non-finite height render no control at all — never a guessed one.
  for (const refused of [0, -1, -1e-9, NaN, Infinity, -Infinity, null, '2.5']) {
    assert.deepEqual(toolParameterViews(textHeight('draw.text', refused), admitted), [], String(refused));
  }
  // At the lower edge the `-` step (0.5 - 0.5 = 0) is not offered: it is disabled
  // with its raw target kept, never clamped up to a minimum.
  const lowest = toolParameterViews(textHeight('draw.text', 0.5), admitted)[0];
  assert.equal(lowest.decrement, 0);
  assert.equal(lowest.canDecrement, false);
  assert.equal(lowest.canIncrement, true);
});

test('a typed height is refused before any write, with the catalog message', async () => {
  // The draft guard is the host's own range: finite and strictly above 0. `0`, a
  // negative, NaN, an empty field and a non-numeric string are all refused.
  for (const raw of ['0', '-1', '-0.5', '-0', 'NaN', 'Infinity', 'abc', '', '   ', '1..2', '2,5']) {
    const commit = commitToolParameterDraft('textHeight', typeToolParameterDraft(NO_TOOL_PARAMETER_DRAFT, raw));
    assert.equal(commit.kind, 'refused', `"${raw}" must be refused`);
    assert.equal(commit.draft.messageKey, TOOL_PARAMETER_INVALID_KEY, raw);
    // The text the user typed is kept as-is: never rewritten, never clamped.
    assert.equal(commit.draft.text, raw);
  }
  // The refusal is a catalog key, and it is the one both catalogs carry.
  assert.equal(TOOL_PARAMETER_INVALID_KEY, 'status.tool_parameter.invalid');
  for (const locale of [zh, en]) assert.equal(typeof locale[TOOL_PARAMETER_INVALID_KEY], 'string');
  // The same values never reach IPC through the write path either.
  for (const number of [0, -1, NaN, Infinity, '1.75', null, undefined]) {
    const result = await setToolParameterNumber(() => assert.fail('a refused height reached IPC'), 'textHeight', number);
    assert.equal(result.kind, 'refused', String(number));
  }
});

test('a valid decimal height is written verbatim through the one IPC', async () => {
  // The commit hands over exactly what was typed.
  assert.deepEqual(commitToolParameterDraft('textHeight', typeToolParameterDraft(NO_TOOL_PARAMETER_DRAFT, '1.75')), {
    kind: 'write', number: 1.75, draft: NO_TOOL_PARAMETER_DRAFT,
  });
  // Surrounding whitespace is trimmed, the value is not re-derived from the steps.
  assert.equal(commitToolParameterDraft('textHeight', typeToolParameterDraft(null, ' 2.25 ')).number, 2.25);
  // …and it travels the one parameter channel, as a real json number.
  const calls = [];
  const result = await setToolParameterNumber(async (...args) => { calls.push(args); return { accepted: true }; }, 'textHeight', 1.75);
  assert.deepEqual(calls, [['cad_set_tool_parameter', { parameter: 'textHeight', number: 1.75 }]]);
  assert.deepEqual(result, { kind: 'accepted' });
});

test('an untouched height field commits nothing and Escape reverts to the host value', () => {
  // A blur of an untouched field is not a value: nothing is written.
  assert.deepEqual(commitToolParameterDraft('textHeight', NO_TOOL_PARAMETER_DRAFT), { kind: 'idle' });
  assert.deepEqual(commitToolParameterDraft('textHeight', null), { kind: 'idle' });
  assert.deepEqual(commitToolParameterDraft('textHeight', undefined), { kind: 'idle' });
  // Typed text is the draft; Escape drops it, so the field renders the last host
  // value again and a blur then commits nothing.
  const typed = typeToolParameterDraft(NO_TOOL_PARAMETER_DRAFT, '9');
  assert.equal(typed.text, '9');
  const reverted = cancelToolParameterDraft();
  assert.deepEqual(reverted, NO_TOOL_PARAMETER_DRAFT);
  assert.equal(reverted.text, null);
  assert.deepEqual(commitToolParameterDraft('textHeight', reverted), { kind: 'idle' });
  // A refusal keeps the typed text and clears its message on the next keystroke.
  const refused = commitToolParameterDraft('textHeight', typeToolParameterDraft(null, '0'));
  assert.equal(typeToolParameterDraft(refused.draft, '0.').messageKey, null);
});

test('the arrow keys step the height from the draft, never off half-typed text', () => {
  // From the host value: 2.5 ± 0.5.
  assert.equal(stepToolParameterDraft('textHeight', NO_TOOL_PARAMETER_DRAFT, 2.5, 1), 3);
  assert.equal(stepToolParameterDraft('textHeight', NO_TOOL_PARAMETER_DRAFT, 2.5, -1), 2);
  // From a typed decimal the draft wins over the stale host value.
  assert.equal(stepToolParameterDraft('textHeight', typeToolParameterDraft(null, '1.75'), 2.5, 1), 2.25);
  // A draft that is not a usable number falls back to the host value instead of
  // composing off it.
  assert.equal(stepToolParameterDraft('textHeight', typeToolParameterDraft(null, 'abc'), 2.5, 1), 3);
  assert.equal(stepToolParameterDraft('textHeight', typeToolParameterDraft(null, ''), 2.5, -1), 2);
  // A target the host would refuse is not offered at all (0.25 - 0.5 = -0.25).
  assert.equal(stepToolParameterDraft('textHeight', NO_TOOL_PARAMETER_DRAFT, 0.25, -1), null);
  // An unknown key has no step.
  assert.equal(stepToolParameterDraft('nope', NO_TOOL_PARAMETER_DRAFT, 1, 1), null);
});

test('the text height label and hint exist in both catalogs and name their scope', () => {
  for (const locale of [zh, en]) {
    for (const key of ['status.tool_parameter.text_height', 'status.tool_parameter.text_height.hint', TOOL_PARAMETER_INVALID_KEY]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
    // The hint names both tools that own the row and the documented default.
    assert.match(locale['status.tool_parameter.text_height.hint'], /TEXT/);
    assert.match(locale['status.tool_parameter.text_height.hint'], /MTEXT/);
    assert.match(locale['status.tool_parameter.text_height.hint'], /2\.5/);
  }
  // …and it says the height is a world-unit length, not a screen size.
  assert.match(en['status.tool_parameter.text_height.hint'], /drawing units/i);
  assert.match(en['status.tool_parameter.text_height.hint'], /single-line/i);
  assert.match(zh['status.tool_parameter.text_height.hint'], /图形单位/);
  assert.match(zh['status.tool_parameter.text_height.hint'], /单行文字/);
  // The refusal is parameterised by the localized name and says nothing was sent.
  for (const locale of [zh, en]) {
    assert.deepEqual(
      [...locale[TOOL_PARAMETER_INVALID_KEY].matchAll(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g)].map(match => match[1]),
      ['name'],
    );
  }
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
  const statusbar = await readModule('components/StatusBar.tsx');
  // The third argument is still the shell-owned pending map; TS only casts the
  // loose context field so the pure view can type it.
  assert.match(statusbar, /toolParameterViews\(shell\.status, shell\.canCommand, shell\.toolParameterPending(?:\s+as\s+[^)]+)?\)/);
  // Only parameters with a real projected value render: the map yields the views.
  assert.ok(statusbar.includes('{toolParameters.map(view =>'));
  assert.ok(statusbar.includes("view.kind === 'flag'"));
  // The rendered number is the host value; a step dispatches the effective target
  // through the parameter key.
  assert.ok(statusbar.includes('onStep={shell.stepToolParameter}'));
  assert.ok(statusbar.includes('onToggle={shell.toggleToolParameter}'));
  // A control is keyed by its tool too, so a half-typed draft cannot follow the
  // user to another tool that owns the same parameter (TEXT and MTEXT share it).
  assert.ok(statusbar.includes('key={`${tool?.key}:${view.key}`}'));
  // The bar itself stays a pure projection: no local state and no direct IPC.
  assert.ok(!statusbar.includes('useState'));
  assert.ok(!statusbar.includes('invoke('));

  // The controls live in their own module; they render the host value and own
  // only the typed draft.
  const controls = await readModule('components/ParameterControls.tsx');
  assert.ok(controls.includes('{view.text}'));
  // One step-target table serves the `-`/`+` buttons and the arrow keys, and every
  // dispatch goes through the one helper with the parameter key.
  assert.ok(controls.includes('down: typing'));
  assert.ok(controls.includes('? stepToolParameterDraft(view.key, draft, view.effective, -1)'));
  assert.ok(controls.includes('? stepToolParameterDraft(view.key, draft, view.effective, 1)'));
  assert.ok(controls.includes('onStep(view.key, target);'));
  assert.ok(controls.includes('onStep(view.key, result.number);'));
  // Bounded and admission-gated: the buttons disable at the effective edges (and
  // at a target the host refuses) instead of clamping.
  assert.ok(controls.includes('const stepDown = view.disabled ? null : stepTargets.down;'));
  assert.ok(controls.includes('const stepUp = view.disabled ? null : stepTargets.up;'));
  // The typed field is a draft, never a mirror: it commits through the parameter's
  // own guard, reverts on Escape, is IME-safe and never calls IPC itself.
  assert.ok(controls.includes('commitToolParameterDraft(view.key, draft)'));
  assert.ok(controls.includes('setDraft(cancelToolParameterDraft());'));
  assert.ok(controls.includes('if (event.isComposing || event.keyCode === 229) return;'));
  assert.ok(controls.includes("if (event.key === 'Escape') {"));
  // Escape must not reach the shell: the global Escape would blur the field (and
  // thereby commit what the user just discarded) or cancel the running tool.
  assert.ok(controls.includes('event.stopPropagation();'));
  assert.ok(controls.includes("if (event.key === 'Enter') {"));
  // Enter and a blur both commit — the blur is how a typed height survives the
  // click on the drawing that follows it.
  assert.ok(controls.includes('onBlur={commit}'));
  // A decimal field, showing the host value until the user types (draft first).
  assert.ok(controls.includes('inputMode="decimal"'));
  assert.ok(controls.includes('value={draft.text ?? view.text}'));
  assert.ok(!controls.includes('invoke('));

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
    TOOL_PARAMETER_INACTIVE_KEY,
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

// --- a closed set the user reads by name (HATCH's predefined pattern) --------
// The HATCH predefined pattern used to be a raw 1..3 stepper: the user had to
// remember that "2" means ANSI32. The wire value is unchanged (the host's 1-based
// index); only the control gained names, so every assertion below is about the
// *same* numeric row presenting its own `choices`.

const HATCH_DEFAULTS = Object.freeze({
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

const hatch = params => ({ tool: { key: 'draw.hatch', points: 0, ready: false, params: { ...HATCH_DEFAULTS, ...params } } });
const hatchRow = (params = {}, gate = admitted, inflight = null) =>
  toolParameterViews(hatch(params), gate, inflight).find(entry => entry.key === 'hatchPattern');

test('the predefined pattern offers its three implemented names and still sends the integer index', async () => {
  const spec = TOOL_PARAMETER_SPECS.hatchPattern;
  // The names cover the offered set exactly — no code without a name, no name
  // without a code — so the chooser can neither show a code the host would refuse
  // nor hide one it accepts. (A table that ever disagreed demotes the row back to
  // the stepper rather than inventing or dropping a value.)
  assert.deepEqual([...spec.choices], [1, 2, 3]);
  assert.deepEqual(Object.keys(spec.choiceLabelKeys).map(Number).sort(), [...spec.choices]);
  for (const key of Object.values(spec.choiceLabelKeys)) {
    for (const locale of [zh, en]) assert.equal(typeof locale[key], 'string', `missing ${key}`);
  }

  const row = hatchRow({ hatchPatternType: 2 });
  assert.equal(row.kind, 'number');
  // It is still the same numeric row: same key, same value slot, same bounds. The
  // chooser is a presentation of that row, never a second parameter.
  assert.equal(row.value, 1);
  assert.equal(row.text, '1');
  assert.equal(row.decrement, 0);
  assert.equal(row.canDecrement, false);
  assert.equal(row.increment, 2);
  assert.equal(row.canIncrement, true);
  // Three segments, in the host's own code order, each carrying the integer the
  // IPC sends plus the catalog key of the name the user reads.
  assert.deepEqual(row.choices.map(choice => [choice.value, choice.labelKey, choice.selected, choice.disabled]), [
    [1, 'status.tool_parameter.hatch_pattern.ansi31', true, false],
    [2, 'status.tool_parameter.hatch_pattern.ansi32', false, false],
    [3, 'status.tool_parameter.hatch_pattern.ansi37', false, false],
  ]);
  // The chosen state follows the host value, so the segment that is filled is the
  // pattern the host actually holds.
  assert.deepEqual(hatchRow({ hatchPatternType: 2, hatchPattern: 3 }).choices.map(choice => choice.selected), [false, false, true]);
  assert.deepEqual(row.choices.map(choice => choice.selected).filter(Boolean).length, 1);

  // A pick writes exactly the 1-based index through the one numeric channel: no
  // string, no second IPC, no name on the wire.
  for (const [choice, index] of [[row.choices[1], 2], [row.choices[2], 3], [row.choices[0], 1]]) {
    assert.equal(typeof choice.value, 'number');
    assert.equal(Number.isSafeInteger(choice.value), true);
    const calls = [];
    const result = await setToolParameterNumber(async (...args) => { calls.push(args); return { accepted: true }; }, 'hatchPattern', choice.value);
    assert.deepEqual(result, { kind: 'accepted' });
    assert.deepEqual(calls, [['cad_set_tool_parameter', { parameter: 'hatchPattern', number: index }]]);
    assert.equal(typeof calls[0][1].number, 'number');
  }
});

test('the pattern chooser stays visible but inert outside the predefined type', () => {
  // Solid (0), user-defined (1) and gradient (3) ignore the pattern index: the row
  // is still rendered — the three names stay discoverable before the type is
  // switched — but nothing is pickable, and the hint says why.
  for (const type of [0, 1, 3]) {
    const row = hatchRow({ hatchPatternType: type });
    assert.ok(row, `the row stays visible under type ${type}`);
    assert.equal(row.inactive, true, `inactive under type ${type}`);
    for (const choice of row.choices) assert.equal(choice.disabled, true, `type ${type} / ${choice.value}`);
    // Scope, never capability: the command is admitted, so the row's `disabled`
    // (admission alone) stays false — this is not a support claim.
    assert.equal(row.disabled, false);
    // The stored code is still marked, so the pattern in force is readable even
    // while it is not being used.
    assert.deepEqual(row.choices.filter(choice => choice.selected).map(choice => choice.value), [row.value]);
  }
  // Predefined (2) is the one type the index applies to.
  const predefined = hatchRow({ hatchPatternType: 2 });
  assert.equal(predefined.inactive, false);
  for (const choice of predefined.choices) assert.equal(choice.disabled, false);
  // An absent, null or non-numeric sibling value never claims the row applies.
  for (const params of [{}, { hatchPatternType: null }, { hatchPatternType: '2' }, { hatchPatternType: 2.5 }]) {
    const row = hatchRow(params);
    assert.ok(row, JSON.stringify(params));
    assert.equal(row.inactive, true, JSON.stringify(params));
  }
  // Command admission disables every segment independently of scope.
  const deniedRow = hatchRow({ hatchPatternType: 2 }, denied);
  assert.equal(deniedRow.disabled, true);
  for (const choice of deniedRow.choices) assert.equal(choice.disabled, true);
  // The other closed sets keep their plain steppers: only a row whose spec names
  // its codes becomes a chooser, so the pattern-type and style controls did not
  // change shape. (The gradient kind names its two codes, so it is a chooser too.)
  const rendered = toolParameterViews(hatch({ hatchPatternType: 2 }), admitted);
  assert.deepEqual(rendered.map(entry => entry.key), TOOL_PARAMETERS['draw.hatch'].keys);
  for (const entry of rendered) {
    if (entry.kind === 'number') {
      const named = entry.key === 'hatchPattern' || entry.key === 'hatchGradientKind';
      assert.equal(entry.choices === null, !named, entry.key);
    }
  }
});

test('a pattern pick composes through the existing pending delta and never moves the status value', () => {
  const status = hatch({ hatchPatternType: 2, hatchPattern: 1 });
  const row = hatchRow({ hatchPatternType: 2, hatchPattern: 1 }, admitted, pending('hatchPattern', 1, 2));
  // The status value is untouched: the host still holds ANSI31.
  assert.equal(row.value, 1);
  assert.equal(row.text, '1');
  assert.equal(row.effective, 3);
  // …while the picked name already reads as chosen. A choice is an absolute
  // target, so the pick folds into the same bounded delta a step uses.
  assert.deepEqual(row.choices.map(choice => choice.selected), [false, false, true]);
  assert.deepEqual(stepToolParameterPending(null, status, 'hatchPattern', 3), pending('hatchPattern', 1, 2));
  // A pending for another parameter never moves this row's chosen segment.
  const other = hatchRow({ hatchPatternType: 2, hatchPattern: 2 }, admitted, pending('hatchStyle', 1, 1));
  assert.deepEqual(other.choices.map(choice => choice.selected), [false, true, false]);
  // While the host has not echoed, the delta is kept (same reference: no re-render).
  const inflight = pending('hatchPattern', 1, 2);
  assert.equal(reconcileToolParameterPending(inflight, status), inflight);
  // Once the host echoes, the delta is dropped and the name is the status itself.
  const echoedStatus = hatch({ hatchPatternType: 2, hatchPattern: 3 });
  assert.equal(reconcileToolParameterPending(inflight, echoedStatus), null);
  const echoed = toolParameterViews(echoedStatus, admitted).find(entry => entry.key === 'hatchPattern');
  assert.equal(echoed.delta, 0);
  assert.deepEqual(echoed.choices.map(choice => choice.selected), [false, false, true]);
});

test('the pattern names and the scope sentence exist in both catalogs', () => {
  const names = Object.values(TOOL_PARAMETER_SPECS.hatchPattern.choiceLabelKeys);
  const placeholder = /(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}/g;
  for (const key of [TOOL_PARAMETER_INACTIVE_KEY, ...names]) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
    const a = [...zh[key].matchAll(placeholder)].map(match => match[1]).sort();
    const b = [...en[key].matchAll(placeholder)].map(match => match[1]).sort();
    assert.deepEqual(a, b, `placeholders differ for ${key}`);
  }
  // A pattern name is the host's own table name, not translated copy: it is the
  // same string in both locales and is never a number.
  for (const key of names) {
    assert.equal(zh[key], en[key], `${key} must be the pattern's own name`);
    assert.match(zh[key], /^ANSI[0-9]+$/);
  }
  // The scope sentence says the row is not in effect here; it never claims the
  // parameter is unsupported (the host would accept the write either way).
  for (const locale of [zh, en]) {
    assert.doesNotMatch(locale[TOOL_PARAMETER_INACTIVE_KEY], /not supported|unsupported|not implemented|不支持|尚未支持/i);
  }
});
