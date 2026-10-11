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

// The authoritative shared panel-channel surface every host funnel must
// reference. All three hosts now push it through the shared `cad-host` core
// (`HostCore::push` in `crates/cad-host/src/state.rs`); a per-host overlay file
// owns only its genuinely host-specific channels and is asserted separately.
// Any per-host difference must be a real, code-backed difference and documented
// next to the host entry below.
const REQUIRED_PANEL_SYMBOLS = [
  "set_layer_state",
  "set_property_state",
  "set_layout_state",
  "set_diagnostics_state",
  "set_selection_highlight",
  "set_measurement_preview",
  "set_overlay_visibility",
];

// The `UiHandle` panel setters that are expected to exist in handle.rs. Kept as
// a separate list so a rename fails the source contract independently of the
// host-reference contract below.
const REQUIRED_HANDLE_SETTERS = [
  "set_layer_state",
  "set_property_state",
  "set_layout_state",
  "set_diagnostics_state",
];

// The android-specific overlay channels `apps/app-android/src/state_push.rs`
// owns after P2 (the shared panels come from the cad-host core).
const ANDROID_OVERLAY_SYMBOLS = ["set_import_state", "set_resources_sections"];

// Each host lists the file(s) that own its "application state → shell" leg and
// the entry points each file must reference. `gate` marks the file that owns the
// effective-config overlay gate.
const HOSTS = [
  {
    name: "linux",
    // app-linux is a thin wrapper: its panel funnel is the shared cad-host core.
    files: [
      {
        path: "crates/cad-host/src/state.rs",
        required: REQUIRED_PANEL_SYMBOLS,
        gate: true,
      },
    ],
  },
  {
    name: "web",
    // app-web is a thin wrapper too: its *shared* panel funnel is the cad-host
    // core (`HostCore::push`). Its browser-specific overlay (import progress +
    // font summary) is asserted separately below so this contract is not
    // tautological with the linux entry.
    files: [
      {
        path: "crates/cad-host/src/state.rs",
        required: REQUIRED_PANEL_SYMBOLS,
        gate: true,
      },
    ],
  },
  {
    name: "android",
    // app-android is a thin wrapper too (P2): the shared panels come from the
    // cad-host core exactly like linux/web; `state_push.rs` now owns only the
    // android-specific overlay (import progress + resources font summary).
    files: [
      {
        path: "crates/cad-host/src/state.rs",
        required: REQUIRED_PANEL_SYMBOLS,
        gate: true,
      },
      {
        path: "apps/app-android/src/state_push.rs",
        required: ANDROID_OVERLAY_SYMBOLS,
        gate: false,
      },
    ],
  },
];

// The overlay gate reads the *effective* configuration, so an overlay the
// config disables is never drawn. The shared core and Android use this exact
// expression; if a host adopts a variant (e.g. a cached accessor) update this
// list and the message here, do not weaken the assertion silently.
const OVERLAY_GATE = /set_overlay_visibility\s*\(\s*handle\.effective_config\(\)\.view\.overlays\.into\(\)\s*\)/;

for (const host of HOSTS) {
  for (const file of host.files) {
    test(`${host.name} host (${file.path}) references every required panel/overlay channel`, () => {
      const source = readSource(file.path);
      for (const symbol of file.required) {
        assert.ok(
          source.includes(symbol),
          `${file.path} is missing required panel/overlay entry point "${symbol}"; ` +
            `the ${host.name} host funnel has drifted from the shared panel surface`,
        );
      }
    });

    if (file.gate) {
      test(`${host.name} host (${file.path}) gates overlays on the effective config`, () => {
        const source = readSource(file.path);
        assert.match(
          source,
          OVERLAY_GATE,
          `${file.path} must call ` +
            `set_overlay_visibility(handle.effective_config().view.overlays.into()) ` +
            `so overlay gating parity is pinned across hosts`,
        );
      });
    }
  }
}

// The web host's *browser-specific* overlay (import progress + font summary) is
// not in the shared core. Assert it directly so the web parity contract is not
// tautological with the linux entry (both would otherwise point at the same
// cad-host file) and the web overlay cannot be silently dropped.
const WEB_OVERLAY_SYMBOLS = ["set_import_state", "set_resources_sections"];

test("web host overlay pushes the browser-specific import/font channels", () => {
  const overlay = readSource("apps/app-web/src/browser/state_push.rs");
  for (const symbol of WEB_OVERLAY_SYMBOLS) {
    assert.ok(
      overlay.includes(symbol),
      `apps/app-web/src/browser/state_push.rs is missing "${symbol}"; ` +
        `the browser-specific panel overlay has drifted`,
    );
  }
  assert.ok(
    overlay.includes("images_modeled: false"),
    "the web overlay must keep images_modeled: false (raster images are not modeled on web)",
  );
  const browser = readSource("apps/app-web/src/browser.rs");
  assert.ok(
    browser.includes("core.push()"),
    "apps/app-web/src/browser.rs must run the shared HostCore::push",
  );
  assert.ok(
    browser.includes("push_web_overlay"),
    "apps/app-web/src/browser.rs must re-apply the web overlay after the shared push",
  );
  assert.ok(
    browser.includes("set_layout_switch_sink"),
    "apps/app-web/src/browser.rs must install a layout-switch sink",
  );
  assert.ok(
    browser.includes("Box::new(layout_sink)"),
    "the web layout-switch sink must be the web sink (web funnel), not the raw HostCore",
  );
});

// The android host's *android-specific* overlay (import progress + font summary)
// is not in the shared core. Assert it directly so the android contract covers
// both halves: the shared panels come from `crates/cad-host/src/state.rs` (above)
// and the overlay file owns only its own channels.
test("android host overlay pushes the android-specific import/font channels", () => {
  const overlay = readSource("apps/app-android/src/state_push.rs");
  for (const symbol of ANDROID_OVERLAY_SYMBOLS) {
    assert.ok(
      overlay.includes(symbol),
      `apps/app-android/src/state_push.rs is missing "${symbol}"; ` +
        `the android-specific panel overlay has drifted`,
    );
  }
  assert.ok(
    overlay.includes("images_modeled: false"),
    "the android overlay must keep images_modeled: false (raster images are not modeled on android)",
  );
  assert.ok(
    overlay.includes("core.push()"),
    "the android overlay must run the shared HostCore::push before re-applying itself",
  );
  const lib = readSource("apps/app-android/src/lib.rs");
  assert.ok(
    lib.includes("push_android_state"),
    "apps/app-android/src/lib.rs must run the android state push",
  );
});

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
