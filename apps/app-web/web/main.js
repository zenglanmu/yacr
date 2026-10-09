// Browser composition root. Keep wasm loading dynamic so failures are localized.
import { createFileHost } from "./host/files.js";
import { installTouchNavigation } from "./host/touch.js";
import { createA11y } from "./host/a11y.js";
import { createI18n } from "./host/i18n.js";
import { createConfigHost } from "./host/config.js";
import { startStatePolling } from "./host/renderer.js";
import {
  chooseBackend,
  installViewportSizing,
  wireRecoveryBackend,
  withDeadline,
} from "./host/startup.js";
import {
  forceSingleSampleCanvas,
  normalizeCanvasResizeObserver,
  ignoreWinitHandoff,
  isHandoffError,
} from "./host/runtime.js";

let wasmModule = null;
const i18n = createI18n(() => wasmModule);
// Single announcement funnel (audit U12): every localized host status is
// mirrored into the visually-hidden live region, so `#host-state` can stay
// `aria-live="off"` without losing screen-reader feedback.
const a11y = createA11y(i18n);
i18n.attachA11y(a11y);
const { setStateKey } = i18n;
const showRecoveryBackend = wireRecoveryBackend(() => wasmModule, setStateKey);
let resizeViewport;

// Redeploys change the wasm-bindgen glue/wasm import set. Pairing `pkg/yacr.js`
// and `pkg/yacr_bg.wasm` under one build stamp keeps a stale cached glue from
// being linked with a newer wasm (the "import object field ... is not a
// Function" LinkError). `scripts/build-web.sh` injects the stamp into index.html.
const buildStamp =
  (document.querySelector('meta[name="yacr-build"]') || {}).content || "dev";

async function main() {
  forceSingleSampleCanvas();
  normalizeCanvasResizeObserver();
  await i18n.initialize();
  resizeViewport = installViewportSizing(() => wasmModule);
  setStateKey("host.loading_wasm");

  const loadedModule = await withDeadline(
    import(`./pkg/yacr.js?v=${buildStamp}`),
    60000,
    "wasm module load",
  );
  await withDeadline(
    loadedModule.default(
      new URL(`./pkg/yacr_bg.wasm?v=${buildStamp}`, import.meta.url),
    ),
    60000,
    "wasm initialization",
  );
  wasmModule = loadedModule;
  setStateKey("host.wasm_loaded");

  const { wireFilePickers } = createFileHost(wasmModule, i18n);
  wireFilePickers();
  installTouchNavigation(wasmModule);
  ignoreWinitHandoff();

  const configHost = createConfigHost(() => wasmModule);

  // Preserve the public diagnostics/test surface across the module split.
  window.yacr = {
    renderer_state_report: wasmModule.renderer_state_report,
    diagnostics_report: wasmModule.diagnostics_report_json,
    open_document_bytes: (name, bytes) =>
      wasmModule.open_document_bytes(name, bytes),
    load_fonts: () => wasmModule.load_web_fonts(),
    font_load_report: wasmModule.font_load_report,
    set_locale: i18n.setLocale,
    current_locale: i18n.currentLocale,
    // ViewerConfig host API: set/update/query and host-allowed user preferences.
    setConfig: configHost.setConfig,
    updateConfig: configHost.updateConfig,
    applyUserPreference: configHost.applyUserPreference,
    clearUserPreference: configHost.clearUserPreference,
    config: configHost.config,
    shell_geometry: wasmModule.shell_geometry,
    canvas_hit_test: wasmModule.canvas_hit_test,
    // Async open (F01): the heartbeat pushes the panel; these expose the raw
    // export and the honest worker capability for diagnostics/headless checks.
    async_open_poll: wasmModule.async_open_poll_json,
    async_open_worker_available: wasmModule.async_open_worker_available,
  };

  startStatePolling(wasmModule, setStateKey, {
    onReady: () => {
      document.getElementById("open-drawing").disabled = false;
      resizeViewport();
    },
    onFailure: showRecoveryBackend,
  });

  try {
    const backend = await chooseBackend();
    // winit owns the event loop: this promise may intentionally never settle.
    // Readiness/failure deadlines belong to renderer polling, not this promise.
    await wasmModule.start_web_with_backend(backend);
    window.yacrHandoff = true;
  } catch (error) {
    if (isHandoffError(error)) {
      window.yacrHandoff = true;
      return;
    }
    reportStartupError(error);
  }
}

function reportStartupError(error) {
  console.error("yacr: startup failed", error);
  window.yacrStartupError = String(error);
  setStateKey("host.startup_failed", { error: String(error) });
  showRecoveryBackend();
}

main().catch(reportStartupError);
