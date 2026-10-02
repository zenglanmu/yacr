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
