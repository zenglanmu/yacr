// Browser/Slint compatibility setup; no application or document state.

// wgpu's GL present blit is invalid with a multisampled default framebuffer.
// Install this before Slint creates the context; only canvas-level MSAA is lost.
export function forceSingleSampleCanvas() {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function getContext(type, attrs) {
    if (type === "webgl2" || type === "webgl") {
      attrs = Object.assign({}, attrs, { antialias: false });
    }
    return original.call(this, type, attrs);
  };
}

// Some Chromium mobile/DPR emulation paths expose compositor pixels rather
// than device pixels in devicePixelContentBoxSize. winit consumes that box
// directly, so normalize only inconsistent canvas entries; native entries pass
// through unchanged. Other elements and observers retain their native values.
export function normalizeCanvasResizeObserver() {
  const NativeObserver = window.ResizeObserver;
  window.ResizeObserver = class extends NativeObserver {
    constructor(callback) {
      super((entries, observer) => {
        callback(
          entries.map((entry) => {
            if (entry.target.id !== "canvas") return entry;
            const box = entry.devicePixelContentBoxSize?.[0];
            const css = entry.contentBoxSize?.[0];
            const scale = window.devicePixelRatio || 1;
            if (
              !box ||
              !css ||
              scale === 1 ||
              Math.abs(box.inlineSize - css.inlineSize * scale) <= 1
            )
              return entry;
            return new Proxy(entry, {
              get(target, key) {
                if (key === "devicePixelContentBoxSize")
                  return [
                    {
                      inlineSize: Math.round(css.inlineSize * scale),
                      blockSize: Math.round(css.blockSize * scale),
                    },
                  ];
                return Reflect.get(target, key, target);
              },
            });
          }),
          observer,
        );
      });
    }
  };
}

function isHandoffMessage(message) {
  return (
    typeof message === "string" &&
    message.includes("Using exceptions for control flow")
  );
}

/// Only winit's documented exception is ignored, never another startup error.
export function isHandoffError(error) {
  if (isHandoffMessage(error)) return true;
  return isHandoffMessage(
    error && error.message ? error.message : String(error),
  );
}

export function ignoreWinitHandoff() {
  window.addEventListener("error", (event) => {
    if (isHandoffMessage(event.message)) {
      event.preventDefault();
      return false;
    }
    return undefined;
  });
  window.addEventListener("unhandledrejection", (event) => {
    if (isHandoffError(event.reason)) event.preventDefault();
  });
}
