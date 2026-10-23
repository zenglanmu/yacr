import test from 'node:test';
import assert from 'node:assert/strict';
import { canvasBackingSize, isReportableRect, metricsKey, pointerPosition, rectFromElement } from '../src/metrics.mjs';
import { readAppSource, readCss, readModule } from './helpers.mjs';

const element = rect => ({ getBoundingClientRect: () => rect });

test('rectFromElement reports the partition in CSS pixels with the DPR', () => {
  const rect = rectFromElement(element({ left: 8, top: 101, width: 784, height: 439 }), 1.5);
  assert.deepEqual(rect, { x: 8, y: 101, width: 784, height: 439, dpr: 1.5 });
});

test('metricsKey is stable for identical rects and differs when anything moves', () => {
  const base = { x: 0, y: 100, width: 800, height: 400, dpr: 1 };
  assert.equal(metricsKey(base), metricsKey({ ...base }));
  for (const change of [{ x: 1 }, { y: 101 }, { width: 801 }, { height: 401 }, { dpr: 2 }]) {
    assert.notEqual(metricsKey({ ...base, ...change }), metricsKey(base));
  }
});

test('isReportableRect rejects degenerate and non-finite partitions', () => {
  assert.equal(isReportableRect({ x: 0, y: 0, width: 10, height: 10, dpr: 1 }), true);
  assert.equal(isReportableRect({ x: 0, y: 0, width: 0, height: 10, dpr: 1 }), false);
  assert.equal(isReportableRect({ x: 0, y: 0, width: 10, height: -1, dpr: 1 }), false);
  assert.equal(isReportableRect({ x: 0, y: 0, width: 10, height: 10, dpr: 0 }), false);
  assert.equal(isReportableRect({ x: NaN, y: 0, width: 10, height: 10, dpr: 1 }), false);
});

test('pointerPosition is local to the partition, not the window', () => {
  const target = element({ left: 8, top: 101, width: 784, height: 439 });
  assert.deepEqual(pointerPosition(target, { clientX: 28, clientY: 131 }), { x: 20, y: 30 });
});

test('canvasBackingSize converts the same rect to integer physical pixels', () => {
  // A DPR-1 partition is already physical: identity, no forced rounding.
  assert.deepEqual(canvasBackingSize({ width: 784, height: 439, dpr: 1 }), { width: 784, height: 439 });
  // DPR scales both axes; the CSS rect itself is never rounded.
  assert.deepEqual(canvasBackingSize({ width: 801.5, height: 300.5, dpr: 2 }), { width: 1603, height: 601 });
  assert.deepEqual(canvasBackingSize({ width: 784, height: 439, dpr: 1.5 }), { width: 1176, height: 659 });
});

test('canvasBackingSize rounds to the nearest physical pixel', () => {
  // `Math.round` on each axis: .49 down, .5 up (documented, uniform policy).
  assert.deepEqual(canvasBackingSize({ width: 100.4, height: 100.6, dpr: 1 }), { width: 100, height: 101 });
  assert.deepEqual(canvasBackingSize({ width: 1.25, height: 1.24, dpr: 2 }), { width: 3, height: 2 });
});

test('canvasBackingSize clamps a positive rect to at least one physical pixel', () => {
  // A sub-pixel partition is still positive, so it reports the smallest size a
  // surface can be configured at - never 0.
  assert.deepEqual(canvasBackingSize({ width: 0.3, height: 0.2, dpr: 1 }), { width: 1, height: 1 });
  assert.deepEqual(canvasBackingSize({ width: 0.1, height: 40, dpr: 0.5 }), { width: 1, height: 20 });
});

test('canvasBackingSize rejects degenerate rects instead of inventing a size', () => {
  assert.equal(canvasBackingSize(null), null);
  assert.equal(canvasBackingSize(undefined), null);
  assert.equal(canvasBackingSize({ width: 0, height: 10, dpr: 1 }), null);
  assert.equal(canvasBackingSize({ width: 10, height: 0, dpr: 1 }), null);
  assert.equal(canvasBackingSize({ width: -4, height: 10, dpr: 1 }), null);
  assert.equal(canvasBackingSize({ width: 10, height: 10, dpr: 0 }), null);
  assert.equal(canvasBackingSize({ width: 10, height: 10, dpr: -1 }), null);
  assert.equal(canvasBackingSize({ width: NaN, height: 10, dpr: 1 }), null);
  assert.equal(canvasBackingSize({ width: 10, height: Infinity, dpr: 1 }), null);
  assert.equal(canvasBackingSize({ width: 10, height: 10 }), null);
});

test('the input binding listens to every chrome region so metrics refresh on layout change', async () => {
  const source = await readModule('input.mjs');
  assert.ok(source.includes(".closest?.('.shell')"));
  assert.ok(source.includes(".querySelectorAll('.app-region')"));
  assert.ok(source.includes('rectFromElement'));
  assert.ok(source.includes('isReportableRect'));
  assert.ok(source.includes('metricsKey'));
  assert.ok(!source.includes('JSON.stringify(data)'));
});

test('every region element is tagged so the CAD rect re-reports when chrome reflows', async () => {
  const source = await readAppSource();
  const tags = [...source.matchAll(/app-region/g)];
  assert.ok(tags.length >= 7, `expected every region tagged, saw ${tags.length}`);
  for (const component of ['TitleBar.jsx', 'Ribbon.jsx', 'DocTabs.jsx', 'LayoutTabs.jsx', 'StatusBar.jsx', 'NavRail.jsx', 'Workspace.jsx']) {
    assert.ok(source.includes(component), `${component} must be part of the app source`);
  }
});

test('the CAD partition paints nothing, holds no DOM children, and its DOM carrier is a sibling', async () => {
  const css = await readCss();
  assert.match(css, /\.native-input\s*\{[^}]*background:\s*transparent/);
  const app = await readAppSource();
  assert.match(app, /ref=\{shell\.surfaceRef\}/);
  assert.ok(app.includes("'native-input'"));
  assert.ok(!app.includes('position:absolute;inset:0;z-index'));
  // ADR 0024 D3: the second carrier may only be mounted behind the explicit
  // `dom-canvas` branch, as an input-transparent sibling of `.native-input` -
  // never a child, never a wrapper, and never rendered unconditionally.
  const workspace = (await readModule('components/Workspace.jsx')).replace(/\{\/\*[\s\S]*?\*\/\}/g, '');
  assert.match(
    workspace,
    /\{domCanvas \? \(\s*<canvas id="cad-canvas" className="cad-canvas" aria-hidden="true" \/>\s*\) : null\}/,
  );
  // Immediately preceding sibling: nothing but the branch close separates the
  // canvas from the `.native-input` div.
  assert.match(workspace, /<canvas id="cad-canvas"[^>]*\/>\s*\) : null\}\s*<div\s+ref=\{shell\.surfaceRef\}/);
  assert.ok(
    workspace.indexOf('"workspace__canvas"') < workspace.indexOf('<canvas id="cad-canvas"'),
    'the carrier lives inside .workspace__canvas',
  );
  // `.native-input` stays empty and self-closing, so the canvas cannot be its child.
  assert.match(workspace, /ref=\{shell\.surfaceRef\}[^>]*\/>/);
  const carrier = css.match(/\.cad-canvas\s*\{[^}]*\}/);
  assert.ok(carrier, 'the .cad-canvas rule exists');
  // Both carriers are sized from one measurement: the canvas overlays exactly
  // the `.native-input` rect, out of layout flow, and never takes a pointer.
  assert.match(carrier[0], /position:\s*absolute/);
  assert.match(carrier[0], /inset:\s*0/);
  assert.match(carrier[0], /width:\s*100%/);
  assert.match(carrier[0], /height:\s*100%/);
  assert.match(carrier[0], /pointer-events:\s*none/);
  assert.ok(!/background/.test(carrier[0]), 'the canvas carries no fill of its own');
});


test('every production ancestor over the CAD partition stays transparent', async () => {
  const css = await readCss();
  // WP2 composites the native CAD surface *below* the DOM, so an opaque fill on
  // any ancestor spanning the partition rect hides it. Chrome regions keep their
  // own opaque fills; only the shell/workspace/canvas chain is see-through.
  for (const selector of ['.shell', '.workspace', '.workspace__canvas']) {
    const rule = css.match(new RegExp(`${selector.replace('.', '\\.')}\\s*\\{([^}]*)\\}`));
    assert.ok(rule, `${selector} rule exists`);
    assert.match(rule[1], /background:\s*transparent/, `${selector} paints nothing over the CAD surface`);
  }
  for (const selector of ['.workspace__left', '.navrail']) {
    const rule = css.match(new RegExp(`${selector.replace('.', '\\.')}\\s*\\{([^}]*)\\}`));
    assert.ok(rule, `${selector} rule exists`);
    assert.match(rule[1], /background:\s*var\(--chrome-3\)/, `${selector} keeps its opaque dock fill`);
  }
});
