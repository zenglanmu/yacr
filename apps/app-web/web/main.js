// Browser composition root. Keep wasm loading dynamic so failures are localized.
import { createFileHost } from "./host/files.js";
import { createI18n } from "./host/i18n.js";
import { startStatePolling } from "./host/renderer.js";
import {
  forceSingleSampleCanvas,
  ignoreWinitHandoff,
  isHandoffError,
} from "./host/runtime.js";

let wasmModule = null;
const i18n = createI18n(() => wasmModule);
const { setStateKey } = i18n;

async function main() {
  forceSingleSampleCanvas();
  await i18n.initialize();
  setStateKey("host.loading_wasm");

  wasmModule = await import("./pkg/yacr.js");
  await wasmModule.default();
  setStateKey("host.wasm_loaded");

  const { exportAnnotations, wireFilePickers } = createFileHost(
    wasmModule,
    i18n,
  );
  wireFilePickers();
  ignoreWinitHandoff();

  // Preserve the public diagnostics/test surface across the module split.
  window.yacr = {
    renderer_state_report: wasmModule.renderer_state_report,
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
    has_recovery_snapshot: wasmModule.has_recovery_snapshot,
    restore_recovery_snapshot: wasmModule.restore_recovery_snapshot,
    discard_recovery_snapshot: wasmModule.discard_recovery_snapshot,
  };

  startStatePolling(wasmModule, setStateKey);
  if (wasmModule.has_recovery_snapshot()) setStateKey("host.recovery_pending");

  try {
    await wasmModule.start_web();
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
}

main().catch(reportStartupError);
