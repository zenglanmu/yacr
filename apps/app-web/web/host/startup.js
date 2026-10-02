// Bounded backend probing and presentation sizing. Never reload unsaved work.
export async function withDeadline(promise, milliseconds, label) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`${label}: timeout (${milliseconds}ms)`)),
          milliseconds,
        );
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

export async function chooseBackend({ probeTimeout = 8000 } = {}) {
  let requested = new URL(location.href).searchParams.get("backend");
  if (!requested) {
    try {
      requested = localStorage.getItem("yacr.cad.backend");
    } catch (_) {
      /* read-only storage */
    }
  }
  requested = (requested || "auto").toLowerCase();
  let chosen = "webgl2";
  let reason = null;
  if (requested !== "webgl2" && navigator.gpu) {
    try {
      // API presence and even an adapter alone do not prove device creation.
      await withDeadline(
        (async () => {
          const adapter = await navigator.gpu.requestAdapter();
          if (!adapter) throw new Error("WebGPU returned no adapter");
          const device = await adapter.requestDevice();
          device.destroy();
        })(),
        probeTimeout,
        "WebGPU probe",
      );
      chosen = "webgpu";
    } catch (error) {
      reason = String(error);
    }
  } else if (requested === "webgpu") {
    reason = "navigator.gpu is unavailable";
  }
  window.yacrBackendProbe = { requested, chosen, reason };
  return chosen;
}

export function installViewportSizing(getWasmModule) {
  const host = document.getElementById("canvas-host");
  const canvas = document.getElementById("canvas");
  const toolbar = document.getElementById("host-controls");
  let frame = null;
  const resize = () => {
    document.documentElement.style.setProperty(
      "--host-toolbar-height",
      `${toolbar.getBoundingClientRect().height}px`,
    );
    const { width, height } = host.getBoundingClientRect();
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    const scale = window.devicePixelRatio || 1;
    const wasm = getWasmModule();
    if (wasm?.web_resize) wasm.web_resize(width, height, scale);
  };
  const schedule = () => {
    if (frame !== null) cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      frame = null;
      resize();
    });
  };
  const observer = new ResizeObserver(schedule);
  observer.observe(host);
  observer.observe(toolbar);
  window.addEventListener("resize", schedule);
  window.visualViewport?.addEventListener("resize", schedule);
  resize();
  return schedule;
}

export function wireRecoveryBackend(getWasmModule, setStateKey) {
  const button = document.getElementById("retry-renderer");
  button.addEventListener("click", () => {
    if (getWasmModule()?.open_requires_decision?.()) {
      setStateKey("host.retry_unsaved");
      return;
    }
    const url = new URL(location.href);
    url.searchParams.set("backend", "webgl2");
    location.assign(url.href);
  });
  return () => {
    button.hidden = false;
  };
}
