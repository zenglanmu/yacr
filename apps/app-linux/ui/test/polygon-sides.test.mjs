// POLYGON side-count control contract: it renders only while POLYGON is the
// active tool, reads the value from the real `status.tool.params.sides` (never a
// local mirror), steps within the 3..=1024 range, dispatches exactly one
// `cad_set_polygon_sides` per step, reports a refusal honestly, and composes
// rapid steps through a bounded pending delta without ever moving the displayed
// number.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  POLYGON_SIDES_COMMAND,
  POLYGON_SIDES_MAX,
  POLYGON_SIDES_MIN,
  POLYGON_SIDES_PENDING_CAP,
  polygonSidesView,
  reconcilePolygonPending,
  setPolygonSides,
  stepPolygonPending,
} from '../src/cad.mjs';
import { readAppSource, readCatalog, readModule } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

const polygon = sides => ({ tool: { key: 'draw.polygon', points: 1, ready: false, params: { sides } } });
const pending = (base, delta) => ({ key: 'draw.polygon', base, delta });

test('the polygon stepper renders only for the active POLYGON tool with a real in-range value', () => {
  assert.deepEqual(polygonSidesView(polygon(8), true), {
    sides: 8, delta: 0, effective: 8, disabled: false,
    canDecrement: true, canIncrement: true, decrement: 7, increment: 9,
  });
  // No tool, another tool (whose `params.sides` is null), or a missing/`null`/
  // malformed/out-of-range value renders nothing at all — never a default.
  for (const status of [
    null,
    undefined,
    {},
    { tool: null },
    { tool: 'draw.polygon' },
    { tool: { key: 'draw.line', params: { sides: null } } },
    { tool: { key: 'draw.polygon', params: { sides: null } } },
    { tool: { key: 'draw.polygon', params: null } },
    { tool: { key: 'draw.polygon' } },
    { tool: { key: 'draw.polygon', params: { sides: 2 } } },
    { tool: { key: 'draw.polygon', params: { sides: 1025 } } },
    { tool: { key: 'draw.polygon', params: { sides: 4.5 } } },
    { tool: { key: 'draw.polygon', params: { sides: '8' } } },
  ]) {
    assert.equal(polygonSidesView(status, true), null);
  }
});

test('the stepper is bounded at 3 and 1024 and disabled until the command is admitted', () => {
  assert.equal(POLYGON_SIDES_MIN, 3);
  assert.equal(POLYGON_SIDES_MAX, 1024);
  // At the edges the matching button is disabled, so an out-of-range step is
  // never offered and the value is never rewritten.
  const low = polygonSidesView(polygon(POLYGON_SIDES_MIN), true);
  assert.equal(low.canDecrement, false);
  assert.equal(low.canIncrement, true);
  const high = polygonSidesView(polygon(POLYGON_SIDES_MAX), true);
  assert.equal(high.canDecrement, true);
  assert.equal(high.canIncrement, false);
  // Admission mirrors `canCommand`: host unavailable / vocabulary not loaded /
  // pending all disable the whole control.
  assert.equal(polygonSidesView(polygon(8), false).disabled, true);
  assert.equal(polygonSidesView(polygon(8), undefined).disabled, true);
  assert.equal(polygonSidesView(polygon(8), true).disabled, false);
});

test('a step dispatches exactly one cad_set_polygon_sides with the intended number', async () => {
  const calls = [];
  const result = await setPolygonSides(async (...args) => { calls.push(args); return { accepted: true }; }, 9);
  assert.deepEqual(calls, [['cad_set_polygon_sides', { sides: 9 }]]);
  assert.equal(POLYGON_SIDES_COMMAND, 'cad_set_polygon_sides');
  assert.deepEqual(result, { kind: 'accepted' });
  // Acceptance is queue admission, not completion: no value/status is returned.
  assert.equal('sides' in result, false);
  assert.equal('completed' in result, false);
});

test('an out-of-range value is refused before IPC and a refusal is reported honestly', async () => {
  for (const sides of [2, 1025, 0, -1, 4.5, NaN, '8', null, undefined]) {
    const result = await setPolygonSides(() => assert.fail('out-of-range value reached IPC'), sides);
    assert.equal(result.kind, 'refused');
    assert.ok(result.error);
  }
  for (const invoke of [
    async () => { throw Error('viewer mode'); },
    async () => ({ accepted: false }),
    async () => null,
    async () => undefined,
  ]) {
    const result = await setPolygonSides(invoke, 6);
    assert.equal(result.kind, 'failed');
    assert.ok(result.error);
  }
});

test('two rapid steps compose while the rendered number still follows the status', () => {
  const status = polygon(8);
  // Step 1: the view still renders 8 and offers 9.
  const v1 = polygonSidesView(status, true, null);
  assert.equal(v1.sides, 8);
  assert.equal(v1.effective, 8);
  assert.equal(v1.increment, 9);
  const p1 = stepPolygonPending(null, status, v1.increment);
  assert.deepEqual(p1, pending(8, 1));
  // Step 2 (host has not echoed yet): the rendered number is *still* 8, but the
  // effective target is 9, so the next step composes to 10 = status.sides + 2.
  const v2 = polygonSidesView(status, true, p1);
  assert.equal(v2.sides, 8);
  assert.equal(v2.effective, 9);
  assert.equal(v2.increment, 10);
  const p2 = stepPolygonPending(p1, status, v2.increment);
  assert.deepEqual(p2, pending(8, 2));
  // The two dispatched targets were 9 then 10 — never 9 then 9.
  assert.deepEqual([v1.increment, v2.increment], [9, 10]);
});

test('the pending delta resets when the status value changes (host caught up)', () => {
  const p = pending(8, 2);
  // Still the same value: keep accumulating (same reference, so no re-render).
  assert.equal(reconcilePolygonPending(p, polygon(8)), p);
  // The host echoed: the delta is dropped and the next step builds on the new value.
  assert.equal(reconcilePolygonPending(p, polygon(10)), null);
  const v = polygonSidesView(polygon(10), true, null);
  assert.equal(v.sides, 10);
  assert.equal(v.increment, 11);
});

test('the pending delta resets on tool switch, cancel and an absent/null value', () => {
  const p = pending(8, 2);
  for (const status of [
    { tool: { key: 'draw.line', params: { sides: null } } },  // tool switched
    { tool: { key: 'draw.polygon', params: { sides: null } } }, // value absent
    { tool: { key: 'draw.polygon', params: null } },
    { tool: { key: 'draw.polygon' } },
    { tool: { key: 'draw.polygon', params: { sides: 2 } } },    // out of range
    { tool: { key: 'draw.polygon', params: { sides: 1025 } } },
    { tool: null },                                             // cancelled
    null,
  ]) {
    assert.equal(reconcilePolygonPending(p, status), null);
  }
});

test('the pending delta resets on a host error even while the value is unchanged', () => {
  const p = pending(8, 2);
  // An in-queue refusal (queue full, Viewer edge) surfaces through `status.error`
  // without changing the value. A stuck delta would leave `pending.base === sides`
  // and compose the next click up to the cap away from the real value, so the
  // error must drop it.
  assert.equal(reconcilePolygonPending(p, { ...polygon(8), error: 'cad_set_polygon_sides: queue full' }), null);
  // An absent or empty error string is not a real error: the delta is kept.
  assert.equal(reconcilePolygonPending(p, polygon(8)), p);
  assert.equal(reconcilePolygonPending(p, { ...polygon(8), error: '' }), p);
  assert.equal(reconcilePolygonPending(p, { ...polygon(8), error: null }), p);
  // After the error the next step builds on the real value (9), not the stale
  // base+delta+1 (11).
  const errored = { ...polygon(8), error: 'viewer mode' };
  const view = polygonSidesView(errored, true, null);
  assert.equal(view.sides, 8);
  assert.equal(view.effective, 8);
  assert.equal(view.increment, 9);
});

test('folding a step refuses a pending that no longer matches the status', () => {
  const status = polygon(8);
  // A pending measured against a different base (the host already echoed) is
  // stale: the step is refused rather than composed off the wrong base.
  assert.equal(stepPolygonPending(pending(7, 1), status, 9), null);
  // A pending for a different tool is likewise refused.
  assert.equal(stepPolygonPending({ key: 'draw.line', base: 8, delta: 1 }, status, 9), null);
  // A matching pending folds normally.
  assert.deepEqual(stepPolygonPending(pending(8, 1), status, 10), pending(8, 2));
});

test('the pending delta is capped and stops at the cap and the range bounds', () => {
  assert.equal(POLYGON_SIDES_PENDING_CAP, 10);
  // At the positive cap the `+` step is no longer offered.
  const atCapUp = polygonSidesView(polygon(3), true, pending(3, POLYGON_SIDES_PENDING_CAP));
  assert.equal(atCapUp.effective, 13);
  assert.equal(atCapUp.canIncrement, false);
  assert.equal(atCapUp.canDecrement, true);
  // At the negative cap the `-` step is no longer offered.
  const atCapDown = polygonSidesView(polygon(1024), true, pending(1024, -POLYGON_SIDES_PENDING_CAP));
  assert.equal(atCapDown.effective, 1014);
  assert.equal(atCapDown.canDecrement, false);
  assert.equal(atCapDown.canIncrement, true);
  // The reducer refuses to grow past the cap (defensive: the view already disables).
  assert.equal(stepPolygonPending(pending(3, POLYGON_SIDES_PENDING_CAP), polygon(3), 14), null);
  assert.deepEqual(stepPolygonPending(pending(3, 5), polygon(3), 4), pending(3, 1));
  // A step that lands back on the status value collapses to "no pending".
  assert.equal(stepPolygonPending(pending(8, -1), polygon(8), 8), null);
});

test('the button disabled state follows the effective target, not the stale status', () => {
  // Status 1023 with one pending step: the effective target is already 1024, so
  // `+` is disabled and `-` still works — even though the rendered number is 1023.
  const view = polygonSidesView(polygon(1023), true, pending(1023, 1));
  assert.equal(view.sides, 1023);
  assert.equal(view.effective, 1024);
  assert.equal(view.canIncrement, false);
  assert.equal(view.canDecrement, true);
  // The raw target is left at 1025 — it is *not* silently clamped to 1024; it is
  // simply not offered.
  assert.equal(view.increment, 1025);
  // Mirror case at the low bound: status 4 with one pending decrement => target 3.
  const low = polygonSidesView(polygon(4), true, pending(4, -1));
  assert.equal(low.effective, 3);
  assert.equal(low.canDecrement, false);
  assert.equal(low.canIncrement, true);
  assert.equal(low.decrement, 2);
});

test('the status bar renders the stepper from status and never mirrors it locally', async () => {
  const statusbar = await readModule('components/StatusBar.jsx');
  assert.ok(statusbar.includes("polygonSidesView(shell.status, shell.canCommand('POLYGON'), shell.polygonSidesPending)"));
  // Only rendered while a real POLYGON value is present: no placeholder chip.
  assert.ok(statusbar.includes('{polygonSides ? ('));
  // The rendered number is the host value; a step dispatches the effective target.
  assert.ok(statusbar.includes('onStep={target => shell.stepPolygonSides(target)}'));
  assert.ok(statusbar.includes('{view.sides}'));
  assert.ok(statusbar.includes('onStep(view.decrement)'));
  assert.ok(statusbar.includes('onStep(view.increment)'));
  // Bounded and admission-gated: the buttons disable at the effective edges.
  assert.ok(statusbar.includes('const stepDown = view.disabled || !view.canDecrement;'));
  assert.ok(statusbar.includes('const stepUp = view.disabled || !view.canIncrement;'));
  // No local mirror of the value and no direct IPC in the view component.
  assert.ok(!statusbar.includes('useState'));
  assert.ok(!statusbar.includes('invoke('));
  const app = await readAppSource();
  // The pending delta is owned by the shell, reconciled on every status and
  // folded in before the one IPC; a refusal clears it and reaches the banner.
  assert.ok(app.includes('polygonSidesPending: polygonPending'));
  assert.ok(app.includes('reconcilePolygonPending(prev, status)'));
  assert.ok(app.includes('stepPolygonPending(prev, status, target)'));
  assert.ok(app.includes('applyPolygonSides(invoke, target)'));
  assert.ok(app.includes('setPolygonPending(null)'));
  assert.ok(app.includes("setError(t('status.polygon_sides.failed', { error: result.error }))"));
  assert.ok(app.includes('invoke(POLYGON_SIDES_COMMAND, { sides })'));
});

test('the polygon sides catalog keys exist in both locales with matching placeholders', () => {
  const keys = [
    'status.polygon_sides',
    'status.polygon_sides.decrease',
    'status.polygon_sides.increase',
    'status.polygon_sides.hint',
    'status.polygon_sides.failed',
  ];
  const placeholder = /(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}/g;
  for (const key of keys) {
    for (const locale of [zh, en]) {
      assert.equal(typeof locale[key], 'string', `missing ${key}`);
      assert.ok(locale[key].trim().length > 0, `empty ${key}`);
    }
    const a = [...zh[key].matchAll(placeholder)].map(m => m[1]).sort();
    const b = [...en[key].matchAll(placeholder)].map(m => m[1]).sort();
    assert.deepEqual(a, b, `placeholders differ for ${key}`);
  }
  assert.match(zh['status.polygon_sides'], /边数/);
  assert.match(en['status.polygon_sides'], /sides/i);
  // The failure copy interpolates the host's refusal string.
  assert.deepEqual([...en['status.polygon_sides.failed'].matchAll(placeholder)].map(m => m[1]), ['error']);
});

test('the polygon sides hint copy states the real 3 to 1024 range', () => {
  for (const locale of [zh, en]) {
    assert.match(locale['status.polygon_sides.hint'], /3/);
    assert.match(locale['status.polygon_sides.hint'], /1024/);
  }
});
