// Responsive / mobile viewport-matrix contracts for the web host.
//
// This is the wasm-free, browser-free companion to the heavy Playwright checks
// (`check-web-mobile.mjs`, `check-web-ui.mjs`, `check-web-ribbon.mjs`). It pins
// the *pure classification logic* and the documented device/viewport matrix so
// a threshold drift is caught in CI without a browser or a GPU, exactly like
// `test-web-host.mjs` / `test-web-touch.mjs`.
//
// Both the Rust authority `cad-app::viewer_config::UiPresentationModel::resolve`
// and the Slint shell helper
// `cad-ui-slint::responsive::ResponsiveMetrics::derive` read the *same*
// `Breakpoints` config from `crates/cad-app/src/viewer_config.rs`: `mobile_below`
// defaults to 720 and `compact_below` to 1200, and a short surface
// (< 540 logical px tall) is compact even when its width is wide. On the Slint
// path the mode is mapped to `Breakpoint::Phone` (< 720), `Breakpoint` Tablet
// (unreachable at the 720/1200 defaults) and `Breakpoint::Desktop` (>= 1200).
//
// `UiPresentationModel::resolve` is the source of truth; the constants below
// **must** match it and its `tests`. If this test ever disagrees with Rust, the
// Rust side wins and this file is the bug.
//
// Run: node --test scripts/test-web-responsive.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// ---------------------------------------------------------------------------
// Thresholds replicated from `crates/cad-app/src/viewer_config.rs`:
//   `Breakpoints::default`  -> compact_below: 1200.0, mobile_below: 720.0
//   `UiPresentationModel::resolve` -> short landscape compact at height < 540.0
// ---------------------------------------------------------------------------
const MOBILE_BELOW = 720;
const COMPACT_BELOW = 1200;
const SHORT_HEIGHT = 540;

// `UiPresentationModel::resolve` touch targets (>= 48 mobile, else 36).
const MIN_TOUCH_TARGET = 48;
const MIN_POINTER_TARGET = 36;

// The matrices called out by `docs/responsive-ui.md` §1 / §6.
//   [name, logical width, logical height, devicePixelRatio]
const VIEWPORT_MATRIX = [
  ["phone-portrait", 360, 800, 3],
  ["phone-landscape", 800, 360, 3],
  ["tablet-portrait-short", 800, 1280, 2],
  ["desktop", 1280, 800, 1],
];
const DPXS = [1, 2, 3];
// top/right/bottom/left logical-pixel safe-area insets (notch + home bar).
const SAFE_INSETS = [
  [0, 0, 0, 0],
  [59, 0, 34, 0],
  [24, 48, 24, 0],
];

/**
 * Pure replica of the Rust layout classification.
 *
 * Mirrors `UiPresentationModel::resolve`:
 *   insets are top/right/bottom/left and are subtracted from the logical
 *   viewport before the breakpoints are applied; `mobileBelow` wins first, then
 *   `compactBelow` OR a short (< 540) surface, otherwise desktop.
 *
 * Never throws on a degenerate/zero viewport: the size clamp and the order of
 * the branches always yield a defined mode.
 */
export function classifyViewport(logicalWidth, logicalHeight, insets = [0, 0, 0, 0]) {
  const [top = 0, right = 0, bottom = 0, left = 0] = insets;
  // `Number.isFinite` sanitisation keeps a missing/NaN surface at 0 so the
  // comparison below is `true` and the degenerate case stays a phone.
  const sanitize = (value) => (Number.isFinite(value) ? value : 0);
  const width = Math.max(0, sanitize(logicalWidth) - right - left);
  const height = Math.max(0, sanitize(logicalHeight) - top - bottom);
  let mode;
  if (width < MOBILE_BELOW) mode = "mobile";
  else if (width < COMPACT_BELOW || height < SHORT_HEIGHT) mode = "compact";
  else mode = "desktop";
  return {
    mode,
    width,
    height,
    // Mobile is always touch, so it keeps the 48px floor; compact/desktop use
    // the finer pointer target. Width-based, matching the Rust resolution.
    touchTarget: mode === "mobile" ? MIN_TOUCH_TARGET : MIN_POINTER_TARGET,
    // Arrangement flags, mutually exclusive by construction.
    phone: mode === "mobile",
    compactShell: mode === "compact",
    desktop: mode === "desktop",
    drawer: mode === "mobile",
    sidePanel: mode === "desktop",
    floatingNav: mode === "desktop",
    // Safe-area consumers: the CSS inset expression must actually be fed.
    safeAreaInsets: [top, right, bottom, left],
  };
}

const MODES = new Set(["mobile", "compact", "desktop"]);

// ---------------------------------------------------------------------------
// 1. Breakpoint thresholds and the reference matrix.
// ---------------------------------------------------------------------------

test("reference viewports classify exactly like the Rust breakpoints", () => {
  const expected = new Map([
    ["phone-portrait", "mobile"],
    // A phone held landscape is 800 wide: wider than mobileBelow, and only
    // 360 tall, so the short-height rule makes it compact.
    ["phone-landscape", "compact"],
    ["tablet-portrait-short", "compact"],
    ["desktop", "desktop"],
  ]);
  // DPR is a rendering concern; the logical classification is DPR-independent,
  // so each matrix entry is checked at every documented device pixel ratio.
  for (const dpr of DPXS) {
    for (const [name, width, height] of VIEWPORT_MATRIX) {
      for (const insets of SAFE_INSETS) {
        const result = classifyViewport(width, height, insets);
        assert.equal(
          result.mode,
          expected.get(name),
          `${name} @dpr${dpr} insets=${insets}`,
        );
        assert.ok(MODES.has(result.mode));
      }
    }
  }
  // The matrix itself still advertises a DPR column: keep it honest.
  for (const [, , , dpr] of VIEWPORT_MATRIX) {
    assert.ok(DPXS.includes(dpr), `matrix dpr ${dpr} must be in ${DPXS}`);
  }
});

test("breakpoint boundaries match mobileBelow=720 / compactBelow=1200", () => {
  // Boundaries are exclusive on the low end (`<`).
  assert.equal(classifyViewport(719, 800).mode, "mobile");
  assert.equal(classifyViewport(720, 800).mode, "compact");
  assert.equal(classifyViewport(1199, 800).mode, "compact");
  assert.equal(classifyViewport(1200, 800).mode, "desktop");
  // Safety: the pinned constants are the documented defaults.
  assert.equal(MOBILE_BELOW, 720);
  assert.equal(COMPACT_BELOW, 1200);
});

test("a short surface stays compact even at desktop width (height < 540)", () => {
  assert.equal(classifyViewport(1280, 539).mode, "compact");
  assert.equal(classifyViewport(1280, 540).mode, "desktop");
  assert.equal(classifyViewport(1600, 300).mode, "compact");
});

test("safe-area insets are subtracted before classification", () => {
  // 800 wide landscape without insets: 800 >= 720 but only 360 tall, so the
  // short-height rule already makes it compact (see the reference matrix).
  assert.equal(classifyViewport(800, 360, [0, 0, 0, 0]).mode, "compact");
  // A 100px left inset drops 800 to 700 < 720, which is genuinely mobile.
  assert.equal(classifyViewport(800, 360, [0, 0, 0, 100]).mode, "mobile");
  // A wide desktop (1280) drops below 1200 with a 100px horizontal inset.
  assert.equal(classifyViewport(1280, 800, [0, 100, 0, 0]).mode, "compact");
  // A huge horizontal inset collapses 1280 to 880 < 1200, so it is no longer
  // desktop (still compact thanks to the wide height).
  assert.equal(classifyViewport(1280, 800, [0, 200, 0, 200]).mode, "compact");
  // A tall inset can push a desktop surface into the short-height rule.
  assert.equal(classifyViewport(1280, 800, [300, 0, 0, 0]).mode, "compact");
  // Insets are echoed back so the CSS expression has real values to consume.
  assert.deepEqual(classifyViewport(390, 844, [59, 0, 34, 0]).safeAreaInsets, [
    59, 0, 34, 0,
  ]);
});

// ---------------------------------------------------------------------------
// 2. Arrangement flags mirror `arrangement_flags_are_mutually_exclusive_and_wide_only`.
// ---------------------------------------------------------------------------

test("phone/compact/desktop arrangement flags are mutually exclusive", () => {
  for (const width of [320, 480, 600, 800, 1024, 1280, 1600]) {
    const result = classifyViewport(width, 800);
    const flags = [result.phone, result.compactShell, result.desktop];
    assert.equal(
      flags.filter(Boolean).length,
      1,
      `width ${width} must select exactly one arrangement`,
    );
    assert.equal(result.drawer, result.phone);
    assert.equal(result.sidePanel, result.desktop);
    assert.equal(result.floatingNav, result.desktop);
    if (result.phone) {
      assert.equal(result.drawer, true);
      assert.equal(result.sidePanel, false);
      assert.equal(result.floatingNav, false);
    }
    if (result.desktop) {
      assert.equal(result.drawer, false);
      assert.equal(result.sidePanel, true);
      assert.equal(result.floatingNav, true);
    }
  }
});

// ---------------------------------------------------------------------------
// 3. Degenerate / hostile input never throws and is always a phone.
// ---------------------------------------------------------------------------

test("a degenerate viewport classifies as mobile instead of throwing", () => {
  for (const [width, height] of [
    [0, 0],
    [-100, -100],
    [Number.NaN, Number.NaN],
    [undefined, undefined],
    [null, null],
  ]) {
    const result = classifyViewport(width, height);
    assert.equal(result.mode, "mobile", `${width}x${height}`);
    assert.equal(result.touchTarget, MIN_TOUCH_TARGET);
    assert.ok(result.width >= 0 && result.height >= 0);
  }
  // Insets larger than the viewport must not produce a negative surface.
  const clamped = classifyViewport(360, 800, [1000, 1000, 1000, 1000]);
  assert.equal(clamped.width, 0);
  assert.equal(clamped.height, 0);
  assert.equal(clamped.mode, "mobile");
});

// ---------------------------------------------------------------------------
// 4. Touch targets: >= 48 logical px on mobile, >= 36 elsewhere.
// ---------------------------------------------------------------------------

test("touch targets keep the 48px mobile floor and the 36px fine-pointer floor", () => {
  for (const width of [320, 360, 480, 599, 719]) {
    const result = classifyViewport(width, 800);
    assert.equal(result.mode, "mobile");
    assert.ok(
      result.touchTarget >= MIN_TOUCH_TARGET,
      `mobile width ${width} must keep >=48px targets`,
    );
  }
  for (const width of [720, 800, 1000, 1199, 1280]) {
    const result = classifyViewport(width, 800);
    assert.notEqual(result.mode, "mobile");
    assert.ok(
      result.touchTarget >= MIN_POINTER_TARGET,
      `non-mobile width ${width} must keep >=36px targets`,
    );
  }
});

// ---------------------------------------------------------------------------
// 5. Drift guards against the Rust source of truth and the served CSS.
// ---------------------------------------------------------------------------

test("pinned thresholds still match the Rust viewer_config source of truth", () => {
  const source = readFileSync(
    new URL("../crates/cad-app/src/viewer_config.rs", import.meta.url),
    "utf8",
  );
  assert.match(
    source,
    new RegExp(`compact_below:\\s*${COMPACT_BELOW}\\.0`),
    "Breakpoints::default compact_below drifted from this test",
  );
  assert.match(
    source,
    new RegExp(`mobile_below:\\s*${MOBILE_BELOW}\\.0`),
    "Breakpoints::default mobile_below drifted from this test",
  );
  assert.match(
    source,
    new RegExp(`height\\s*<\\s*${SHORT_HEIGHT}\\.0`),
    "the short-height compact rule drifted from this test",
  );
});

test("real phone orientations route to mobile or the short-landscape compact rule", () => {
  // Portrait phone: narrow enough for the mobile breakpoint.
  // Landscape phone: 800x360/800x390 is wider than 720, so it is *compact* —
  // not mobile — via the explicit `height < 540` rule, not via the empty
  // [600, 719) band. This documents that both real orientations are handled.
  for (const width of [360, 390, 414, 434, 480, 599]) {
    assert.equal(
      classifyViewport(width, 800).mode,
      "mobile",
      `${width}px portrait must route to mobile`,
    );
    assert.ok(
      width < MOBILE_BELOW,
      "real phone portrait widths stay below the mobile breakpoint",
    );
    // Rotated, the width is `height`-derived but the real surface is ~800 wide.
    assert.equal(
      classifyViewport(800, width).mode,
      "compact",
      `${width}px landscape must use the short-height compact rule`,
    );
  }
});
