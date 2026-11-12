// Contract tests for QuickCalc's floating-window geometry.
//
// QuickCalc is the one non-modal calculator float: `position: fixed` in viewport
// coordinates, dragged by its header, closable and reopenable without losing
// where the user put it. The pure arithmetic lives in
// `src/layout/quick-calc-window.ts`, so this suite pins it without a DOM:
//
//   * the first placement is a lower-right corner that is fully on-screen with
//     the promised 16px gutter (not the palette's left dock);
//   * a drag clamps at every edge, and the horizontal clamp still keeps 80px of
//     width visible so the header never becomes ungrabbable;
//   * a viewport too small for the card still keeps the header reachable;
//   * the two mode widths match the stylesheet's `min(width, 100vw - 16px)`.
//
// `.ts` by filename only: Node's type stripping runs it, `tsc` never sees it.
import test from 'node:test';
import assert from 'node:assert/strict';

import {
  PALETTE_HEADER_HEIGHT,
  PALETTE_MIN_VISIBLE_WIDTH,
} from '../src/layout/palette-window.ts';
import { readCss, readModule } from './helpers.ts';
import {
  QUICKCALC_BASIC_HEIGHT,
  QUICKCALC_BASIC_WIDTH,
  QUICKCALC_EDGE_MARGIN,
  QUICKCALC_SCIENTIFIC_HEIGHT,
  QUICKCALC_SCIENTIFIC_WIDTH,
  clampQuickCalc,
  defaultQuickCalcGeometry,
  moveQuickCalc,
  normalizeViewport,
  quickCalcNominalHeight,
  quickCalcNominalWidth,
  quickCalcWidth,
} from '../src/layout/quick-calc-window.ts';

const VIEWPORT = { width: 1280, height: 800 };

test('the scientific card is one key column wider and taller than the basic one', () => {
  assert.equal(QUICKCALC_BASIC_WIDTH, 384);
  assert.equal(QUICKCALC_SCIENTIFIC_WIDTH, 473);
  assert.ok(QUICKCALC_SCIENTIFIC_WIDTH > QUICKCALC_BASIC_WIDTH, 'the function strip needs its own column');
  assert.ok(QUICKCALC_SCIENTIFIC_HEIGHT > QUICKCALC_BASIC_HEIGHT, 'three function rows need height');
  assert.equal(quickCalcNominalWidth('basic'), QUICKCALC_BASIC_WIDTH);
  assert.equal(quickCalcNominalWidth('scientific'), QUICKCALC_SCIENTIFIC_WIDTH);
  assert.equal(quickCalcNominalHeight('basic'), QUICKCALC_BASIC_HEIGHT);
  assert.equal(quickCalcNominalHeight('scientific'), QUICKCALC_SCIENTIFIC_HEIGHT);
});

test('the clamp width follows min(nominal, 100vw - 16px)', () => {
  // A desktop viewport gets the full mode width.
  assert.equal(quickCalcWidth('basic', VIEWPORT.width), QUICKCALC_BASIC_WIDTH);
  assert.equal(quickCalcWidth('scientific', VIEWPORT.width), QUICKCALC_SCIENTIFIC_WIDTH);
  // A viewport narrower than the card hands over the whole width minus the gutter.
  assert.equal(quickCalcWidth('scientific', 400), 400 - QUICKCALC_EDGE_MARGIN);
  assert.equal(quickCalcWidth('basic', 300), 300 - QUICKCALC_EDGE_MARGIN);
  // Below the shared 280px floor the floor wins, so the card stays usable.
  assert.equal(quickCalcWidth('scientific', 200), 280);
  assert.equal(quickCalcWidth('basic', 200), 280);
  // No measurement is not an instruction: the nominal width is kept.
  assert.equal(quickCalcWidth('scientific', undefined), QUICKCALC_SCIENTIFIC_WIDTH);
  assert.equal(quickCalcWidth('scientific', Number.NaN), QUICKCALC_SCIENTIFIC_WIDTH);
  assert.equal(quickCalcWidth('basic', 0), QUICKCALC_BASIC_WIDTH);
});

test('the first placement is a lower-right corner inside the viewport', () => {
  const placement = defaultQuickCalcGeometry(VIEWPORT, quickCalcWidth('basic', VIEWPORT.width));
  assert.deepEqual(placement, { left: 880, top: 344, width: 384 });
  // Fully on-screen with the promised gutter on the right and at the bottom.
  assert.equal(placement.left + placement.width, VIEWPORT.width - QUICKCALC_EDGE_MARGIN);
  assert.equal(placement.top + QUICKCALC_BASIC_HEIGHT, VIEWPORT.height - QUICKCALC_EDGE_MARGIN);
  assert.ok(placement.left >= 0 && placement.top >= 0);
  // Right half, lower half: it never drifts to the palette's left dock.
  assert.ok(placement.left > VIEWPORT.width / 2);
  assert.ok(placement.top > VIEWPORT.height / 3);
});

test('a drag clamps at all four edges', () => {
  const start = defaultQuickCalcGeometry(VIEWPORT, quickCalcWidth('basic', VIEWPORT.width));
  // Left: the card may overhang, but 80px of its right edge stays inside.
  assert.equal(moveQuickCalc(start, -10000, 0, VIEWPORT).left, -(start.width - PALETTE_MIN_VISIBLE_WIDTH));
  // Right: 80px of its left edge stays inside.
  assert.equal(moveQuickCalc(start, 10000, 0, VIEWPORT).left, VIEWPORT.width - PALETTE_MIN_VISIBLE_WIDTH);
  // Top: the header can never be dragged above the viewport.
  assert.equal(moveQuickCalc(start, 0, -10000, VIEWPORT).top, 0);
  // Bottom: the whole header stays reachable inside the viewport.
  assert.equal(moveQuickCalc(start, 0, 10000, VIEWPORT).top, VIEWPORT.height - PALETTE_HEADER_HEIGHT);
  // An interior move is exact and never changes the width.
  assert.deepEqual(moveQuickCalc(start, -120, -80, VIEWPORT), { left: 760, top: 264, width: 384 });
});

test('a stored placement is re-clamped after the viewport changes', () => {
  const wide = defaultQuickCalcGeometry({ width: 1600, height: 900 }, QUICKCALC_BASIC_WIDTH);
  const narrow = clampQuickCalc(wide, { width: 640, height: 480 });
  assert.ok(narrow.left <= 640 - PALETTE_MIN_VISIBLE_WIDTH, 'the card is pulled back to the smaller viewport');
  assert.ok(narrow.top + PALETTE_HEADER_HEIGHT <= 480, 'the header stays visible');
  assert.ok(narrow.width <= 640);
});

test('a tiny viewport still keeps the header reachable', () => {
  const tiny = { width: 200, height: 120 };
  const placement = defaultQuickCalcGeometry(
    tiny,
    quickCalcWidth('scientific', tiny.width),
    quickCalcNominalHeight('scientific'),
  );
  // The floor width is kept (the card overhangs, but it stays usable) and the
  // header is pinned to the top so it can always be grabbed.
  assert.equal(placement.width, 280);
  assert.equal(placement.top, 0);
  assert.ok(placement.top + PALETTE_HEADER_HEIGHT <= tiny.height);
  assert.ok(placement.left >= -(placement.width - PALETTE_MIN_VISIBLE_WIDTH));
  assert.ok(placement.left <= tiny.width - PALETTE_MIN_VISIBLE_WIDTH);
});

test('a missing or non-finite viewport never produces NaN geometry', () => {
  assert.deepEqual(normalizeViewport({ width: Number.NaN, height: Number.POSITIVE_INFINITY }), { width: 0, height: 0 });
  assert.deepEqual(normalizeViewport({ width: -50, height: 100.4 }), { width: 0, height: 100 });
  const placement = defaultQuickCalcGeometry({ width: 0, height: 0 });
  for (const value of [placement.left, placement.top, placement.width]) {
    assert.ok(Number.isFinite(value), 'every placement component is a finite number');
  }
  assert.equal(placement.top, 0);
});

test('QuickCalc is a float, not a modal dialog', async () => {
  const source = await readModule('components/QuickCalcDialog.tsx');
  // No scrim, no modal semantics, no focus trap: the drawing stays usable.
  assert.ok(!source.includes('popup-overlay'));
  assert.doesNotMatch(source, /aria-modal=/);
  assert.ok(!source.includes('useConfirmDialogFocus'));
  // The shared non-modal hook owns Escape/focus restore, and the expression
  // field still gets the focus on open.
  assert.ok(source.includes("import { useSurfacePanelFocus } from './surface-focus.ts';"));
  assert.ok(source.includes('useSurfacePanelFocus(onClose)'));
  assert.ok(source.includes('inputRef.current?.focus'));
  // The dialog identity the router/automation looks for is unchanged.
  assert.ok(source.includes('data-yacr-dialog="quickcalc"'));
  assert.ok(source.includes('role="dialog"'));
  assert.ok(source.includes('tabIndex={-1}'));
});

test('the header drag reuses the shared predicate and pointer capture', async () => {
  const source = await readModule('components/QuickCalcDialog.tsx');
  assert.ok(source.includes("import { isDragHandleTarget } from '../layout/palette-window.ts';"));
  assert.ok(source.includes('isDragHandleTarget(event.target'));
  assert.ok(source.includes('setPointerCapture'));
  assert.ok(source.includes('releasePointerCapture'));
  assert.ok(source.includes('onPointerMove'));
  assert.ok(source.includes('onPointerUp'));
  assert.ok(source.includes('onPointerCancel'));
  // The drag maths goes through the pure helper, not inline arithmetic.
  assert.ok(source.includes('moveQuickCalc('));
  assert.ok(source.includes('clampQuickCalc('));
  assert.ok(source.includes('defaultQuickCalcGeometry('));
  // The position survives close/reopen through a module-level store.
  assert.ok(source.includes('let placement: Placement | null = null;'));
  assert.ok(source.includes('useSyncExternalStore(subscribePlacement'));
});

test('the keypad keeps the familiar numeric block and wires the scientific keys', async () => {
  const source = await readModule('components/QuickCalcDialog.tsx');
  for (const digit of ['7', '8', '9', '4', '5', '6', '1', '2', '3', '0']) {
    assert.ok(source.includes(`tokenKey('${digit}', null, '${digit}', 'digit')`), `digit ${digit} is unchanged`);
  }
  assert.ok(source.includes("key('±', 'quickcalc.plus_minus'"));
  assert.ok(source.includes("tokenKey('.', 'quickcalc.decimal', '.')"));
  assert.ok(source.includes("key('=', 'quickcalc.equals', evaluateCalc, 'equals')"));
  // Every scientific key is implemented by the engine, never a placeholder.
  for (const [face, keyName, token] of [
    ['sin', 'quickcalc.sci.sin', 'sin('],
    ['cos', 'quickcalc.sci.cos', 'cos('],
    ['tan', 'quickcalc.sci.tan', 'tan('],
    ['asin', 'quickcalc.sci.asin', 'asin('],
    ['acos', 'quickcalc.sci.acos', 'acos('],
    ['atan', 'quickcalc.sci.atan', 'atan('],
    ['ln', 'quickcalc.sci.ln', 'ln('],
    ['log', 'quickcalc.sci.log', 'log('],
    ['sqrt', 'quickcalc.sci.sqrt', 'sqrt('],
    ['abs', 'quickcalc.sci.abs', 'abs('],
    ['xʸ', 'quickcalc.sci.power', '^'],
    ['π', 'quickcalc.sci.pi', 'pi'],
    ['e', 'quickcalc.sci.e', 'e'],
  ]) {
    assert.ok(
      source.includes(`fnKey('${face}', '${keyName}', '${token}')`),
      `${face} inserts the ${token} token`,
    );
  }
  assert.ok(source.includes('appendCalcToken(state, value)'), 'the function keys share one token transition');
  assert.ok(source.includes("unaryKey('x²', 'quickcalc.sci.square', 'square')"));
  assert.ok(source.includes("unaryKey('1/x', 'quickcalc.sci.inverse', 'inverse')"));
  assert.ok(source.includes('applyCalcUnary(state, kind)'), 'the unary keys share one transform transition');
  assert.ok(source.includes('toggleCalcAngle(state)'), 'the DEG/RAD switch is wired');
  assert.ok(source.includes("inputMode={scientific ? 'text' : 'decimal'}"));
  assert.ok(source.includes('aria-pressed={mode === value}'));
});

test('the stylesheet stacks the float above quick-view and below the popup layer', async () => {
  const css = await readCss();
  const rule = selector => {
    const match = css.match(new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`));
    assert.ok(match, `${selector} rule exists`);
    return match[1];
  };
  const window = rule('.quickcalc-window');
  assert.match(window, /position:\s*fixed/);
  assert.match(window, /z-index:\s*55/);
  assert.match(window, /width:\s*min\(384px, calc\(100vw - 16px\)\)/);
  assert.match(window, /max-height:\s*calc\(100vh - 16px\)/);
  // 55 is above `.quick-view` (50) and the palette layer (10), below `.popup-overlay` (60).
  const quickView = Number(/z-index:\s*(\d+)/.exec(rule('.quick-view'))[1]);
  const overlay = Number(/z-index:\s*(\d+)/.exec(rule('.popup-overlay'))[1]);
  const paletteLayer = Number(/z-index:\s*(\d+)/.exec(rule('.palette-layer'))[1]);
  const own = Number(/z-index:\s*(\d+)/.exec(window)[1]);
  assert.ok(own > quickView && own > paletteLayer, 'the float is above the drawing chrome');
  assert.ok(own < overlay, 'a modal still covers the float');
  // Scientific mode is one key column wider and the keypad is the same module.
  assert.match(rule('.quickcalc-window.is-scientific'), /width:\s*min\(473px, calc\(100vw - 16px\)\)/);
  assert.match(rule('.quickcalc__keys'), /repeat\(4, minmax\(0, 1fr\)\)/);
  assert.match(rule('.quickcalc__funcs'), /repeat\(5, minmax\(0, 1fr\)\)/);
  assert.match(rule('.quickcalc__funcs'), /grid-auto-flow:\s*column/);
  // Coarse pointers get 40px touch targets.
  assert.match(css, /@media \(pointer: coarse\) \{[\s\S]*?\.quickcalc__key \{ height: 40px; \}/);
  assert.match(rule('.quickcalc-window__head'), /cursor:\s*grab/);
});
