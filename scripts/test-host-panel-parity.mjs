// Mobile-shell panel wiring parity across the Linux, Web and Android hosts.
//
// This is a *source-text* contract, not a device or browser test: it reads the
// three host modules that own the "application state → shell" leg and asserts
// each one references the full set of derived panel/overlay entry points plus
// the effective-config overlay gate. Drift between hosts then fails CI without a
// device. Rendering correctness is out of scope; see docs/panels.md §3.2.
//
// Run: node --test scripts/test-host-panel-parity.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const ROOT = new URL("../", import.meta.url);

function readSource(relative) {
  return readFileSync(new URL(relative, ROOT), "utf8");
}

// The authoritative panel-channel surface every host funnel must reference. All
// three hosts use the exact same set: the Android doc comment says it "mirrors
// the same getters the web host uses so the two hosts cannot drift", and the
// Linux host pushes the same family. Any per-host difference must be a real,
// code-backed difference and documented next to the host entry below.
const REQUIRED_PANEL_SYMBOLS = [
  "set_layer_state",
  "set_property_state",
  "set_annotation_state",
  "set_layout_state",
  "set_diagnostics_state",
  "set_selection_highlight",
  "set_measurement_preview",
  "set_annotation_preview",
  "set_overlay_visibility",
];

// The `UiHandle` panel setters that are expected to exist in handle.rs. Kept as
// a separate list so a rename fails the source contract independently of the
// host-reference contract below.
const REQUIRED_HANDLE_SETTERS = [
  "set_layer_state",
  "set_property_state",
  "set_annotation_state",
  "set_layout_state",
  "set_diagnostics_state",
];

const HOSTS = [
  {
    name: "linux",
    file: "apps/app-linux/src/host/state.rs",
    required: REQUIRED_PANEL_SYMBOLS,
  },
  {
    name: "web",
    file: "apps/app-web/src/browser/state_push.rs",
    required: REQUIRED_PANEL_SYMBOLS,
  },
  {
    name: "android",
    file: "apps/app-android/src/state_push.rs",
    required: REQUIRED_PANEL_SYMBOLS,
  },
];

// The overlay gate reads the *effective* configuration, so an overlay the
// config disables is never drawn. All three hosts use this exact expression; if
// a host adopts a variant (e.g. a cached accessor) update this list and the
// message here, do not weaken the assertion silently.
const OVERLAY_GATE = /set_overlay_visibility\s*\(\s*handle\.effective_config\(\)\.view\.overlays\.into\(\)\s*\)/;

for (const host of HOSTS) {
  test(`${host.name} host references every required panel/overlay channel`, () => {
    const source = readSource(host.file);
    for (const symbol of host.required) {
      assert.ok(
        source.includes(symbol),
        `${host.file} is missing required panel/overlay entry point "${symbol}"; ` +
          `the ${host.name} host funnel has drifted from the shared panel surface`,
      );
    }
  });

  test(`${host.name} host gates overlays on the effective config`, () => {
    const source = readSource(host.file);
    assert.match(
      source,
      OVERLAY_GATE,
      `${host.file} must call ` +
        `set_overlay_visibility(handle.effective_config().view.overlays.into()) ` +
        `so overlay gating parity is pinned across hosts`,
    );
  });
}

test("required panel channels are real UiHandle public setters", () => {
  const handle = readSource("crates/cad-ui-slint/src/handle.rs");
  // Parse `pub fn set_<name>` declarations. The handle also exposes non-panel
  // setters (config, locale, status, …); the required panel set must be a
  // subset, never the whole set.
  const declared = new Set(
    [...handle.matchAll(/pub\s+fn\s+(set_[A-Za-z0-9_]+)/g)].map((m) => m[1]),
  );
  for (const setter of REQUIRED_HANDLE_SETTERS) {
    assert.ok(
      declared.has(setter),
      `crates/cad-ui-slint/src/handle.rs no longer declares UiHandle::${setter}; ` +
        `a renamed or removed panel setter breaks every host`,
    );
  }
  // A source-only sanity check: the handle really exposed a broader set, so the
  // subset assertion above is meaningful rather than accidentally total.
  assert.ok(
    declared.size > REQUIRED_HANDLE_SETTERS.length,
    "expected UiHandle to expose more set_* methods than the required panel subset",
  );
});

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
