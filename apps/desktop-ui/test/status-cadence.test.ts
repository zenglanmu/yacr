// The status stream arrives every rendered frame. Only semantic changes may
// re-render the shell; the two per-frame fields go to a leaf store. These tests
// pin the split so a future field addition cannot silently start churning React
// (or, worse, be dropped because it looked volatile).
import test from 'node:test';
import assert from 'node:assert/strict';

import {
  createVolatileStatusStore,
  sameStatusIgnoringVolatile,
} from '../src/cad/status-cadence.ts';
import type { CadStatus } from '../src/cad/cad-status.ts';

function status(overrides: Record<string, unknown> = {}): CadStatus {
  return {
    frameCount: 1,
    cursorWorld: [1, 2, 0],
    documentLabel: 'drawing.dwg',
    loading: false,
    error: null,
    canOpen: true,
    canNew: true,
    canSave: true,
    canPlot: true,
    canFit: true,
    entityCount: 3,
    selection: { count: 0, refs: [] },
    commandLog: [],
    layouts: [],
    references: [],
    ...overrides,
  } as unknown as CadStatus;
}

test('no previous status is never equal: the first frame must apply', () => {
  assert.equal(sameStatusIgnoringVolatile(null, status()), false);
});

test('a frame that changes only the volatile pair does not re-render the shell', () => {
  const before = status();
  const after = status({
    frameCount: 99,
    cursorWorld: [10.5, -20.25, 0],
  });
  assert.equal(sameStatusIgnoringVolatile(before, after), true);

  // The same coordinate values are still volatile even when the array identity
  // differs, which is exactly what a freshly-parsed JSON frame produces.
  assert.equal(sameStatusIgnoringVolatile(status(), status({ cursorWorld: [1, 2, 0] })), true);
});

test('every semantic field change is applied immediately', () => {
  assert.equal(sameStatusIgnoringVolatile(status(), status({ error: 'boom' })), false);
  assert.equal(sameStatusIgnoringVolatile(status(), status({ loading: true })), false);
  assert.equal(sameStatusIgnoringVolatile(status(), status({ entityCount: 4 })), false);
  assert.equal(sameStatusIgnoringVolatile(status(), status({ documentLabel: null })), false);

  // Nested state is semantic too: selection/tool/layouts/references/command log.
  const selected = status({ selection: { count: 1, refs: [{ index: 7 }] } });
  assert.equal(sameStatusIgnoringVolatile(status(), selected), false);

  const named = status({ layouts: [{ id: 1, name: 'A', supported: true, reason: null, viewportCount: 2 }] });
  const renamed = status({ layouts: [{ id: 1, name: 'B', supported: true, reason: null, viewportCount: 2 }] });
  assert.equal(sameStatusIgnoringVolatile(named, renamed), false);

  const logged = status({ commandLog: [{ sequence: 1, kind: 'admitted', text: 'FIT' }] });
  assert.equal(sameStatusIgnoringVolatile(status(), logged), false);
});

test('a structurally identical nested frame (fresh arrays/objects) is equal', () => {
  const before = status({
    selection: { count: 1, refs: [{ index: 7, label: 'LINE' }] },
    layouts: [{ id: 1, name: 'A', supported: true, reason: null, viewportCount: 2 }],
  });
  const after = status({
    selection: { count: 1, refs: [{ index: 7, label: 'LINE' }] },
    layouts: [{ id: 1, name: 'A', supported: true, reason: null, viewportCount: 2 }],
  });
  assert.equal(sameStatusIgnoringVolatile(before, after), true);
});

test('added or removed keys are a semantic change', () => {
  const before = status({ tool: null });
  const without = status();
  delete (without as unknown as Record<string, unknown>).tool;
  assert.equal(sameStatusIgnoringVolatile(before, without), false);
  assert.equal(sameStatusIgnoringVolatile(without, before), false);
});

test('the leaf store notifies only when the rendered text or fps really changes', () => {
  const store = createVolatileStatusStore();
  const seen: unknown[] = [];
  const unsubscribe = store.subscribe(() => seen.push(store.getSnapshot()));

  const initial = store.getSnapshot();
  assert.equal(initial.fps, null);
  assert.equal(initial.cursor.text, '\u2014, \u2014');

  store.setCursor([1, 2, 0]);
  assert.equal(seen.length, 1);
  assert.match(store.getSnapshot().cursor.text, /^1\.0000, 2\.0000/);

  // A sub-display-precision move formats to the same text: no notification.
  store.setCursor([1.000001, 2, 0]);
  assert.equal(seen.length, 1);

  store.setCursor([3, 4, 0]);
  assert.equal(seen.length, 2);

  store.setFrameRate(60);
  assert.equal(seen.length, 3);
  store.setFrameRate(60);
  assert.equal(seen.length, 3);
  store.setFrameRate(59.5);
  assert.equal(seen.length, 4);
  store.setFrameRate(null);
  assert.equal(seen.length, 5);

  unsubscribe();
  store.setCursor([9, 9, 0]);
  assert.equal(seen.length, 5);
});

test('the store snapshot is stable while nothing visible changes', () => {
  const store = createVolatileStatusStore();
  const first = store.getSnapshot();
  store.setCursor([0, 0, 0]);
  const second = store.getSnapshot();
  assert.notEqual(first, second);
  store.setCursor([0, 0, 0]);
  assert.equal(store.getSnapshot(), second);
});

test('the cursor and fps selectors do not invalidate each other', () => {
  const store = createVolatileStatusStore();
  const cursor = store.getCursor();
  store.setFrameRate(59.5);
  assert.equal(store.getCursor(), cursor, 'an fps update keeps the cursor slice stable');

  const fps = store.getFrameRate();
  store.setCursor([7, 8, 0]);
  assert.equal(store.getFrameRate(), fps, 'a cursor update keeps the fps slice stable');
});
