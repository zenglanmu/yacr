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
    // The CSS `env(safe-area-inset-*)` values are reported separately so the
    // authoritative canvas metrics are the surface minus the safe area. A build
    // without the export keeps zero insets (never a guessed margin).
    if (wasm?.web_safe_insets) {
      const [top, right, bottom, left] = readSafeAreaInsets();
      wasm.web_safe_insets(top, right, bottom, left);
    }
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

/**
 * Measure the CSS `env(safe-area-inset-*)` values in logical pixels.
 *
 * A hidden probe resolves the `env()` expressions to computed px. Missing or
 * non-finite values become 0 — the host never invents a margin — and a DOM
 * without the probe API (node contract tests) reports all-zero.
 *
 * @returns {[number, number, number, number]} `[top, right, bottom, left]`.
 */
export function readSafeAreaInsets() {
  if (!document.body || typeof document.createElement !== "function") {
    return [0, 0, 0, 0];
  }
  const probe = document.createElement("div");
  probe.style.position = "fixed";
  probe.style.visibility = "hidden";
  probe.style.pointerEvents = "none";
  probe.style.paddingTop = "env(safe-area-inset-top)";
  probe.style.paddingRight = "env(safe-area-inset-right)";
  probe.style.paddingBottom = "env(safe-area-inset-bottom)";
  probe.style.paddingLeft = "env(safe-area-inset-left)";
  document.body.appendChild(probe);
  const style = getComputedStyle(probe);
  const px = (value) => {
    const n = Number.parseFloat(value);
    return Number.isFinite(n) && n >= 0 ? n : 0;
  };
  const insets = [
    px(style.paddingTop),
    px(style.paddingRight),
    px(style.paddingBottom),
    px(style.paddingLeft),
  ];
  probe.remove();
  return insets;
}

export function wireRecoveryBackend(getWasmModule, setStateKey) {
  const button = document.getElementById("retry-renderer");
  button.addEventListener("click", () => {
    const url = new URL(location.href);
    url.searchParams.set("backend", "webgl2");
    location.assign(url.href);
  });
  return () => {
    button.hidden = false;
  };
}
