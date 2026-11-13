// DOM pinch signals → one incremental CAD zoom multiplier.
//
// The native session consumes `InputKind::Pinch` through the single
// `probe_input` funnel, so everything this module does is turn the two DOM
// shapes Windows/macOS actually deliver into that record's `scale`:
//
// - WebView2/Chromium: a trackpad pinch arrives as `wheel` with `ctrlKey`.
// - WKWebView/Safari: the WebKit `gesturestart`/`gesturechange`/`gestureend`
//   sequence carries a cumulative `scale`.
//
// Keeping the arithmetic here (no DOM, no listeners) makes the contract
// unit-testable, exactly like `metrics.ts`.

/**
 * Zoom-per-CSS-pixel shared with the host's wheel zoom (`1 + dy * 0.0015`, see
 * `cad-host/src/input/interaction.rs`): a trackpad pinch and a wheel notch
 * change the camera scale by the same magnitude.
 */
export const PINCH_SCALE_PER_PIXEL = 0.0015;

/**
 * The incremental zoom multiplier for one `ctrl`+`wheel` pinch sample.
 *
 * Exponential rather than linear so successive samples compose exactly the way
 * the lane composes two queued pinch records (`tail.scale * input.scale`):
 * `exp(-a) * exp(-b) === exp(-(a + b))`. It is also strictly positive for any
 * finite delta, so a scale can never become `0` or negative. A non-finite delta
 * reports the honest "no zoom" (`1`) instead of pushing `NaN` into the lane —
 * the host would read that as a broken gesture.
 */
export function wheelPinchScale(deltaY: number): number {
  if (!Number.isFinite(deltaY)) return 1;
  return Math.exp(-deltaY * PINCH_SCALE_PER_PIXEL);
}

/**
 * The WebKit non-standard gesture event, narrowed to what the binding reads.
 *
 * TypeScript's DOM lib has no `GestureEvent`, so the listener casts an `Event`
 * to this shape once at the boundary; it is never produced by our code.
 */
export interface WebKitGestureEvent {
  readonly type: string;
  readonly clientX: number;
  readonly clientY: number;
  /** Cumulative gesture scale (`1` at `gesturestart`). */
  readonly scale: number;
}

/**
 * Converts a WebKit gesture's cumulative `scale` into the **incremental** ratio
 * the wire expects.
 *
 * The host applies each `pinch` sample as a multiplier about the midpoint, so
 * reporting the cumulative value twice would zoom twice: `change(2.25)` after
 * `change(1.5)` must report `1.5`, not `2.25`. `start` records the baseline;
 * a gesture whose `start` never arrived still works (baseline `1`) and never
 * divides by zero, because an invalid or non-positive sample is refused without
 * moving the baseline.
 */
export class GesturePinch {
  #previous = 1;

  /** Record the gesture's starting scale; invalid input falls back to `1`. */
  start(scale: number): void {
    this.#previous = Number.isFinite(scale) && scale > 0 ? scale : 1;
  }

  /** The ratio since the last sample, or `null` when there is nothing to send. */
  change(scale: number): number | null {
    if (!Number.isFinite(scale) || scale <= 0) return null;
    const previous = this.#previous;
    this.#previous = scale;
    const ratio = scale / previous;
    // `previous` is always finite and positive (constructor `1`, `start`
    // guarded, `change` only stores a guarded value), so this cannot divide by
    // zero; a `1` ratio means the sample carries no zoom and is not a record.
    return ratio === 1 ? null : ratio;
  }

  /** Forget the baseline; the next gesture starts from `1` again. */
  end(): void {
    this.#previous = 1;
  }
}
