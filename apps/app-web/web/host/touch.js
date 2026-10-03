// Native multi-touch only inside the CAD rectangle. Slint keeps control input.
//
// Finger-count changes re-establish the gesture baseline. A count change never
// emits a navigation delta of its own: the first sample after the change becomes
// the new baseline, and only the move *after* that is compared against it. This
// is what stops a second finger landing (whose midpoint is far from the previous
// one-finger sample) from jumping the pan/pinch by the centroid shift.
export function sampleTouches(touches) {
  if (!touches.length) return null;
  const points = Array.from(touches).slice(0, 2);
  const x = points.reduce((n, p) => n + p.clientX, 0) / points.length;
  const y = points.reduce((n, p) => n + p.clientY, 0) / points.length;
  const distance =
    points.length === 2
      ? Math.hypot(
          points[0].clientX - points[1].clientX,
          points[0].clientY - points[1].clientY,
        )
      : 0;
  return { x, y, distance, count: points.length };
}

export function installTouchNavigation(wasm) {
  const canvas = document.getElementById("canvas");
  let active = false;
  // Last baseline sample and the finger count it was taken at. The pair is
  // updated together so a count change can never be compared to a sample of a
  // different count.
  let previous = null;
  let previousCount = 0;
  let origin = null;
  let moved = false;
  const cadPointers = new Set();

  function inside(x, y) {
    try {
      const [cx, cy, width, height] = wasm.shell_geometry();
      const rect = canvas.getBoundingClientRect();
      if (typeof wasm.canvas_hit_test === "function") {
        return wasm.canvas_hit_test(x - rect.left, y - rect.top);
      }
      return (
        x >= rect.left + cx &&
        x < rect.left + cx + width &&
        y >= rect.top + cy &&
        y < rect.top + cy + height
      );
    } catch {
      return false;
    }
  }

  // Re-establish the baseline from a fresh touch sample. Emits nothing: the
  // caller decides whether the change may navigate.
  const rebase = (touches) => {
    previous = sampleTouches(touches);
    previousCount = previous ? previous.count : 0;
  };

  const consume = (event) => {
    event.preventDefault();
    event.stopImmediatePropagation();
  };

  // winit uses PointerEvents on some browsers. Stop only CAD touch events so
  // it cannot interpret the same pinch as two independent pan/selection inputs.
  for (const type of [
    "pointerdown",
    "pointermove",
    "pointerup",
    "pointercancel",
  ]) {
    canvas.addEventListener(
      type,
      (event) => {
        if (event.pointerType !== "touch") return;
        if (type === "pointerdown" && inside(event.clientX, event.clientY))
          cadPointers.add(event.pointerId);
        if (cadPointers.has(event.pointerId)) consume(event);
        if (type === "pointerup" || type === "pointercancel")
          cadPointers.delete(event.pointerId);
      },
      { capture: true, passive: false },
    );
  }

  canvas.addEventListener(
    "touchstart",
    (event) => {
      if (
        !active &&
        !inside(event.touches[0].clientX, event.touches[0].clientY)
      )
        return;
      if (!active) {
        origin = sampleTouches(event.touches);
        moved = false;
      }
      if (event.touches.length > 1) {
        moved = true;
        wasm.touch_cancel_draw();
      }
      active = true;
      // Only a finger-count change re-establishes the baseline. A touchstart
      // that does not change the (capped) count — a third finger, or a retouch
      // of an existing one — must not move the baseline and jump the gesture.
      const next = sampleTouches(event.touches);
      if (!previous || !next || next.count !== previousCount) rebase(event.touches);
      consume(event);
    },
    { capture: true, passive: false },
  );

  canvas.addEventListener(
    "touchmove",
    (event) => {
      if (!active) return;
      consume(event);
      const next = sampleTouches(event.touches);
      if (
        origin &&
        next &&
        Math.hypot(next.x - origin.x, next.y - origin.y) > 8
      )
        moved = true;
      // Navigate only between samples of the same finger count. When the count
      // differs (a finger landed off-canvas, or a start/end was not delivered to
      // this target), consume the sample as the new baseline without a delta so
      // the centroid shift caused by the count change is never navigation.
      if (moved && next && previous && next.count === previousCount) {
        const zoom =
          previous.distance > 0 && next.distance > 0
            ? next.distance / previous.distance
            : 1;
        try {
          wasm.touch_navigate(
            next.x - previous.x,
            next.y - previous.y,
            Math.min(5, Math.max(0.2, zoom)),
          );
        } catch (error) {
          console.warn("CAD touch navigation:", error);
        }
      }
      previous = next;
      previousCount = next ? next.count : 0;
    },
    { capture: true, passive: false },
  );

  for (const type of ["touchend", "touchcancel"]) {
    canvas.addEventListener(
      type,
      (event) => {
        if (!active) return;
        consume(event);
        if (
          type === "touchend" &&
          event.touches.length === 0 &&
          !moved &&
          origin
        ) {
          const rect = canvas.getBoundingClientRect();
          const [cx, cy] = wasm.shell_geometry();
          wasm.touch_pick(origin.x - rect.left - cx, origin.y - rect.top - cy);
        }
        if (type === "touchcancel") {
          wasm.touch_cancel_draw();
          previous = null;
          previousCount = 0;
        } else {
          rebase(event.touches);
        }
        active = previous !== null;
      },
      { capture: true, passive: false },
    );
  }
  return () => {
    active = false;
    previous = null;
    previousCount = 0;
  };
}
