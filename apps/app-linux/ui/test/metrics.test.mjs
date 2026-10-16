import test from 'node:test';
import assert from 'node:assert/strict';
import { isReportableRect, metricsKey, pointerPosition, rectFromElement } from '../src/metrics.mjs';
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

test('the CAD partition paints nothing and holds no DOM children', async () => {
  const css = await readCss();
  assert.match(css, /\.native-input\s*\{[^}]*background:\s*transparent/);
  const app = await readAppSource();
  assert.match(app, /<div\s+ref=\{shell\.surfaceRef\}\s+className="native-input"[^>]*\/>/);
  assert.ok(!app.includes('<canvas'));
  assert.ok(!app.includes('position:absolute;inset:0;z-index'));
});
