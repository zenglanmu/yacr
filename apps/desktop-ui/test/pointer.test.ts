// The CAD-partition pointer gate: the system cursor is hidden over the CAD
// partition only while the host is actually drawing its own crosshair
// (`status.crosshair`). A 3D view draws no crosshair, so the class is absent and
// the system arrow stays — the pointer is never left invisible.
import test from 'node:test';
import assert from 'node:assert/strict';
import { parseCadStatus } from '../src/cad.ts';
import { readCss, readModule } from './helpers.ts';

const empty = {
  documentLabel: null, entityCount: null, loading: false, importProgress: null,
  error: null, canOpen: true, canNew: true, gridEnabled: true, snapGridEnabled: false, snapSpacing: 10, orthoEnabled: false, canSave: false, canPlot: false, canFit: false, backend: null, adapter: null,
  camera: null, importCount: 0, frameCount: 0, cursorWorld: null,
  selection: { count: 0, empty: true, rows: [], mixedKeys: [] },
};

test('the crosshair gate is validated as an optional real boolean', () => {
  const on = { ...empty, crosshair: true };
  assert.equal(parseCadStatus(on), on);
  const off = { ...empty, crosshair: false };
  assert.equal(parseCadStatus(off), off);
  // Absent (an older host) is tolerated: the UI then keeps the system arrow.
  assert.equal(parseCadStatus(empty), empty);
  for (const value of [
    { ...empty, crosshair: 'true' },
    { ...empty, crosshair: 1 },
    { ...empty, crosshair: null },
  ]) {
    assert.throws(() => parseCadStatus(value), /CAD_STATUS_INVALID/);
  }
});

test('the CAD partition blanks the cursor only under the crosshair class', async () => {
  const css = await readCss();
  // The base rule must NOT blank the cursor: a 3D view would then have none.
  const base = css.match(/\.native-input\s*\{[^}]*\}/);
  assert.ok(base, 'the .native-input base rule exists');
  assert.ok(
    !/cursor\s*:\s*none/.test(base[0]),
    'the base .native-input rule must not set cursor: none',
  );
  assert.match(css, /\.native-input\.is-crosshair\s*\{\s*cursor:\s*none;\s*\}/);
});

test('the workspace gates the crosshair class on the host status flag', async () => {
  const source = await readModule('components/Workspace.tsx');
  assert.ok(source.includes('const crosshair = shell.status?.crosshair === true;'));
  assert.ok(source.includes("crosshair ? 'is-crosshair' : ''"));
  // The class is a function of the host flag, never a local mirror.
  assert.ok(!source.includes('useState'));
});
