//! Browser host: the **wasm CAD session + WebGL2 canvas** of the `dom-canvas`
//! runtime path (ADR 0024, `surfaceMode: dom-canvas`).
//!
//! This crate owns one CAD session **inside wasm**: it imports a drawing, builds
//! the display scene, renders it with the renderer's offscreen frame texture and
//! presents that frame on a DOM `<canvas>` through
//! [`cad_render_wgpu::canvas::CanvasPresenter`]. It is not a second rendering
//! mechanism next to the native presenter — it is the pixel carrier for hosts
//! that have no native surface to draw on (the browser, and later the Linux
//! X11/XWayland `dom-canvas` path). The WebView/native shell still owns window
//! lifetime, i18n and the admission policy; it must **not** hold a second CAD
//! session or a copy of the document (ADR 0024 D2).
//!
//! # Exported API (the contract the X11 host lane is wired against)
//!
//! ```text
//! start(canvas_id, css_w, css_h, dpr) -> JSON status string   (async)
//! resize(css_w, css_h, dpr)           -> JSON status string
//! zoom_at(factor, x, y)               -> JSON status string   (x/y: CSS px, canvas-local)
//! pan(dx, dy)                         -> JSON status string   (CSS px)
//! status()                            -> JSON status string
//! ```
//!
//! Every call returns the JSON status object on success and `Err(JsValue)` with a
//! message naming the failing step on failure; the last failure is also kept so
//! [`status()`] reports it. A status carries at least
//! `{ ok, backend, adapter_name, format, physical: [w, h], batches, draws, presented }`,
//! plus `step`/`error` (and, for a `step` of `webgl2`, the stable `marker`
//! `yacr: unsupported-runtime: webgl2`) on failure.
//!
//! # Honest limits (2026-10-24, this lane)
//!
//! - **Synthetic fixture only.** The bytes are `include_bytes!` of
//!   `fixtures/dwg/synthetic-four-lines.dwg` (authorized, `fixtures/manifest`);
//!   wasm has no filesystem, and opening a user DWG in wasm is a later phase
//!   (ADR 0024 stage ⑧), not this one.
//! - **WebGL2 only.** Neither target runtime exposes `navigator.gpu`, so the
//!   presenter requests `Backends::GL`. A missing/broken WebGL2 context is an
//!   explicit refusal (stable marker + status `step: "webgl2"`), never a blank
//!   canvas.
//! - **No command surface.** Pan/zoom are camera-only (a uniform change: no
//!   re-parse and no re-upload), which is exactly what a viewport needs. There is
//!   no measurement, annotation, editing or wire-protocol work here.
//! - **No browser run evidence from this lane.** The crate compiles for
//!   `wasm32-unknown-unknown` and the shell under `apps/app-web/web/` is the
//!   minimal page, but nothing here was executed in a browser for this change;
//!   pixel evidence belongs to the evidence lane.
//!
//! Native builds cannot present on a DOM canvas, so every entry point below
//! returns the explicit NOT IMPLEMENTED diagnostic instead of a stub success.

/// The stable reason a native build has no canvas host.
///
/// This is a platform constraint, not a placeholder: presenting into a DOM
/// `<canvas>` requires a wasm module running in the page.
pub const NOT_IMPLEMENTED: &str =
    "web canvas host: NOT IMPLEMENTED on this target — build for wasm32-unknown-unknown";

#[cfg(target_arch = "wasm32")]
mod session;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Start the wasm CAD session and present the first frame on `canvas_id`.
///
/// `css_w`/`css_h` are the canvas's CSS-pixel size and `dpr` its
/// `devicePixelRatio`; the presenter is configured in physical pixels
/// (`CSS px × dpr`), so a fractional-DPI display is not blurred. Sizes must be
/// finite and positive. `canvas_id` must name an `HtmlCanvasElement` in the host
/// page.
///
/// Fails (and records the failure in [`status()`]) when the canvas is missing,
/// when no usable WebGL2 context can be created, or when the fixture cannot be
/// imported/uploaded/presented. A second `start` on a live session is refused:
/// one canvas has exactly one presenter.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start(canvas_id: &str, css_w: f64, css_h: f64, dpr: f64) -> Result<String, JsValue> {
    session::start_impl(canvas_id, css_w, css_h, dpr).await
}

/// Reconfigure the canvas for a new CSS size / device pixel ratio and present a
/// fresh frame at the new physical size.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn resize(css_w: f64, css_h: f64, dpr: f64) -> Result<String, JsValue> {
    session::resize_impl(css_w, css_h, dpr)
}

/// Zoom about a canvas-local CSS-pixel cursor, then present immediately.
///
/// `factor > 1` zooms in. The world point under the cursor stays under it —
/// the same `cad_app::Camera::zoom_at` rule the desktop session applies.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn zoom_at(factor: f64, x: f64, y: f64) -> Result<String, JsValue> {
    session::zoom_at_impl(factor, x, y)
}

/// Pan the view by a CSS-pixel pointer delta, then present immediately.
///
/// The delta is a **screen** delta (x right, y down); it is converted with the
/// camera's own `world_per_px`, exactly like the desktop host's pointer pan.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn pan(dx: f64, dy: f64) -> Result<String, JsValue> {
    session::pan_impl(dx, dy)
}

/// The JSON status object of the current session (or of the last failure).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn status() -> String {
    session::status_impl()
}

/// Native builds cannot run the browser canvas host; this is a platform
/// constraint, not a placeholder (see [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn start(_canvas_id: &str, _css_w: f64, _css_h: f64, _dpr: f64) -> Result<String, String> {
    Err(NOT_IMPLEMENTED.to_string())
}

/// Native builds cannot resize a DOM canvas (see [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn resize(_css_w: f64, _css_h: f64, _dpr: f64) -> Result<String, String> {
    Err(NOT_IMPLEMENTED.to_string())
}

/// Native builds cannot zoom a DOM canvas (see [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn zoom_at(_factor: f64, _x: f64, _y: f64) -> Result<String, String> {
    Err(NOT_IMPLEMENTED.to_string())
}

/// Native builds cannot pan a DOM canvas (see [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn pan(_dx: f64, _dy: f64) -> Result<String, String> {
    Err(NOT_IMPLEMENTED.to_string())
}

/// Native builds have no session to report on (see [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn status() -> String {
    NOT_IMPLEMENTED.to_string()
}
