import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createStackingDiagnostics, isStackingRegression, observeCadInput, recordProbeAck, stopControlEvent } from '../src/stacking-regression/state.mjs';

const source = path => readFile(new URL(path, import.meta.url), 'utf8');

test('only the exact host boolean selects the regression fixture; normal App is preserved', async () => {
  for (const value of [undefined, null, false, 0, 1, 'true', '1', {}]) {
    assert.equal(isStackingRegression({ __YACR_STACKING_REGRESSION__: value }), false);
  }
  assert.equal(isStackingRegression(null), false);
  assert.equal(isStackingRegression({ __YACR_STACKING_REGRESSION__: true }), true);
  const main = await source('../src/main.jsx');
  assert.match(main, /isStackingRegression\(window\) \? <StackingRegression \/> : <App \/>/);
});

test('one empty native-input uses the real ordered lane and binding, with sibling overlays', async () => {
  const jsx = await source('../src/stacking-regression/Probe.jsx');
  assert.equal((jsx.match(/className="native-input"/g) ?? []).length, 1);
  assert.match(jsx, /<main className="shell stacking-regression"/);
  assert.match(jsx, /<div className="native-input"[^>]+\/>\s*<div className="probe-overlay/);
  assert.match(jsx, /createInputLane\(/);
  assert.match(jsx, /await invoke\('probe_input', \{ input \}\)/);
  assert.match(jsx, /const ack = await invoke\('probe_input', \{ input \}\);\s*recordProbeAck\(d, ack\)/);
  assert.match(jsx, /data-probe="admission"/);
  assert.match(jsx, /bindProbeInput\(surface, lane\)/);
  assert.doesNotMatch(jsx, /layout_ready|layoutReady|cad_open_popup|setPointerCapture/);
  for (const event of ['onPointerDown', 'onPointerMove', 'onPointerUp', 'onPointerCancel', 'onWheel']) {
    assert.ok(jsx.includes(`${event}: stopControlEvent`));
  }
  assert.match(jsx, /data-probe="opaque-button" \{\.\.\.controlEvents\}/);
  assert.match(jsx, /aria-label=\{t\('probe.dom_on_top.popup_title'\)\} \{\.\.\.controlEvents\}/);
});

test('fixture styles are scoped, ancestors/CAD transparent, popup crosses the CAD edge', async () => {
  const css = await source('../src/stacking-regression/probe.css');
  const withoutComments = css.replace(/\/\*[\s\S]*?\*\//g, '');
  for (const rule of withoutComments.split('}').filter(part => part.trim())) {
    for (const selector of rule.split('{')[0].split(',')) {
      assert.match(selector.trim(), /^(html\.yacr-stacking-regression|\.shell\.stacking-regression|\.stacking-regression\s)/);
    }
  }
  assert.match(css, /html\.yacr-stacking-regression #root/);
  assert.match(css, /\.stacking-regression \.native-input \{ background: transparent; \}/);
  assert.match(css, /height: 116px/);
  assert.match(css, /top: 82px/);
  assert.match(css, /background: rgba\(29, 92, 128, .55\)/);
  assert.match(css, /background: transparent;[^}]*pointer-events: none/s);
  for (const region of ['probe-translucent', 'probe-comparison']) {
    const rule = css.match(new RegExp(`\\.stacking-regression \\.${region} \\{([^}]+)\\}`))[1];
    assert.match(rule, /top: 50%; transform: translateY\(-50%\)/);
    assert.match(rule, /width: 26%/);
  }
  assert.match(css, /left: 50%; top: 50%; transform: translate\(-50%, -50%\)/);
});

test('resolved acknowledgement distinguishes host admission from refusal and transport errors', () => {
  const d = createStackingDiagnostics();
  const refusal = { accepted: false, reason: 'layout_not_ready', sequence: 1 };
  recordProbeAck(d, refusal);
  assert.equal(d.ipc.resolved, 1);
  assert.equal(d.ipc.accepted, 0);
  assert.equal(d.ipc.refused, 1);
  assert.equal(d.ipc.lastRefusalReason, 'layout_not_ready');
  assert.deepEqual(d.ipc.lastAck, refusal);
  assert.equal(d.lastIpcError, null);
  recordProbeAck(d, { accepted: true, sequence: 2 });
  assert.equal(d.ipc.accepted, 1);
  assert.equal(d.ipc.refused, 1);
  assert.equal(d.ipc.lastRefusalReason, 'layout_not_ready');
  d.lastIpcError = 'transport disconnected';
  recordProbeAck(d, { accepted: false });
  assert.equal(d.ipc.refused, 2);
  assert.equal(d.ipc.lastRefusalReason, null);
  assert.equal(d.lastIpcError, 'transport disconnected');
  for (const ack of [null, undefined, {}, { accepted: 'true' }, { accepted: 1 }]) recordProbeAck(d, ack);
  assert.equal(d.ipc.resolved, 8);
  assert.equal(d.ipc.accepted, 1);
  assert.equal(d.ipc.refused, 2);
  assert.equal(d.ipc.unrecognized, 5);
});

test('DOM observation counts CAD drags/wheels, not control events, and disposes', () => {
  const cad = new EventTarget();
  const button = new EventTarget();
  const d = createStackingDiagnostics();
  let changes = 0;
  const dispose = observeCadInput(cad, d, () => changes++);
  const dispatch = (target, type) => {
    const event = new Event(type, { bubbles: true });
    Object.defineProperty(event, 'pointerId', { value: 7 });
    target.dispatchEvent(event);
  };
  button.addEventListener('pointerdown', stopControlEvent);
  button.addEventListener('wheel', stopControlEvent);
  dispatch(button, 'pointerdown');
  dispatch(button, 'wheel');
  assert.equal(changes, 0);
  dispatch(cad, 'pointerdown');
  dispatch(cad, 'pointermove');
  dispatch(cad, 'pointerup');
  dispatch(cad, 'pointermove');
  dispatch(cad, 'wheel');
  assert.deepEqual(d.counters, { pointerDown: 1, pointerMove: 2, pointerUp: 1, pointerCancel: 0, wheel: 1, dragMove: 1, button: 0 });
  assert.equal(d.lastIpcError, null);
  assert.equal(d.mounted, false);
  dispose();
  dispatch(cad, 'wheel');
  assert.equal(changes, 5);
  let stopped = false;
  stopControlEvent({ stopPropagation() { stopped = true; } });
  assert.equal(stopped, true);
});

test('fixture auto-opens its popup once for multi-frame compositor capture', async () => {
  const jsx = await source('../src/stacking-regression/Probe.jsx');
  // Fixture-only automation: flips the same popup state as the toggle button
  // after a fixed delay, so the collector can sample a popup-open frame without
  // physical input. It must not open production dialogs or synthesize input.
  assert.match(jsx, /const AUTO_POPUP_DELAY_MS = 9000;/);
  assert.match(jsx, /setTimeout\(\(\) => \{\s*if \(!d\.popup\) \{\s*d\.popup = true;\s*setRevision\(value => value \+ 1\);/);
  assert.match(jsx, /return \(\) => clearTimeout\(timer\);/);
  assert.doesNotMatch(jsx, /cad_open_popup|native dialog/i);
});

test('fixture emits click-free pointer moves on the real CAD binding for marker motion', async () => {
  const jsx = await source('../src/stacking-regression/Probe.jsx');
  assert.match(jsx, /const MARKER_MOTION_DELAYS_MS = \[8000, 11000, 14000, 17000\];/);
  assert.match(jsx, /surface\.dispatchEvent\(new PointerEvent\('pointermove'/);
  // Click-free (no press captured) and scoped to the fixture's own surface.
  assert.match(jsx, /buttons: 0,/);
  assert.doesNotMatch(jsx, /pointerdown/);
});

test('every probe label exists in both authoritative catalogs', async () => {
  const jsx = await source('../src/stacking-regression/Probe.jsx');
  const keys = [...jsx.matchAll(/t\('(probe\.dom_on_top\.[^']+)'/g)].map(match => match[1]);
  for (const locale of ['en', 'zh-CN']) {
    const catalog = JSON.parse(await source(`../../../crates/cad-ui-core/i18n/${locale}.json`));
    for (const key of keys) assert.equal(typeof catalog[key], 'string', `${locale}: ${key}`);
  }
});
