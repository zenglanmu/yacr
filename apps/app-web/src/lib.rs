//! Browser host: the **real CAD session + WebGL2 canvas** of the `dom-canvas`
//! runtime path (ADR 0024, `surfaceMode: dom-canvas`).
//!
//! This crate owns one CAD session **inside wasm**: `cad_ui_core::session::CadSession`
//! — the same owner-thread session the native hosts drive, with the real `cad-db`
//! authority and the real command router — presenting through
//! [`cad_render_wgpu::canvas::CanvasPresenter`]. It is not a second rendering
//! mechanism next to the native presenter, and it is not a second document next to
//! the chrome: the WebView/native shell owns window lifetime, i18n and the
//! admission policy, and must **not** hold a CAD session or a copy of the document
//! (ADR 0024 D2, `AGENTS.md` iron rule 8).
//!
//! # One session, two carriers
//!
//! | entry point | canvas | runs in | session |
//! |---|---|---|---|
//! | [`start`] / [`open_bytes`] | the page's `<canvas id=...>` | the page's main thread | **the real [`CadSession`]** ([`live`] + [`page`]) |
//! | [`start_offscreen`] / [`open_bytes_offscreen`] | an `OffscreenCanvas` transferred with `transferControlToOffscreen()` | a **Worker** | the viewer ([`session`]), unchanged |
//!
//! The **page** carrier is the production flip of ADR 0024 stage ⑧: it applies the
//! ADR 0025 budgets, keeps the database (it is the authority), answers the whole
//! desktop wire vocabulary ([`direct::invoke`]) and runs the real input path. The
//! **worker** carrier still runs the older viewer session — camera-only navigation
//! over a released database — and is deliberately untouched by this change; it has
//! no `invoke`/`probe_input`/`shell_*` surface yet (those refuse explicitly). Moving
//! it onto the real session is the remaining half of stage ⑧ (`docs/handoff.md`).
//!
//! ## Measured capability split (2026-10-24)
//!
//! Worker-side rendering is a **browser capability**, and the two engines this
//! project targets disagree:
//!
//! - **Chromium 141** — WebGL2 **works** inside a dedicated Worker on an
//!   `OffscreenCanvas` transferred with `transferControlToOffscreen()` (verified:
//!   ANGLE/Intel adapter, clear + `readPixels` correct).
//! - **WebKitGTK 2.52.6** — `OffscreenCanvas.getContext("webgl2")` in a Worker
//!   returns **`null`** (verified). **WebKitGTK cannot do worker rendering at all**;
//!   it is not a tuning or a fallback problem. [`start_offscreen`] therefore fails
//!   with the explicit `webgl2` refusal (marker `yacr: unsupported-runtime: webgl2`)
//!   instead of painting a blank canvas.
//!
//! [`worker_capability`] is how a host learns this *before* choosing A carrier: it
//! returns `{ok, isolation, reason, context, webgl2_in_worker}`, so the page can
//! state `isolation=main-thread (no WebGL2 in worker)` on WebKitGTK and run isolated
//! on Chromium. A host must never guess isolation from a feature sniff
//! (`OffscreenCanvas` exists on both engines).
//!
//! ## Single owner, enforced
//!
//! A canvas has exactly one session and a wasm instance has exactly one of them:
//! [`start`] claims the page slot before the (async) presenter exists and refuses a
//! second start, [`open_bytes`] **replaces the document in place**
//! (`CadSession::open_bytes` → the host's own `deliver_external_open`), and
//! `start_offscreen` is refused outside a Worker so a page-side `OffscreenCanvas`
//! session can never compete with the page's own. The two carriers cannot both be
//! live: the page path needs a page `<canvas>` (absent in a Worker) and the worker
//! path needs an `OffscreenCanvas` in a Worker.
//!
//! # Exported API (the contract the hosts are wired against)
//!
//! ```text
//! start(canvas_id, css_w, css_h, dpr)                       -> JSON status (async)
//! open_bytes(canvas_id, label, css_w, css_h, dpr, bytes)    -> JSON status (async)
//! frame()                                                   -> JSON frame line
//! resize(css_w, css_h, dpr)                                 -> JSON status
//! zoom_at(factor, x, y)                                     -> JSON status (CSS px)
//! pan(dx, dy)                                               -> JSON status (CSS px)
//! fit_view()                                                -> JSON status
//! status()                                                  -> JSON status
//! document()                                                -> JSON document summary
//! last_import_ms()                                          -> f64 milliseconds
//! probe_input(args_json)                                    -> JSON pointer acknowledgement
//! invoke(name, args_json) -> {"ok":true,"value":…} | {"ok":false,"error":…}
//! shell_status()                                            -> JSON `cad_status` snapshot
//! shell_commands()                                          -> JSON array of command names
//! shell_aliases()                                           -> JSON array of `{alias, command}` rows
//! shell_command(name)                                       -> `{"accepted":true}` or a refusal
//! start_offscreen(canvas, css_w, css_h, dpr)                -> JSON status (async, Worker only)
//! open_bytes_offscreen(canvas, label, css_w, css_h, dpr, bytes) -> JSON status (async, Worker only)
//! worker_capability()                                       -> JSON capability report
//! ```
//!
//! `resize`/`zoom_at`/`pan`/`fit_view`/`status`/`document`/`last_import_ms` are
//! **carrier-agnostic**: they act on whichever carrier owns the session, so the page
//! and the Worker call exactly the same exports. Only the *opening* calls differ,
//! because only they need a canvas handle.
//!
//! # The frame loop is the host's, not wasm's
//!
//! [`frame`] is **one** explicit step (tick → publish the prepared scene → render →
//! blit on the canvas). The host calls it from `requestAnimationFrame`; nothing
//! inside wasm starts a timer. The navigation exports present one frame themselves,
//! so a chrome that predates [`frame`] (the current one) keeps repainting during a
//! pan or a zoom.
//!
//! # Failure vocabulary (stable)
//!
//! Every canvas export rejects with `Err(JsValue)` whose text is `step: message`,
//! and the status carries the same `step`/`error`:
//!
//! - `import` — the importer rejected the bytes; the message is the importer's own
//!   diagnostic, prefixed with the file label.
//! - `webgl2` — no usable WebGL2 context/surface (carries the refusal marker
//!   `yacr: unsupported-runtime: webgl2`, which the host turns into exit code 6).
//!   This includes the measured WebKitGTK worker case.
//! - `isolation` — `start_offscreen`/`open_bytes_offscreen` was called outside a
//!   Worker, i.e. where it would be a second owner of the single session.
//! - `canvas-element` — the page has no such element / it is not a `<canvas>`.
//! - `viewport` — the CSS size / `devicePixelRatio` is not usable.
//! - `budget` — a **named memory budget** refused the input (see [`budget`]): the
//!   file is larger than [`budget::MAX_INPUT_BYTES`], or the wasm heap already
//!   exceeds [`budget::MAX_WASM_HEAP_BYTES`] before another document is installed.
//!   The message names the budget, its limit and the measured value, and the status
//!   carries a structured `budget: {name, limit_bytes, measured_bytes}`. The open
//!   document stays intact and navigable — a refusal is never a blank canvas and
//!   never a silent degradation.
//! - `input` — a pointer/command step the session refused (an unsupported button, a
//!   stale metrics revision, the modal popup gate).
//! - `state` — there is no live session for the call.
//!
//! [`open_bytes`] applies both budgets **before** the bytes reach the importer and
//! replaces the document in place, so a bad/unsupported/over-budget file leaves the
//! open document, its pixels and its camera exactly as they were.
//!
//! # Measurement surfaces (evidence, not performance claims)
//!
//! - `last_import_ms()` — the last completed build/import, measured with the
//!   session's injected `web_time` clock (never `std::time` on this target).
//! - `wasm_heap_bytes` — the module's own `WebAssembly.Memory` byte length at the
//!   time of the call. It is the **wasm heap**, not process RSS, and it is a
//!   high-water mark (wasm memory never shrinks).
//! - `heap_after_open` — the same measurement taken when an open completed; with
//!   [`live::DB_RETAINED`] `true` it includes the retained database, which is the
//!   number that separates this carrier from the viewer's released one.
//! - `batches` — the prepared scene's resident batch count; `draws` is `null` (the
//!   session's engine owns the draw submission count and does not publish it here).
//!
//! # Honest limits (2026-10-28, this lane)
//!
//! - **Bytes in, one file.** `open_bytes` takes the DWG bytes from the caller; there
//!   is no filesystem in wasm and no reading of a path. Import, proxy and resource
//!   decoding are exactly what `cad-import-acadrust` does on this target.
//! - **WebGL2 only.** Neither target runtime exposes `navigator.gpu`, so the
//!   presenter requests `Backends::GL`. A missing/broken WebGL2 context is an
//!   explicit refusal, never a blank canvas.
//! - **The carrier advertises no host-side New/Open/Save/Plot path**
//!   ([`cad_platform::HostCapabilities::default`]): `canNew`/`canOpen`/`canSave`/
//!   `canPlot` are `false` because there is no filesystem, no save dialog and no
//!   plot export behind them. Opening is the transport's command (the page's file
//!   picker), which lifts `canOpen` itself.
//! - **No shaping fonts, no image decoder.** A drawing that requests a face or a
//!   raster image still opens, and the resource install is reported as a
//!   non-fatal diagnostic ([`platform`]) — its text is drawn unshaped and images
//!   are not decoded. That gap is unchanged by this lane.
//! - **The worker carrier is still the viewer.** It has no wire surface
//!   (`invoke`/`probe_input`/`shell_*` refuse there), its status keeps the viewer's
//!   fields, and it still releases the database after the display build.
//! - **No browser run in this file's docs.** The runtime evidence for the flip is
//!   `docs/validation-dom-canvas-flip-2026-10-28.md` (headless Chromium,
//!   SwiftShader: layout/DOM/IPC evidence, **not** GPU, pixel-correctness or
//!   performance acceptance), and the host-side acceptance is `cargo test -p
//!   app-web` — a real session, no GPU, no DOM.

/// The stable reason a native build has no canvas host.
///
/// This is a platform constraint, not a placeholder: presenting into a DOM
/// `<canvas>` requires a wasm module running in the page.
pub const NOT_IMPLEMENTED: &str =
    "web canvas host: NOT IMPLEMENTED on this target — build for wasm32-unknown-unknown";

/// Named memory budgets (ADR 0025), applied by the production session.
///
/// Target-independent (no `web-sys`, no `wgpu`, no CAD type) so its arithmetic is
/// unit-tested on the host, not only observable as a dead module in a browser.
pub mod budget;

/// The camera arithmetic of the **viewer** carrier's open and `FIT` selector.
///
/// Target-independent on purpose (no DOM, no GPU, no session state). The production
/// page carrier does not use it: its fit and its zoom steps are the real session's
/// (`CadSession::command("FIT")`, the same rule the desktop hosts run).
pub mod view;

/// The four platform contracts the browser supplies to the shared desktop CAD
/// session, plus the bytes-based entry it is built with.
///
/// Target-independent on purpose (no DOM, no wgpu, no session state): every refusal
/// and every honest absence (no filesystem, non-persistent config, unmapped surface,
/// no font/image package) is asserted by `cargo test -p app-web` on the host. See
/// [`platform`] for the capability table and the honest limits.
pub mod platform;

/// Which isolation a browser can actually deliver (`worker_capability()`).
///
/// Target-independent model ([`capability::Capability`]) plus a wasm-only probe, so
/// the decision matrix (Chromium 141 vs WebKitGTK 2.52.6) is unit tested on the
/// host.
pub mod capability;

/// The **host-agnostic shell surface**: `shell_status`/`shell_commands`/
/// `shell_aliases`/`shell_command`, i.e. the `cad_status` snapshot and command
/// vocabulary the shared React chrome (`apps/desktop-ui`) polls, served without
/// Tauri IPC.
///
/// Target-independent on purpose: the status is the session's own
/// `CadStatus` snapshot plus the four carrier keys, so its exact wire shape is
/// asserted by host unit tests against a real session rather than only in a
/// browser. Read the module docs for the per-field table.
pub mod shell;

/// **The production `dom-canvas` session** (`CadSession` behind the canvas
/// exports): budgets, document replacement, input synthesis, the popup gate and
/// both status projections.
///
/// Target-independent on purpose — the session state and every rule are plain Rust
/// over `cad-host`/`cad-ui-core`, so the acceptance of the flip runs as
/// `cargo test -p app-web` on the host. The pixel half (device, render, blit) is
/// [`page`], which is wasm-only. Read [`live`] for the two things the real session
/// changes (retained database, in-place replacement) and for where the budgets
/// apply.
pub mod live;

/// **The `dom-canvas` transport**: one `invoke(name, args_json)` entry over the
/// shared command semantics (`cad_ui_core::session::wire`).
///
/// The desktop hosts serve the chrome through Tauri IPC; a browser has none, so the
/// same JS call is served in-process. This module decodes the same arguments into
/// the same typed values and calls the *same* `wire::*` function per command, so a
/// name, an argument or a refusal cannot differ between host transports.
/// Target-independent on purpose: the dispatch table and the equivalence matrix are
/// asserted by `cargo test -p app-web`.
pub mod direct;

#[cfg(target_arch = "wasm32")]
mod page;

#[cfg(target_arch = "wasm32")]
mod session;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Route a carrier-agnostic export to whichever carrier owns the session.
///
/// The page carrier is the real session ([`page`]); the worker carrier is the
/// viewer ([`session`]). They cannot both be live (see the module docs), so the
/// question "is the page carrier engaged" is the whole dispatch.
#[cfg(target_arch = "wasm32")]
fn carrier<T>(page_carrier: impl FnOnce() -> T, worker_carrier: impl FnOnce() -> T) -> T {
    if page::is_engaged() {
        page_carrier()
    } else {
        worker_carrier()
    }
}

/// Start the wasm CAD session on the **isolated carrier**: an `OffscreenCanvas`
/// transferred into this Worker.
///
/// This is the worker-side sibling of [`start`]: same viewer session, same
/// compiled-in synthetic fixture, same budgets and upload — only the carrier
/// differs. `canvas` must be an `OffscreenCanvas` this Worker owns, produced by the
/// page with `HtmlCanvasElement.transferControlToOffscreen()` and delivered with
/// `postMessage(message, [offscreen])`; the host must **not** keep using that canvas
/// (the transfer detaches it).
///
/// # Measured limits
///
/// - **Chromium 141**: works (WebGL2 on an `OffscreenCanvas` in a Worker).
/// - **WebKitGTK 2.52.6**: fails here with `step: "webgl2"` and the marker
///   `yacr: unsupported-runtime: webgl2`, because `getContext("webgl2")` returns
///   `null` for an `OffscreenCanvas` in a Worker on that engine. Worker rendering is
///   impossible there; the host must use the main-thread carrier ([`start`]) and
///   report `isolation=main-thread (no WebGL2 in worker)`.
///
/// Refused with `step: "isolation"` when called outside a Worker: a page-side
/// `OffscreenCanvas` session would be a second, competing owner of the single CAD
/// session. Query [`worker_capability`] first rather than attempting it.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start_offscreen(
    canvas: web_sys::OffscreenCanvas,
    css_w: f64,
    css_h: f64,
    dpr: f64,
) -> Result<String, JsValue> {
    session::start_offscreen_impl(canvas, css_w, css_h, dpr).await
}

/// Open **caller-supplied DWG `bytes`** on the isolated carrier (worker-side
/// sibling of [`open_bytes`]).
///
/// Identical staged halves — geometry + budgets first, then replace — so a bad,
/// unsupported, empty or over-budget file leaves the Worker's live session (and its
/// canvas pixels) intact. `label`/`css_w`/`css_h`/`dpr` follow [`open_bytes`];
/// `canvas` and the isolation rule follow [`start_offscreen`].
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn open_bytes_offscreen(
    canvas: web_sys::OffscreenCanvas,
    label: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
    bytes: &[u8],
) -> Result<String, JsValue> {
    session::open_bytes_offscreen_impl(canvas, label, css_w, css_h, dpr, bytes).await
}

/// The capability report of the **current** JS context:
/// `{ok, isolation, reason, context, webgl2_in_worker}`.
///
/// Isolated worker rendering is a browser capability, and the measured engines
/// disagree (see the module docs): Chromium 141 gives WebGL2 on a worker-side
/// `OffscreenCanvas`, WebKitGTK 2.52.6 returns `null`. Called from the worker this
/// answers that question for real (`new OffscreenCanvas(1, 1).getContext("webgl2")`);
/// called from the page it reports `isolation: "main-thread"` with the reason that
/// the probe must run in a worker.
///
/// A host uses it to choose a carrier **before** creating any session: it must not
/// infer worker support from the presence of `OffscreenCanvas`/`transferControlToOffscreen`,
/// which both engines have.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn worker_capability() -> String {
    capability::report()
}

/// Native builds cannot start an isolated session (see the wasm-only
/// [`start_offscreen`]).
///
/// The canvas argument is `()` because there is no native `OffscreenCanvas` handle
/// to accept: this signature cannot be called into a success by construction, which
/// is the point — a native build has no Worker and no WebGL2 canvas.
#[cfg(not(target_arch = "wasm32"))]
pub fn start_offscreen(_canvas: (), _css_w: f64, _css_h: f64, _dpr: f64) -> Result<String, String> {
    Err(NOT_IMPLEMENTED.to_string())
}

/// Native builds cannot open a DWG on an isolated carrier (see
/// [`start_offscreen`] and [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn open_bytes_offscreen(
    _canvas: (),
    _label: &str,
    _css_w: f64,
    _css_h: f64,
    _dpr: f64,
    _bytes: &[u8],
) -> Result<String, String> {
    Err(NOT_IMPLEMENTED.to_string())
}

/// Native builds have no JS context to probe, so they report isolation lie-free:
/// `main-thread` (the only thing a native build could ever be) with the native
/// refusal as the reason. Never `worker`.
#[cfg(not(target_arch = "wasm32"))]
pub fn worker_capability() -> String {
    capability::Capability::new(capability::Context::Window, false).to_json()
}

/// Start the wasm CAD session on the compiled-in synthetic fixture and present the
/// first frame on `canvas_id`.
///
/// The session is the **real** desktop session ([`live`]): `cad-db` authority,
/// command router, the full wire vocabulary. `css_w`/`css_h` are the canvas's
/// CSS-pixel size and `dpr` its `devicePixelRatio`; the presenter is configured in
/// physical pixels (`CSS px × dpr`) and the session's canvas metrics are set from
/// the same measurement, so a fractional-DPI display is not blurred and a
/// canvas-local pointer coordinate is not scaled twice.
///
/// The ADR 0025 budgets run before the bytes are parsed. A first frame that the
/// engine has not prepared yet is reported as `waited` (see [`frame`]), never as a
/// presented frame.
///
/// Fails (and records the failure in [`status()`]) when the canvas is missing, when
/// no usable WebGL2 context can be created, when a budget refuses the fixture or
/// when the fixture cannot be imported. A second `start` on a live session is
/// refused: one canvas has exactly one presenter. Use [`open_bytes`] to open
/// another file.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start(canvas_id: &str, css_w: f64, css_h: f64, dpr: f64) -> Result<String, JsValue> {
    page::start_impl(canvas_id, css_w, css_h, dpr).await
}

/// Open **caller-supplied DWG `bytes`** as the session document and present the
/// first frame on `canvas_id`.
///
/// `label` is the source file name shown by the host (`status()`/`document()` report
/// it); it must not be empty. `css_w`/`css_h`/`dpr` follow [`start`].
///
/// A live session **replaces its document in place** (the desktop open path:
/// `CadSession::open_bytes` → `HostCore::deliver_external_open`), so the device, the
/// presenter, the platform adapters and the import counter survive. Both budgets run
/// first, and a failure — an import refusal, a budget refusal — is reported
/// **without** disturbing the live session: the open document keeps its pixels and
/// stays navigable. With no session yet, this builds one exactly like [`start`].
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
    page::open_bytes_impl(canvas_id, label, css_w, css_h, dpr, bytes).await
}

/// **One frame step** on the live page session.
///
/// The host calls this from its `requestAnimationFrame` loop (that is the whole
/// loop; nothing inside wasm runs a timer): the session ticks (host pass, metrics,
/// view sync, scene preparation), the prepared scene revision is published to the
/// GPU, the frame is rendered and blitted onto the canvas.
///
/// Answers `{ok, presented, frames, waited, ticks, canvasFrames}`. `presented` is
/// `false` when `render` reported no frame — a real wait for a scene that is not
/// committed yet, counted in `waited`, **never** a presented frame.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn frame() -> Result<String, JsValue> {
    page::frame_impl()
}

/// Reconfigure the canvas for a new CSS size / device pixel ratio, set the session's
/// canvas metrics, and present a fresh frame at the new physical size.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn resize(css_w: f64, css_h: f64, dpr: f64) -> Result<String, JsValue> {
    carrier(
        || page::resize_impl(css_w, css_h, dpr),
        || session::resize_impl(css_w, css_h, dpr),
    )
}

/// Zoom about a canvas-local CSS-pixel cursor, then present immediately.
///
/// `factor > 1` zooms in. On the page carrier this is **one wheel event** through
/// the session's own input path (`handle_input`), synthesized from the factor with
/// the host's `1 + delta × 0.0015` rule, so the world point under the cursor stays
/// under it. A factor the host would clamp (`0.2..=5.0`) is refused instead of
/// applied approximately.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn zoom_at(factor: f64, x: f64, y: f64) -> Result<String, JsValue> {
    carrier(
        || page::zoom_at_impl(factor, x, y),
        || session::zoom_at_impl(factor, x, y),
    )
}

/// Pan the view by a CSS-pixel pointer delta, then present immediately.
///
/// The delta is a **screen** delta (x right, y down). On the page carrier it is the
/// **middle-button drag** the desktop host pans with: the press is retained across
/// calls (anchored at the viewport centre) and the accumulated delta is moved, so
/// the pan is exact and no pixel is lost to the drag threshold. The gesture ends at
/// the next non-pan interaction (`probe_input`, a command, a resize).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn pan(dx: f64, dy: f64) -> Result<String, JsValue> {
    carrier(|| page::pan_impl(dx, dy), || session::pan_impl(dx, dy))
}

/// Refit the camera to the drawing extents, then present.
///
/// On the page carrier this is the `FIT` selector through the **real** router
/// (`CadSession::command("FIT")`) — the same fit the desktop `FIT` button runs. It
/// fails explicitly when the session holds no drawing; never a silent no-op that
/// would look like a refit.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn fit_view() -> Result<String, JsValue> {
    carrier(|| page::fit_view_impl(), || session::fit_view_impl())
}

/// The JSON status object of the current session (or of the last failure).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn status() -> String {
    carrier(|| page::status_impl(), || session::status_impl())
}

/// The JSON summary of the open document.
///
/// Page carrier: `{ok, label, bytes, entityCount, batches, draws, presented, frames,
/// importCount, import_ms, wasm_heap_bytes, heap_after_open, db_retained}` — the
/// session's own facts, with `db_retained: true` because the real session keeps the
/// document (`draws` is `null`: the engine does not publish a draw count here).
///
/// With no live session it is `ok: false` with null/zero values — never a made-up
/// file.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn document() -> String {
    carrier(|| page::document_impl(), || session::document_impl())
}

/// Build/import (page carrier) or import + build + upload (viewer) of the last
/// completed open, in milliseconds.
///
/// Negative until one open has completed (no default or estimated number). This is
/// a measurement surface for the evidence run, not a performance claim.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn last_import_ms() -> f64 {
    carrier(
        || page::last_import_ms_impl(),
        || session::last_import_ms_impl(),
    )
}

/// Feed **one wire pointer input** to the live page session.
///
/// `args_json` is the same `ProbeInput` object the desktop `probe_input` IPC carries
/// (`{sequence, kind, x, y, dx, dy, width, height, dpr, button, shift}`), and the
/// effect is the session's own `handle_input` — the single input seam every carrier
/// uses. `kind` is one of `pointer_down`/`pointer_move`/`pointer_up`/
/// `pointer_cancel`/`wheel`/`resize`/`blur`/`pointer_leave`.
///
/// Answers `{ok, accepted, sequence, kind}`; a refusal is an answer with
/// `accepted: false` and a stable `reason` (the modal popup gate refuses every CAD
/// interaction by name, ADR-0021 D2). An argument object that is not a `ProbeInput`
/// rejects instead: that is a transport error, not a session verdict.
///
/// The worker carrier has no input surface of its own: there this refuses with the
/// explicit "no live CAD session on this carrier" message.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn probe_input(args_json: &str) -> Result<String, JsValue> {
    page::probe_input_impl(args_json)
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

/// Native builds have no frame loop (see [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn frame() -> Result<String, String> {
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

/// Native builds cannot refit a DOM canvas (see [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn fit_view() -> Result<String, String> {
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

/// Native builds have no page session to feed input to (see [`NOT_IMPLEMENTED`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn probe_input(_args_json: &str) -> Result<String, String> {
    Err(NOT_IMPLEMENTED.to_string())
}

// ---------------------------------------------------------------------------
// The shell surface (see [`shell`] for the per-field table)
// ---------------------------------------------------------------------------

/// The JSON `cad_status` snapshot for the shared React chrome.
///
/// The session's own `CadStatus` snapshot plus the four `dom-canvas` carrier keys;
/// see [`shell`] for every field's source. Without a live session it is the honest
/// empty status.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn shell_status() -> String {
    page::shell_status_impl()
}

/// The command vocabulary this carrier executes: the router's whole
/// `admissible_names()` list, because the session behind it is the real one.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn shell_commands() -> String {
    page::shell_commands_impl()
}

/// The alias table (`{alias, command}` rows), the desktop one.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn shell_aliases() -> String {
    page::shell_aliases_impl()
}

/// Execute one command line through the **shared** wire layer, or refuse it.
///
/// Answers `{"accepted":true}` — the only shape `queueCadCommand` treats as
/// accepted. A name the shared gate does not admit is refused with the stable
/// `unsupported-runtime: cad_command` prefix followed by the gate's own sentence; a
/// refusal the session makes is returned verbatim. The worker carrier (viewer
/// session) answers the same refusal: it has no command router.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn shell_command(command: &str) -> Result<String, JsValue> {
    page::shell_command_impl(command).map_err(|message| JsValue::from_str(&message))
}

/// Native builds have no browser registry to project: the honest empty status is
/// the whole truth on that target (see [`shell`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn shell_status() -> String {
    shell::status_json(&shell::empty_status(), &shell::Carrier::default())
}

/// The same vocabulary on every target: it is a property of the router, not of the
/// build.
#[cfg(not(target_arch = "wasm32"))]
pub fn shell_commands() -> String {
    shell::commands_json()
}

/// The same alias table on every target (see [`shell::aliases_json`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn shell_aliases() -> String {
    shell::aliases_json()
}

/// Resolve the name through the **shared** gate, then report the platform failure:
/// a native build has no wasm session to run it on, so it must never answer
/// `accepted`.
///
/// An unadmitted name keeps the stable [`shell::SHELL_COMMAND_REFUSAL`] prefix plus
/// the gate's own reason, exactly like a live carrier would refuse it.
#[cfg(not(target_arch = "wasm32"))]
pub fn shell_command(command: &str) -> Result<String, String> {
    match cad_ui_core::shell::check_command(true, false, 0, command) {
        Ok(_) => Err(NOT_IMPLEMENTED.to_string()),
        Err(reason) => Err(shell::refusal(&reason)),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_tests {
    //! The honesty rule of the native target: every browser-only export refuses
    //! with the explicit NOT IMPLEMENTED diagnostic, and the **pure** answers keep
    //! their honest value instead of refusing.

    use super::*;

    #[test]
    fn every_browser_only_export_refuses_explicitly() {
        assert_eq!(
            start("cad-canvas", 800.0, 600.0, 1.0).unwrap_err(),
            NOT_IMPLEMENTED
        );
        assert_eq!(
            open_bytes("cad-canvas", "a.dwg", 800.0, 600.0, 1.0, b"x").unwrap_err(),
            NOT_IMPLEMENTED
        );
        assert_eq!(frame().unwrap_err(), NOT_IMPLEMENTED);
        assert_eq!(resize(800.0, 600.0, 1.0).unwrap_err(), NOT_IMPLEMENTED);
        assert_eq!(zoom_at(1.25, 1.0, 2.0).unwrap_err(), NOT_IMPLEMENTED);
        assert_eq!(pan(3.0, 4.0).unwrap_err(), NOT_IMPLEMENTED);
        assert_eq!(fit_view().unwrap_err(), NOT_IMPLEMENTED);
        assert_eq!(probe_input("{}").unwrap_err(), NOT_IMPLEMENTED);
        assert_eq!(status(), NOT_IMPLEMENTED);
        assert_eq!(document(), NOT_IMPLEMENTED);
        assert_eq!(last_import_ms(), -1.0);
        assert_eq!(
            start_offscreen((), 800.0, 600.0, 1.0).unwrap_err(),
            NOT_IMPLEMENTED
        );
    }

    #[test]
    fn the_pure_answers_stay_honest_instead_of_refusing() {
        // The isolation report is a real answer on any target.
        let capability: serde_json::Value =
            serde_json::from_str(&worker_capability()).expect("valid JSON");
        assert_eq!(capability["isolation"], serde_json::json!("main-thread"));
        // The vocabulary and the status are properties of the carrier, not of the
        // target: `cargo test -p app-web` asserts their exact shape.
        assert!(shell::commands_json().starts_with('['));
        assert!(shell::aliases_json().starts_with('['));
    }
}
