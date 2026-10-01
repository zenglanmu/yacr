// Minimal browser host for the yacr wasm module (spec v2.0 §9.2).
//
// Responsibilities kept deliberately small: load wasm, wire the File API
// pickers to the Rust exports, surface the renderer state, and swallow the
// documented winit control-flow exception used to hand over the event loop.

import init, {
  start_web,
  open_document_bytes,
  renderer_state_report,
  annotation_import_json,
} from "./pkg/yacr.js";

const element = (id) => document.getElementById(id);

function setState(text) {
  const node = element("host-state");
  if (node) node.textContent = text;
}

async function readBytes(file) {
  return new Uint8Array(await file.arrayBuffer());
}

function wireFilePickers() {
  const drawingInput = element("file-input");
  drawingInput.addEventListener("change", async () => {
    const file = drawingInput.files && drawingInput.files[0];
    if (!file) return;
    try {
      open_document_bytes(file.name, await readBytes(file));
    } catch (error) {
      console.error("yacr: open failed", error);
      setState("打开失败：" + error);
    } finally {
      drawingInput.value = "";
    }
  });

  const annotationInput = element("annotation-input");
  annotationInput.addEventListener("change", async () => {
    const file = annotationInput.files && annotationInput.files[0];
    if (!file) return;
    try {
      const count = annotation_import_json(await file.text());
      setState(`已导入 ${count} 条批注`);
    } catch (error) {
      console.error("yacr: annotation import failed", error);
      setState("批注导入失败：" + error);
    } finally {
      annotationInput.value = "";
    }
  });
}

// winit's wasm event loop hands control to the browser by throwing; that is
// documented control flow, not a failure. Suppress only that message.
function ignoreWinitHandoff() {
  const isHandoff = (message) =>
    typeof message === "string" && message.includes("Using exceptions for control flow");
  window.addEventListener("error", (event) => {
    if (isHandoff(event.message)) {
      event.preventDefault();
      return false;
    }
    return undefined;
  });
  window.addEventListener("unhandledrejection", (event) => {
    if (isHandoff(event.reason && event.reason.message ? event.reason.message : String(event.reason))) {
      event.preventDefault();
    }
  });
}

function startStatePolling() {
  let ticks = 0;
  const timer = setInterval(() => {
    ticks += 1;
    try {
      const report = renderer_state_report();
      setState(report);
      window.yacrState = report;
      if (report.includes("backend=Some") && ticks > 2) {
        clearInterval(timer);
        // Keep updating on an interval for diagnostics without spinning.
        setInterval(() => {
          try {
            const latest = renderer_state_report();
            setState(latest);
            window.yacrState = latest;
          } catch (error) {
            /* host torn down */
          }
        }, 2000);
      }
    } catch (error) {
      setState("宿主尚未就绪");
    }
  }, 250);
}

async function main() {
  await init();
  setState("wasm 已加载，正在初始化渲染器…");
  wireFilePickers();
  ignoreWinitHandoff();

  // Expose for diagnostics and headless verification.
  window.yacr = {
    renderer_state_report,
    open_document_bytes: (name, bytes) => open_document_bytes(name, bytes),
  };

  startStatePolling();

  try {
    await start_web();
  } catch (error) {
    console.warn("yacr: event loop handoff", error);
  }
}

main().catch((error) => {
  console.error("yacr: startup failed", error);
  setState("启动失败：" + error);
});