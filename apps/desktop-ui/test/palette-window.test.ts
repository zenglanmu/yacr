// Contract tests for the shared floating-palette container.
//
// Two layers are pinned here:
//   * the pure geometry (defaults, cascade, drag/resize clamps, the header-hit
//     predicate) without any DOM, and
//   * the source/CSS structure that is what actually keeps the CAD partition
//     rect untouched: one `.palette-layer` overlay *after* `.native-input`, the
//     six palettes inside it, and no palette left in `.workspace__left`.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCss, readModule } from './helpers.ts';
import {
  PALETTE_CASCADE_STEP,
  PALETTE_DEFAULT_WIDTH,
  PALETTE_DRAG_BLOCKER_SELECTOR,
  PALETTE_HEADER_HEIGHT,
  PALETTE_MAX_DEFAULT_WIDTH,
  PALETTE_MIN_VISIBLE_WIDTH,
  PALETTE_MIN_WIDTH,
  cascadeOffset,
  clampGeometry,
  clampLeft,
  clampTop,
  clampWidth,
  defaultPaletteGeometry,
  defaultPaletteWidth,
  isDragHandleTarget,
  moveGeometry,
  resizeGeometry,
} from '../src/layout/palette-window.ts';

const LAYER = { width: 1000, height: 600 };

/** A target whose `closest` answers with `hit` (null = plain header chrome). */
function target(hit) {
  return {
    closest(selector) {
      assert.equal(selector, PALETTE_DRAG_BLOCKER_SELECTOR);
      return hit;
    },
  };
}

test('a palette defaults to the left edge and cascades down by a fixed step', () => {
  assert.equal(PALETTE_DEFAULT_WIDTH, 300);
  assert.equal(PALETTE_CASCADE_STEP, 24);
  assert.deepEqual(defaultPaletteGeometry(), { left: 0, top: 0, width: 300 });
  assert.deepEqual(defaultPaletteGeometry(2), { left: 0, top: 48, width: 300 });
  // The six palettes open at 0/24/48/72/96/120, so no two headers coincide.
  assert.deepEqual([0, 1, 2, 3, 4, 5].map(cascadeOffset), [0, 24, 48, 72, 96, 120]);
  // A missing/negative/non-finite slot never invents a position.
  assert.equal(cascadeOffset(-3), 0);
  assert.equal(cascadeOffset(Number.NaN), 0);
});

test('a palette opens wider on a wide drawing, within [300, 460]', () => {
  assert.equal(PALETTE_DEFAULT_WIDTH, 300);
  assert.equal(PALETTE_MAX_DEFAULT_WIDTH, 460);
  assert.equal(defaultPaletteWidth(), 300);
  assert.equal(defaultPaletteWidth(Number.NaN), 300);
  assert.equal(defaultPaletteWidth(0), 300);
  assert.equal(defaultPaletteWidth(-1000), 300);
  // 32% of the drawing, never below 300 and never above 460.
  assert.equal(defaultPaletteWidth(500), 300);
  assert.equal(defaultPaletteWidth(1000), 320);
  assert.equal(defaultPaletteWidth(1250), 400);
  assert.equal(defaultPaletteWidth(1600), 460);
  assert.equal(defaultPaletteWidth(100000), 460);
  // The placement stays at the left edge and keeps its cascade.
  assert.deepEqual(defaultPaletteGeometry(1, 1000), { left: 0, top: 24, width: 320 });
  assert.deepEqual(defaultPaletteGeometry(1), { left: 0, top: 24, width: 300 });
});

test('dragging clamps at all four edges and keeps 80px of width visible', () => {
  const start = defaultPaletteGeometry(0);
  // Left: the window may overhang, but 80px (its right edge) stays in the layer.
  assert.equal(moveGeometry(start, -10000, 0, LAYER).left, -(PALETTE_DEFAULT_WIDTH - PALETTE_MIN_VISIBLE_WIDTH));
  // Right: 80px (its left edge) stays in the layer.
  assert.equal(moveGeometry(start, 10000, 0, LAYER).left, LAYER.width - PALETTE_MIN_VISIBLE_WIDTH);
  // Top: the header can never be dragged above the layer.
  assert.equal(moveGeometry(start, 0, -10000, LAYER).top, 0);
  // Bottom: the whole header stays visible inside the layer.
  assert.equal(moveGeometry(start, 0, 10000, LAYER).top, LAYER.height - PALETTE_HEADER_HEIGHT);
  // An interior move is exact and never changes width.
  assert.deepEqual(moveGeometry(start, 120, 80, LAYER), { left: 120, top: 80, width: 300 });
});

test('resizing clamps to [280, layerWidth] and re-clamps the position', () => {
  const start = defaultPaletteGeometry(0);
  assert.equal(resizeGeometry(start, 100, LAYER).width, PALETTE_MIN_WIDTH);
  assert.equal(resizeGeometry(start, 400, LAYER).width, 400);
  assert.equal(resizeGeometry(start, 5000, LAYER).width, LAYER.width);
  // A window overhanging the left edge is pulled back when it narrows: the
  // visible share must still be inside the layer.
  const overhanging = { left: -220, top: 0, width: 300 };
  assert.deepEqual(resizeGeometry(overhanging, 220, LAYER), { left: -200, top: 0, width: 280 });
  // A layer narrower than the minimum still allows the minimum (min wins): the
  // window would otherwise become unusable, and the layer clips it visually.
  assert.equal(clampWidth(300, 100), PALETTE_MIN_WIDTH);
  assert.equal(clampTop(100, 10), 0);
});

test('a whole placement is clamped before it is rendered or dragged', () => {
  // Width is clamped first; a full-width window can keep only 80px on either
  // side, and a leftward request resolves to the left-edge limit.
  assert.deepEqual(clampGeometry({ left: -9999, top: 9999, width: 9999 }, LAYER), {
    left: -(LAYER.width - PALETTE_MIN_VISIBLE_WIDTH),
    top: LAYER.height - PALETTE_HEADER_HEIGHT,
    width: LAYER.width,
  });
  assert.equal(clampGeometry({ left: 9999, top: 0, width: 300 }, LAYER).left, LAYER.width - PALETTE_MIN_VISIBLE_WIDTH);
  assert.equal(clampLeft(0, 300, LAYER.width), 0);
});

test('the header drag handle is chrome, never a control', () => {
  // No target and a target with no `closest` fail closed.
  assert.equal(isDragHandleTarget(null), false);
  assert.equal(isDragHandleTarget(undefined), false);
  assert.equal(isDragHandleTarget({}), false);
  // Plain header chrome (title text / empty space) is the handle.
  assert.equal(isDragHandleTarget(target(null)), true);
  // Any interactive descendant blocks the drag, so a click never becomes one.
  assert.equal(isDragHandleTarget(target({ tag: 'button' })), false);
  for (const part of ['button', 'a', 'input', 'select', 'textarea', '[role="button"]', '[data-no-drag]']) {
    assert.ok(PALETTE_DRAG_BLOCKER_SELECTOR.includes(part), `the blocker selector names ${part}`);
  }
});

test('the responsive default is applied once and never overwrites a user resize', async () => {
  const source = await readModule('components/PaletteWindow.tsx');
  assert.ok(source.includes('defaultPaletteWidth(layerWidth)'), 'the first measurement reads the responsive width');
  assert.ok(source.includes('if (current.touched || current.sized) return;'), 'a touched or already-sized window is left alone');
  assert.ok(source.includes('touched: true, sized: true'), 'a drag/resize marks the window as user-owned');
  assert.ok(
    source.includes('applyResponsiveDefault(windowKey, initialIndex, measuredWidth)'),
    'the measured layer width is what chooses the default',
  );
  // The close/reopen restore contract is unchanged: the store is still keyed by
  // windowKey and the stored geometry is still what a reopen reads.
  assert.ok(source.includes('STATES.set(windowKey'), 'geometry is still stored per window');
});

test('the shared container owns drag, resize, keyboard and geometry state', async () => {
  const source = await readModule('components/PaletteWindow.tsx');
  assert.ok(source.includes('useSyncExternalStore'), 'geometry survives close/reopen');
  assert.ok(source.includes("'palette-window'"));
  assert.ok(source.includes("'is-dragging'"));
  assert.ok(source.includes('setPointerCapture'));
  assert.ok(source.includes('onPointerMove'));
  assert.ok(source.includes('onPointerUp'));
  assert.ok(source.includes('onPointerCancel'));
  assert.ok(source.includes('maxHeight: `calc(100% - ${view.top}px)`'), 'content is capped by the layer edge');
  // The right-edge grip is an accessible, keyboard-operable separator.
  assert.ok(source.includes('className="palette-window__resize"'));
  assert.ok(source.includes('role="separator"'));
  assert.ok(source.includes('aria-orientation="vertical"'));
  assert.ok(source.includes("aria-label={t('palette.resize_width')}"));
  assert.ok(source.includes('aria-valuemin={PALETTE_MIN_WIDTH}'));
  assert.ok(source.includes('aria-valuemax={maximumWidth}'));
  assert.ok(source.includes('aria-valuenow={Math.round(view.width)}'));
  assert.ok(source.includes('tabIndex={0}'));
  assert.ok(source.includes("event.key !== 'ArrowLeft'"));
  assert.ok(source.includes("event.key !== 'ArrowRight'"));
  // No palette-specific branch: one container serves all six windows.
  for (const name of ['PropertiesPalette', 'LayersPalette', 'FileInfoPalette', 'DiagnosticsPalette', 'ReferencesPanel', 'FontsPanel']) {
    assert.equal(source.includes(`import { ${name} }`), false, `${name} must stay out of the shared container`);
  }
});

test('Workspace floats the six palettes in one layer after the native input', async () => {
  const workspace = await readModule('components/Workspace.tsx');
  assert.ok(workspace.includes("import { PaletteWindow } from './PaletteWindow.tsx';"));

  // The left column is the start screen's alone; no palette flag widens it.
  assert.ok(workspace.includes('const leftOpen = showStart;'));
  assert.ok(!workspace.includes('const leftOpen = showStart ||'));
  const leftColumn = workspace.slice(
    workspace.indexOf('"workspace__left"'),
    workspace.indexOf('"workspace__canvas"'),
  );
  for (const palette of ['PropertiesDock', 'LayersPalette', 'FileInfoPalette', 'DiagnosticsPalette', 'ReferencesPanel', 'FontsPanel']) {
    assert.equal(leftColumn.includes(palette), false, `${palette} must not live in .workspace__left`);
  }

  // The layer is inside the drawing area and after the native input, so it
  // paints above the CAD surface without being a child of it.
  assert.ok(workspace.indexOf('"workspace__canvas"') < workspace.indexOf('className="palette-layer"'));
  assert.ok(workspace.indexOf('ref={shell.surfaceRef}') < workspace.indexOf('className="palette-layer"'));

  // All six palettes are wrapped by the shared container, in a fixed order.
  const expected = [
    ['properties', 'PropertiesDock', 0],
    ['layers', 'LayersPalette', 1],
    ['fileinfo', 'FileInfoPalette', 2],
    ['diagnostics', 'DiagnosticsPalette', 3],
    ['references', 'ReferencesPanel', 4],
    ['fonts', 'FontsPanel', 5],
  ];
  let previous = workspace.indexOf('className="palette-layer"');
  for (const [key, component, index] of expected) {
    const pattern = new RegExp(
      `<PaletteWindow windowKey="${key}" initialIndex=\\{${index}\\}>\\s*<${component} />\\s*</PaletteWindow>`,
    );
    const match = pattern.exec(workspace);
    assert.ok(match, `${key} is wrapped by PaletteWindow`);
    assert.ok(match.index > previous, `${key} follows the previous palette in the layer`);
    previous = match.index;
  }
});

test('the CSS makes the layer a pointer-transparent overlay and each window opaque', async () => {
  const css = await readCss();
  const rule = selector => {
    const match = css.match(new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`));
    assert.ok(match, `${selector} rule exists`);
    return match[1];
  };
  const layer = rule('.palette-layer');
  assert.match(layer, /position:\s*absolute/);
  assert.match(layer, /inset:\s*0/);
  assert.match(layer, /pointer-events:\s*none/);
  assert.match(layer, /z-index:\s*\d+/);

  const window = rule('.palette-window');
  assert.match(window, /position:\s*absolute/);
  assert.match(window, /pointer-events:\s*auto/);
  assert.match(window, /background:\s*var\(--chrome-3\)/);
  assert.match(window, /display:\s*flex/);
  assert.match(window, /overflow:\s*hidden/);

  const resize = rule('.palette-window__resize');
  assert.match(resize, /cursor:\s*col-resize/);
  assert.match(resize, /touch-action:\s*none/);
  assert.match(rule('.palette-window .palette__head'), /cursor:\s*grab/);

  // The compact layout keeps its own sheets: the desktop layer is hidden.
  const mobile = await readModule('mobile.css');
  assert.ok(mobile.includes(':root[data-layout="mobile"] .palette-layer'));
});
