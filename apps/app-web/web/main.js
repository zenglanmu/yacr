// Browser host for the yacr wasm module (spec v2.0 §9.2, N01 host sync).
//
// Responsibilities kept deliberately small: localize the HTML/JS chrome from the
// shared catalog, load wasm, wire the File API pickers to the Rust exports,
// surface the renderer state, and catch the documented winit control-flow
// exception used to hand the event loop to the browser.
//
// The wasm module is imported dynamically so a module/initialisation failure is
// a real, localizable error instead of an uncatchable static-import abort.

const LOCALE_STORAGE_KEY = "yacr.cad.locale";
const DEFAULT_LOCALE = "zh-CN";

let wasmModule = null;
let catalog = {};
let currentLocale = DEFAULT_LOCALE;
// Last host status as a catalog key + args, so a language switch can re-render
// it. Raw strings pushed by the Rust host are shown verbatim.
let lastState = null;

const element = (id) => document.getElementById(id);

// wgpu's WebGL2 present path blits from its surface framebuffer to the canvas
// default framebuffer. A canvas context created with the default
// `antialias: true` has a multisampled default framebuffer, and that blit is an
// `INVALID_OPERATION` in WebGL2 — the canvas stays blank. Force antialias off
// before Slint creates the context; the CAD frame is a texture composite, so the
// shell only loses canvas-level MSAA.
function forceSingleSampleCanvas() {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function getContext(type, attrs) {
    if (type === "webgl2" || type === "webgl") {
      attrs = Object.assign({}, attrs, { antialias: false });
    }
    return original.call(this, type, attrs);
  };
}

// --- i18n -------------------------------------------------------------------

/// Map any accepted tag (`en-US`, `zh-Hans`) onto a shipped stable catalog.
function normalizeLocale(tag) {
  const primary = String(tag || "")
    .trim()
    .toLowerCase()
    .split(/[-_]/)[0];
  if (primary === "en") return "en";
  if (primary === "zh") return DEFAULT_LOCALE;
  return null;
}

function storedLocale() {
  try {
    return normalizeLocale(localStorage.getItem(LOCALE_STORAGE_KEY));
  } catch (error) {
    return null;
  }
}

/// Look up a catalog key, substituting `{name}` placeholders. A missing key
/// renders as `⟦key⟧` (never an empty or invented string), matching the Rust
/// `Message::Missing` policy so omissions are visible.
function t(key, args = {}) {
  const template = catalog[key];
  if (typeof template !== "string") return `\u27e6${key}\u27e7`;
  return template.replace(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (match, name) =>
    Object.prototype.hasOwnProperty.call(args, name) ? String(args[name]) : match,
  );
}

/// Fetch the canonical catalog that `build-web.sh` copied out of
/// `crates/cad-ui-slint/i18n/`. This is the same JSON the Rust `MessageSource`
/// embeds, so host and UI strings cannot drift.
async function loadCatalog(tag) {
  const normalized = normalizeLocale(tag) || DEFAULT_LOCALE;
  if (normalized === currentLocale && Object.keys(catalog).length > 0) {
    return normalized;
  }
  const response = await fetch(`i18n/${normalized}.json`, { cache: "no-store" });
  if (!response.ok) throw new Error(`catalog ${normalized}: HTTP ${response.status}`);
  const parsed = await response.json();
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(`catalog ${normalized}: not a JSON object`);
  }
  catalog = parsed;
  currentLocale = normalized;
  return normalized;
}

/// Apply the active language to the HTML chrome (N01 §6): `lang`, `title`,
/// `noscript` and the language selector.
function applyDocumentLocale() {
  document.documentElement.lang = currentLocale;
  if (typeof catalog["app.title"] === "string") {
    document.title = catalog["app.title"];
  }
  const noscript = document.querySelector("noscript");
  if (noscript) noscript.textContent = t("host.noscript");
  const label = element("language-label");
  if (label) label.textContent = t("host.language_label");
  const select = element("language");
  if (select) select.value = currentLocale;
}

function setStateText(text) {
  lastState = null;
  const node = element("host-state");
  if (node) node.textContent = text;
}

function setStateKey(key, args = {}) {
  lastState = { key, args };
  const node = element("host-state");
  if (node) node.textContent = t(key, args);
}

function rerenderState() {
  if (!lastState) return;
  const node = element("host-state");
  if (node) node.textContent = t(lastState.key, lastState.args);
}

// --- wasm host --------------------------------------------------------------

/// winit's wasm event loop hands control to the browser by throwing; that is
/// documented control flow, not a failure. Only this exact handoff is ignored;
/// every other rejection is a real startup failure.
function isHandoffMessage(message) {
  return (
    typeof message === "string" &&
    message.includes("Using exceptions for control flow")
  );
}

function isHandoffError(error) {
  if (isHandoffMessage(error)) return true;
  return isHandoffMessage(error && error.message ? error.message : String(error));
}

function ignoreWinitHandoff() {
  window.addEventListener("error", (event) => {
    if (isHandoffMessage(event.message)) {
      event.preventDefault();
      return false;
    }
    return undefined;
  });
  window.addEventListener("unhandledrejection", (event) => {
    if (isHandoffError(event.reason)) {
      event.preventDefault();
    }
  });
}

/// Ask the user what to do with unsaved annotations. Returns one of
/// save/recovery/discard/cancel, or null when the host cannot ask. There is no
/// silent default: a null result must keep the current document.
function promptUnsavedDecision() {
  const choice = window.prompt(t("host.open_needs_decision"), "cancel");
  if (choice === null) return null;
  const normalized = choice.trim().toLowerCase();
  return ["save", "recovery", "preserve", "discard", "cancel"].includes(normalized)
    ? normalized
    : null;
}

/// Open a drawing through the explicit unsaved-decision flow (audit B05/B07).
function openDrawing(file) {
  return file.arrayBuffer().then((buffer) => {
    const bytes = new Uint8Array(buffer);
    let decision = "discard";
    if (wasmModule.open_requires_decision()) {
      decision = promptUnsavedDecision();
      if (!decision) {
        setStateKey("host.cancelled_open");
        return;
      }
    }
    try {
      setStateText(wasmModule.open_document_bytes_decided(file.name, bytes, decision));
    } catch (error) {
      console.error("yacr: open failed", error);
      setStateKey("host.open_failed", { error: String(error) });
    }
  });
}

/// Trigger a download of the exported annotation JSON and only confirm the
/// export to the core host once the download has actually started (audit B07).
function exportAnnotations() {
  let bundle;
  try {
    bundle = wasmModule.annotation_export_json();
  } catch (error) {
    setStateKey("host.export_failed", { error: String(error) });
    return;
  }
  const blob = new Blob([bundle.json], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "annotations.cadnotes.json";
  anchor.click();
  URL.revokeObjectURL(url);
  try {
    wasmModule.annotation_confirm_export(bundle.revision);
    setStateKey("host.exported_bytes", { bytes: bundle.json.length });
  } catch (error) {
    setStateKey("host.export_confirm_failed", { error: String(error) });
  }
}

function wireFilePickers() {
  const drawingInput = element("file-input");
  drawingInput.addEventListener("change", async () => {
    const file = drawingInput.files && drawingInput.files[0];
    if (!file) return;
    try {
      await openDrawing(file);
    } catch (error) {
      console.error("yacr: open failed", error);
      setStateKey("host.open_failed", { error: String(error) });
    } finally {
      drawingInput.value = "";
    }
  });

  const annotationInput = element("annotation-input");
  annotationInput.addEventListener("change", async () => {
    const file = annotationInput.files && annotationInput.files[0];
    if (!file) return;
    try {
      const count = wasmModule.annotation_import_json(await file.text());
      setStateKey("host.imported_count", { count });
    } catch (error) {
      console.error("yacr: annotation import failed", error);
      setStateKey("host.import_failed", { error: String(error) });
    } finally {
      annotationInput.value = "";
    }
  });
}

// --- renderer state polling (audit B29) -------------------------------------

let pollTimer = null;
let pollDelay = 400;
let rendererReady = false;

function stopPolling() {
  if (pollTimer !== null) {
    clearTimeout(pollTimer);
    pollTimer = null;
  }
}

function schedulePolling() {
  stopPolling();
  if (document.hidden) return;
  pollTimer = setTimeout(poll, pollDelay);
}

/// One poll: report the adapter field (not `backend`), back off until ready,
/// then keep a slow heartbeat for diagnostics. Stops entirely while hidden.
function poll() {
  pollTimer = null;
  if (document.hidden) return;
  try {
    const report = wasmModule.renderer_state_report();
    window.yacrState = report;
    const ready = /adapter=Some\((\w+)\)/.exec(report);
    if (ready) {
      // Announce readiness once; later heartbeats must not overwrite a user
      // message (open/export/import) with a repeated "ready" line.
      if (!rendererReady) {
        rendererReady = true;
        setStateKey("host.renderer_ready", { backend: ready[1] });
      }
      pollDelay = 2000;
    } else {
      pollDelay = Math.min(Math.round(pollDelay * 1.5), 2000);
    }
  } catch (error) {
    // The host is installed before the event loop runs; keep waiting without
    // spamming a 250ms fixed interval.
    if (!rendererReady) setStateKey("host.poll_not_ready");
    pollDelay = Math.min(Math.round(pollDelay * 1.5), 2000);
  }
  schedulePolling();
}

function startStatePolling() {
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      stopPolling();
    } else {
      pollDelay = 400;
      schedulePolling();
    }
  });
  schedulePolling();
}

// --- language switch (N01) --------------------------------------------------

/// Persist the choice, localize the DOM, and re-apply the catalog to the Slint
/// shell. Only chrome changes: the document, camera, annotations and undo
/// history are untouched.
async function setLocale(requested) {
  const normalized = await loadCatalog(requested);
  currentLocale = normalized;
  try {
    localStorage.setItem(LOCALE_STORAGE_KEY, normalized);
  } catch (error) {
    // A read-only storage does not prevent the in-memory switch.
  }
  if (wasmModule && typeof wasmModule.web_set_locale === "function") {
    const resolved = await wasmModule.web_set_locale(normalized);
    currentLocale = normalizeLocale(resolved) || normalized;
  }
  applyDocumentLocale();
  rerenderState();
  return currentLocale;
}

function wireLanguageSelector() {
  const select = element("language");
  if (!select) return;
  select.addEventListener("change", async () => {
    try {
      await setLocale(select.value);
    } catch (error) {
      console.error("yacr: language switch failed", error);
      setStateKey("host.locale_failed", { error: String(error) });
    }
  });
}

// --- startup ----------------------------------------------------------------

async function main() {
  // Must run before the wasm module creates Slint's WebGL2 context.
  forceSingleSampleCanvas();

  // Localize the host chrome before wasm loads so startup failures are readable.
  try {
    currentLocale = await loadCatalog(storedLocale() || DEFAULT_LOCALE);
  } catch (error) {
    console.error("yacr: catalog load failed", error);
  }
  wireLanguageSelector();
  applyDocumentLocale();
  setStateKey("host.loading_wasm");

  wasmModule = await import("./pkg/yacr.js");
  await wasmModule.default();
  setStateKey("host.wasm_loaded");

  wireFilePickers();
  ignoreWinitHandoff();

  // Expose for diagnostics and headless verification.
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
    set_locale: (tag) => setLocale(tag),
    current_locale: () => currentLocale,
    has_recovery_snapshot: wasmModule.has_recovery_snapshot,
    restore_recovery_snapshot: wasmModule.restore_recovery_snapshot,
    discard_recovery_snapshot: wasmModule.discard_recovery_snapshot,
  };

  startStatePolling();

  if (wasmModule.has_recovery_snapshot()) {
    setStateKey("host.recovery_pending");
  }

  try {
    await wasmModule.start_web();
    // A resolved promise means the event loop was handed off or ended cleanly.
    window.yacrHandoff = true;
  } catch (error) {
    if (isHandoffError(error)) {
      window.yacrHandoff = true;
      return;
    }
    window.yacrStartupError = String(error);
    console.error("yacr: startup failed", error);
    setStateKey("host.startup_failed", { error: String(error) });
  }
}

main().catch((error) => {
  console.error("yacr: startup failed", error);
  window.yacrStartupError = String(error);
  setStateKey("host.startup_failed", { error: String(error) });
});
