// Mobile-shell panel wiring parity across the Linux, Web and Android hosts.
//
// This is a *source-text* contract, not a device or browser test. After P2/P4 the
// shared "application state → shell" leg is no longer a per-host `HostCore::push`:
//
//   cad-host `state.rs`  → typed `HostState` / `HostUpdates` projection
//   cad-ui-slint         → `EngineAdapter::apply_updates` / `UiHandle::apply_host_state`
//   platform wrappers    → re-apply only their host-specific overlay after the flush
//
// The contract pins every required panel/overlay channel to the *actual* owner
// (shared projection, UI apply, or a named platform overlay), plus the ordering
// rule that a platform overlay never overrides the shared authority channels.
// Rendering correctness is out of scope; see docs/panels.md §3.2.
//
// The pure helpers (`missing`, `runsAfter`, `presentIn`, `compact`) are exported
// so the negative-fixture tests can mutate a source and assert the check rejects
// it, without ever widening the search to the whole repository or weakening an
// assertion to "non-empty".
//
// Run: node --test scripts/test-host-panel-parity.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const ROOT = new URL("../", import.meta.url);

function readSource(relative) {
  return readFileSync(new URL(relative, ROOT), "utf8");
}

/** Whitespace-insensitive view so a multi-line Rust expression can be matched. */
export function compact(source) {
  return source.replace(/\s+/g, "");
}

/** Required substrings that a source is missing (empty means all present). */
export function missing(source, required) {
  return required.filter((symbol) => !source.includes(symbol));
}

/** Whether `first` occurs before `then` (both present); the overlay order rule. */
export function runsAfter(source, first, then) {
  const a = source.indexOf(first);
  const b = source.indexOf(then);
  return a !== -1 && b !== -1 && a < b;
}

/** Forbidden substrings that a source wrongly contains (empty means clean). */
export function presentIn(source, forbidden) {
  return forbidden.filter((symbol) => source.includes(symbol));
}

// --- 1. Shared typed projection (UI-independent) ---------------------------

const SHARED_PROJECTION = "crates/cad-host/src/state.rs";
// `take_updates` (the dirty-markers → typed batch publication) lives with the
// `EngineClient` implementation, not the derivation module.
const SHARED_PUBLICATION = "crates/cad-host/src/input.rs";

// Every channel the shared `HostState`/`NavigationState` projection must derive.
// These replace the pre-P2 `set_*`/`set_overlay_*` panel entry points: the host is
// UI-independent and publishes typed state; the UI adapter below owns the concrete
// shell setters.
const SHARED_PROJECTION_CHANNELS = [
  "fn full_state",
  "fn navigation_state",
  // Panel rows / selections (counts derive from these raw rows).
  "layer_rows",
  "selection_properties",
  "layouts",
  "active_space",
  // Tool + measurement + units previews.
  "draw_preview",
  "selected_draw_kind",
  "measurement_preview",
  "units",
  // Diagnostics / import / resources summaries.
  "diagnostics",
  "import_progress",
  "resources",
  // Render overlays (selection highlight / measurement / snap hints / config gate).
  "OverlayInputs",
  "set_overlays",
];

// The effective-config overlay gate is the render-side `OverlayVisibility::from`
// over the authoritative viewer config; an overlay the config disables is never
// drawn. Whitespace-insensitive so the multi-line expression still matches.
const OVERLAY_GATE_COMPACT =
  "OverlayVisibility::from(self.viewer_config.borrow().effective().view.overlays";

test("shared projection (crates/cad-host/src/state.rs) derives every panel/overlay channel", () => {
  const source = readSource(SHARED_PROJECTION);
  assert.deepEqual(
    missing(source, SHARED_PROJECTION_CHANNELS),
    [],
    `${SHARED_PROJECTION} is missing required panel/overlay channels; ` +
      `the shared typed projection has drifted from the panel surface`,
  );
});

test("shared publication (crates/cad-host/src/input.rs) emits the typed HostUpdates batch", () => {
  const source = readSource(SHARED_PUBLICATION);
  assert.deepEqual(
    missing(source, ["fn take_updates", "HostUpdates"]),
    [],
    `${SHARED_PUBLICATION} must publish the typed HostUpdates batch ` +
      `(take_updates); the shared state feed has drifted`,
  );
});

test("shared projection gates render overlays on the effective config", () => {
  const source = readSource(SHARED_PROJECTION);
  assert.ok(
    compact(source).includes(OVERLAY_GATE_COMPACT),
    `${SHARED_PROJECTION} must gate render overlays on the effective config ` +
      `(OverlayVisibility::from(...effective().view.overlays...)) so an overlay ` +
      `the config disables is never drawn`,
  );
});

// --- 2. UI apply pipeline (shared for every host) --------------------------

const UI_APPLY = "crates/cad-ui-slint/src/engine_adapter.rs";
const UI_HANDLE = "crates/cad-ui-slint/src/handle.rs";

// The single shared application path. `apply_updates` is the only writer that
// turns a `HostUpdates` batch into shell state.
const UI_APPLY_CHANNELS = [
  "fn apply_updates",
  "HostUpdates",
  "apply_backend_state",
  "apply_config_snapshot",
  "apply_host_state",
  "set_snap_hints",
  "set_view_state",
  "set_status_message",
];

const UI_HANDLE_CHANNELS = [
  "fn apply_host_state",
  "fn apply_config_snapshot",
  "fn apply_backend_state",
  // Full-state panel setters.
  "set_layer_state",
  "set_property_state",
  "set_layout_state",
  "set_diagnostics_state",
  "set_measurement_state",
  "set_draw_state",
  "set_import_state",
  "set_resources_sections",
];

// Units must render their explicit label, never empty.
const UI_HANDLE_UNIT_BEHAVIOR = ["UnitContext", "drawing_units"];

test("UI adapter applies every HostUpdates channel", () => {
  const source = readSource(UI_APPLY);
  assert.deepEqual(
    missing(source, UI_APPLY_CHANNELS),
    [],
    `${UI_APPLY} is missing a required HostUpdates application channel; ` +
      `the shared UI apply path has drifted`,
  );
});

test("UI handle applies every panel setter for the shared full state", () => {
  const source = readSource(UI_HANDLE);
  assert.deepEqual(
    missing(source, UI_HANDLE_CHANNELS),
    [],
    `${UI_HANDLE} is missing a required panel setter for the shared full state`,
  );
  assert.deepEqual(
    missing(source, UI_HANDLE_UNIT_BEHAVIOR),
    [],
    `${UI_HANDLE} must render the explicit unit label (drawing units fallback)`,
  );
});

test("required panel channels are real UiHandle public setters", () => {
  const handle = readSource(UI_HANDLE);
  // Parse `pub fn set_<name>` declarations. The handle also exposes non-panel
  // setters (config, locale, status, …); the required panel set must be a
  // subset, never the whole set.
  const declared = new Set(
    [...handle.matchAll(/pub\s+fn\s+(set_[A-Za-z0-9_]+)/g)].map((m) => m[1]),
  );
  const required = [
    "set_layer_state",
    "set_property_state",
    "set_layout_state",
    "set_diagnostics_state",
  ];
  for (const setter of required) {
    assert.ok(
      declared.has(setter),
      `${UI_HANDLE} no longer declares UiHandle::${setter}; ` +
        `a renamed or removed panel setter breaks every host`,
    );
  }
  // A source-only sanity check: the handle really exposed a broader set, so the
  // subset assertion above is meaningful rather than accidentally total.
  assert.ok(
    declared.size > required.length,
    "expected UiHandle to expose more set_* methods than the required panel subset",
  );
});

// --- 3. Platform overlays (host-specific), ordered, authority-preserving ----

// A platform overlays only the channels the shared host cannot own; it must run
// *after* the shared flush and must never set the shared authority setters.
const AUTHORITY_SETTERS = [
  "set_layer_state",
  "set_property_state",
  "set_layout_state",
  "set_diagnostics_state",
  "apply_host_state",
];

const WEB_OVERLAY = "apps/app-web/src/browser/state_push.rs";
const WEB_BROWSER = "apps/app-web/src/browser.rs";
const ANDROID_OVERLAY = "apps/app-android/src/state_push.rs";
const ANDROID_POLL = "apps/app-android/src/poll.rs";

const PLATFORM_OVERLAY_CHANNELS = ["push_import_state", "push_resource_sections"];

test("web host re-applies the browser-specific overlay after the shared flush", () => {
  const overlay = readSource(WEB_OVERLAY);
  assert.deepEqual(
    missing(overlay, PLATFORM_OVERLAY_CHANNELS),
    [],
    `${WEB_OVERLAY} is missing a browser-specific overlay channel`,
  );
  assert.ok(
    overlay.includes("images_modeled: false"),
    "the web overlay must keep images_modeled: false (raster images are not modeled on web)",
  );
  const browser = readSource(WEB_BROWSER);
  assert.deepEqual(
    missing(browser, ["push_web_overlay"]),
    [],
    `${WEB_BROWSER} must re-apply the web overlay`,
  );
  assert.ok(
    runsAfter(browser, "self.binding.flush_updates()", "self.push_web_overlay()"),
    `${WEB_BROWSER} must run the shared flush before re-applying the web overlay`,
  );
});

test("android host re-applies the android-specific overlay after the shared flush", () => {
  const overlay = readSource(ANDROID_OVERLAY);
  assert.deepEqual(
    missing(overlay, ["push_android_overlay"].concat(PLATFORM_OVERLAY_CHANNELS)),
    [],
    `${ANDROID_OVERLAY} is missing the android overlay entry point or a channel`,
  );
  assert.ok(
    overlay.includes("images_modeled: false"),
    "the android overlay must keep images_modeled: false (raster images are not modeled on android)",
  );
  const poll = readSource(ANDROID_POLL);
  assert.ok(
    runsAfter(poll, "ui.binding.flush_updates()", "state_push::push_android_overlay("),
    `${ANDROID_POLL} must run the shared flush before re-applying the android overlay`,
  );
});

test("platform overlays never override the shared authority channels", () => {
  for (const path of [WEB_OVERLAY, ANDROID_OVERLAY]) {
    assert.deepEqual(
      presentIn(readSource(path), AUTHORITY_SETTERS),
      [],
      `${path} must not set a shared authority channel; only the shared UI apply ` +
        `path owns layer/property/layout/diagnostics`,
    );
  }
});

// --- 4. Negative fixtures: the helpers reject real mutations, not just pass ---

test("negative fixtures: a dropped channel, a swapped order and an override are rejected", () => {
  // Dropping one channel from an otherwise valid projection is detected.
  const projection = readSource(SHARED_PROJECTION).replace(
    "set_overlays",
    "noop_overlays",
  );
  assert.deepEqual(missing(projection, ["set_overlays"]), ["set_overlays"]);

  // Swapping flush/overlay order is detected.
  const swapped =
    "self.push_web_overlay(); self.binding.flush_updates();";
  assert.equal(
    runsAfter(swapped, "self.binding.flush_updates()", "self.push_web_overlay()"),
    false,
  );

  // A platform overlay that grabs an authority setter is detected.
  const override = readSource(WEB_OVERLAY) + "\nhandle.set_layer_state();\n";
  assert.deepEqual(presentIn(override, AUTHORITY_SETTERS), ["set_layer_state"]);
});

// --- 5. Hermetic import guard ----------------------------------------------

test("the parity contract file is syntactically loadable source", () => {
  // Loadability is verified in the orchestrator's node run; here we assert the
  // module imports only the hermetic `node:` builtins and node:test, so the
  // contract can never grow a network or wasm dependency.
  const source = readSource("scripts/test-host-panel-parity.mjs");
  const imports = [...source.matchAll(/import\s+[^;]*?from\s+["']([^"']+)["']/g)].map(
    (m) => m[1],
  );
  assert.deepEqual(
    imports.sort(),
    ["node:assert/strict", "node:fs", "node:test"],
    "the parity contract must stay hermetic (node builtins + node:test only)",
  );
});
