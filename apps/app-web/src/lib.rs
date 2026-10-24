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
//! start(canvas_id, css_w, css_h, dpr)                       -> JSON status (async)
//! open_bytes(canvas_id, label, css_w, css_h, dpr, bytes)    -> JSON status (async)
//! resize(css_w, css_h, dpr)                                 -> JSON status
//! zoom_at(factor, x, y)                                     -> JSON status (CSS px)
//! pan(dx, dy)                                               -> JSON status (CSS px)
//! status()                                                  -> JSON status
//! document()                                                -> JSON document summary
//! last_import_ms()                                          -> f64 milliseconds
//! ```
//!
//! Every status call returns the JSON status object on success and `Err(JsValue)`
//! with a message naming the failing step on failure; the last failure is also
//! kept so [`status()`] reports it. A status carries at least
//! `{ ok, backend, adapter_name, format, physical: [w, h], label, bytes, batches,
//! draws, presented }`, plus `step`/`error` (and, for a `step` of `webgl2`, the
//! stable `marker` `yacr: unsupported-runtime: webgl2`) on failure.
//!
//! `start` loads the compiled-in synthetic fixture with the stable label
//! `synthetic-four-lines.dwg`; `open_bytes` opens **caller-supplied DWG bytes**
//! (the browser shell reads them with the File API) and replaces a live session.
//!
//! # Failure vocabulary (stable)
//!
//! - `import` — the importer rejected the bytes; the message is the importer's
//!   own diagnostic, prefixed with the file label.
//! - `empty-scene` — the file parsed but produced zero draw batches; a blank
//!   canvas is never presented as a successful open.
//! - `webgl2` — no usable WebGL2 context/surface (carries the refusal marker).
//!
//! `open_bytes` runs the geometry load **before** it touches the session registry,
//! so a bad/unsupported/empty file leaves a live session (and its canvas pixels)
//! intact; the failure is reported on the still-open session instead.
//!
//! # Measurement surfaces (evidence, not performance claims)
//!
//! - `last_import_ms()` — the last completed import + scene build + GPU upload,
//!   measured with `web_time::Instant` (never `std::time::Instant`, which panics
//!   on `wasm32-unknown-unknown`). Negative until one open has completed.
//! - `document().wasm_heap_bytes` — the module's own `WebAssembly.Memory` byte
//!   length. It is the **wasm heap**, not process RSS: there is no process to
//!   measure in a browser tab.
//!
//! # Honest limits (2026-10-24, this lane)
//!
//! - **Bytes in, one file.** `open_bytes` takes the DWG bytes from the caller;
//!   there is no filesystem in wasm and no reading of a path. Import, proxy and
//!   resource decoding are exactly what `cad-import-acadrust` does on this
//!   target — no more.
//! - **WebGL2 only.** Neither target runtime exposes `navigator.gpu`, so the
//!   presenter requests `Backends::GL`. A missing/broken WebGL2 context is an
//!   explicit refusal (stable marker + status `step: "webgl2"`), never a blank
//!   canvas.
//! - **No command surface.** Pan/zoom are camera-only (a uniform change: no
//!   re-parse and no re-upload), which is exactly what a viewport needs. There is
//!   no measurement, annotation, editing or wire-protocol work here.
//! - **No second session.** One process-global session: `start` refuses to build
//!   a second one, and `open_bytes` disposes the previous renderer/presenter
//!   before creating the replacement.
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

/// Start the wasm CAD session on the compiled-in synthetic fixture and present
/// the first frame on `canvas_id`.
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
/// one canvas has exactly one presenter. Use [`open_bytes`] to open another file.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start(canvas_id: &str, css_w: f64, css_h: f64, dpr: f64) -> Result<String, JsValue> {
    session::start_impl(canvas_id, css_w, css_h, dpr).await
}

/// Open **caller-supplied DWG `bytes`** as the session document and present the
/// first frame on `canvas_id`.
///
/// `label` is the source file name shown by the host (`status()`/`document()`
/// report it); it must not be empty. `css_w`/`css_h`/`dpr` follow [`start`].
///
/// Replaces a live session: the previous renderer and canvas presenter are
/// disposed before the replacement presenter is created, so one canvas never has
/// two presenters and no surface is leaked. A failure to import the bytes
/// (`step: "import"`) or a file with no drawable geometry
/// (`step: "empty-scene"`) is reported **without** disturbing a live session.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn open_bytes(
    canvas_id: &str,
    label: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
    bytes: &[u8],
) -> Result<String, JsValue> {
    session::open_bytes_impl(canvas_id, label, css_w, css_h, dpr, bytes).await
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

/// The JSON summary of the open document:
/// `{ ok, label, batches, draws, presented, bytes, import_ms, wasm_heap_bytes }`.
///
/// With no live session it is `ok: false` with null/zero values — never a
/// made-up file. `wasm_heap_bytes` is the module's own wasm linear memory (not
/// process RSS).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn document() -> String {
    session::document_impl()
}

/// Import + display-representation build + GPU upload of the last completed open,
/// in milliseconds, measured with `web_time::Instant`.
///
/// Negative until one open has completed (no default or estimated number). This
/// is a measurement surface for the evidence run, not a performance claim.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn last_import_ms() -> f64 {
    session::last_import_ms_impl()
}

/// Native builds cannot run the browser canvas host; this is a platform
/// constraint, not a placeholder (see [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn start(_canvas_id: &str, _css_w: f64, _css_h: f64, _dpr: f64) -> Result<String, String> {
    Err(NOT_IMPLEMENTED.to_string())
}

/// Native builds cannot open a DWG into a DOM canvas (see [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn open_bytes(
    _canvas_id: &str,
    _label: &str,
    _css_w: f64,
    _css_h: f64,
    _dpr: f64,
    _bytes: &[u8],
) -> Result<String, String> {
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

/// Native builds have no document (see [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn document() -> String {
    NOT_IMPLEMENTED.to_string()
}

/// Native builds measure nothing: the documented "no measurement" value, not a
/// fabricated duration (see [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn last_import_ms() -> f64 {
    -1.0
}
