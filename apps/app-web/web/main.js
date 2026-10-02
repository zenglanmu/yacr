// Browser composition root. Keep wasm loading dynamic so failures are localized.
import { createFileHost } from "./host/files.js";
import { installTouchNavigation } from "./host/touch.js";
import { createI18n } from "./host/i18n.js";
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
const { setStateKey } = i18n;
const showRecoveryBackend = wireRecoveryBackend(() => wasmModule, setStateKey);
let resizeViewport;

async function main() {
  forceSingleSampleCanvas();
  normalizeCanvasResizeObserver();
  await i18n.initialize();
  resizeViewport = installViewportSizing(() => wasmModule);
  setStateKey("host.loading_wasm");

  const loadedModule = await withDeadline(
    import("./pkg/yacr.js"),
    60000,
    "wasm module load",
  );
  await withDeadline(loadedModule.default(), 60000, "wasm initialization");
  wasmModule = loadedModule;
  setStateKey("host.wasm_loaded");

  const { exportAnnotations, wireFilePickers } = createFileHost(
    wasmModule,
    i18n,
  );
  wireFilePickers();
  installTouchNavigation(wasmModule);
  ignoreWinitHandoff();

  // Preserve the public diagnostics/test surface across the module split.
  window.yacr = {
    renderer_state_report: wasmModule.renderer_state_report,
    diagnostics_report: wasmModule.diagnostics_report_json,
    open_document_bytes: (name, bytes) =>
      wasmModule.open_document_bytes(name, bytes),
    open_requires_decision: wasmModule.open_requires_decision,
    open_document_decided: (name, bytes, decision) =>
      wasmModule.open_document_bytes_decided(name, bytes, decision),
    load_fonts: () => wasmModule.load_web_fonts(),
    font_load_report: wasmModule.font_load_report,
    export_annotations: exportAnnotations,
    set_locale: i18n.setLocale,
    current_locale: i18n.currentLocale,
    shell_geometry: wasmModule.shell_geometry,
    has_recovery_snapshot: wasmModule.has_recovery_snapshot,
    restore_recovery_snapshot: wasmModule.restore_recovery_snapshot,
    discard_recovery_snapshot: wasmModule.discard_recovery_snapshot,
  };

  startStatePolling(wasmModule, setStateKey, {
    onReady: () => {
      document.getElementById("open-drawing").disabled = false;
      resizeViewport();
    },
    onFailure: showRecoveryBackend,
  });
  if (wasmModule.has_recovery_snapshot()) setStateKey("host.recovery_pending");

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
