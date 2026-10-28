//! Browser host: the **wasm CAD session + WebGL2 canvas** of the `dom-canvas`
//! runtime path (ADR 0024, `surfaceMode: dom-canvas`).
//!
//! This crate owns one CAD session **inside wasm**: it imports a drawing, builds
//! the display scene, renders it with the renderer's offscreen frame texture and
//! presents that frame on a web canvas through
//! [`cad_render_wgpu::canvas::CanvasPresenter`]. It is not a second rendering
//! mechanism next to the native presenter — it is the pixel carrier for hosts
//! that have no native surface to draw on (the browser, and the Linux
//! X11/XWayland `dom-canvas` path). The WebView/native shell still owns window
//! lifetime, i18n and the admission policy; it must **not** hold a second CAD
//! session or a copy of the document (ADR 0024 D2).
//!
//! # One session, two carriers (ADR 0024 stage ⑧)
//!
//! The same session implementation runs behind two carriers, selected only by which
//! constructor is used:
//!
//! | entry point | canvas | runs in |
//! |---|---|---|
//! | [`start`] / [`open_bytes`] | the page's `<canvas id=...>` | the page's main thread |
//! | [`start_offscreen`] / [`open_bytes_offscreen`] | an `OffscreenCanvas` transferred with `transferControlToOffscreen()` | a **Worker** |
//!
//! Everything downstream of the carrier — import, scene build, both memory budgets,
//! the chunked upload, the frame path, pan/zoom — is the *same* code
//! ([`session`]): there is exactly one session implementation. The carrier is
//! resolved and created in [`session::surface`], and the status reports it as
//! `target` (`html-canvas` | `offscreen-canvas`) plus `isolation` (`main-thread` |
//! `worker`).
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
//! [`worker_capability`] is how a host learns this *before* choosing: it returns
//! `{ok, isolation, reason, context, webgl2_in_worker}`, so the page can state
//! `isolation=main-thread (no WebGL2 in worker)` on WebKitGTK and run isolated on
//! Chromium. A host must never guess isolation from a feature sniff
//! (`OffscreenCanvas` exists on both engines).
//!
//! ## Single owner, enforced
//!
//! A canvas has exactly one session. [`start_offscreen`] is refused with
//! `step: "isolation"` when it is called outside a Worker, because a page-side
//! `OffscreenCanvas` session would compete with the page's own session for the one
//! document; and the page shell creates **no** session at all on the isolated path
//! (it does not even instantiate the wasm module: the Worker owns it). One canvas,
//! one CAD session, one document.
//!
//! # Exported API (the contract the X11 host lane is wired against)
//!
//! ```text
//! start(canvas_id, css_w, css_h, dpr)                       -> JSON status (async)
//! open_bytes(canvas_id, label, css_w, css_h, dpr, bytes)    -> JSON status (async)
//! start_offscreen(canvas, css_w, css_h, dpr)                -> JSON status (async, Worker only)
//! open_bytes_offscreen(canvas, label, css_w, css_h, dpr, bytes) -> JSON status (async, Worker only)
//! worker_capability()                                       -> JSON capability report
//! resize(css_w, css_h, dpr)                                 -> JSON status
//! zoom_at(factor, x, y)                                     -> JSON status (CSS px)
//! pan(dx, dy)                                               -> JSON status (CSS px)
//! fit_view()                                                -> JSON status
//! status()                                                  -> JSON status
//! document()                                                -> JSON document summary
//! last_import_ms()                                          -> f64 milliseconds
//! shell_status()                                            -> JSON `cad_status` snapshot
//! shell_commands()                                          -> JSON array of executable command names
//! shell_aliases()                                           -> JSON array of `{alias, command}` rows
//! shell_command(name)                                       -> `{"accepted":true}` or Err("unsupported-runtime: cad_command: …")
//! ```
//!
//! `resize`/`zoom_at`/`pan`/`fit_view`/`status`/`document`/`last_import_ms` are
//! **carrier-agnostic**: they act on the one process-global session and never touch
//! a canvas handle, so a Worker calls exactly the same exports as the page. Only the
//! *opening* calls differ, because only they need a canvas.
//!
//! Every status call returns the JSON status object on success and `Err(JsValue)`
//! with a message naming the failing step on failure; the last failure is also
//! kept so [`status()`] reports it. A status carries at least
//! `{ ok, backend, adapter_name, format, target, isolation, physical: [w, h], label,
//! bytes, batches, draws, presented, upload_chunks, peak_batches_per_chunk,
//! heap_after_open, db_retained }`, plus `step`/`error` (and, for a `step` of
//! `webgl2`, the stable `marker` `yacr: unsupported-runtime: webgl2`) on failure.
//!
//! `start` loads the compiled-in synthetic fixture with the stable label
//! `synthetic-four-lines.dwg`; `open_bytes` opens **caller-supplied DWG bytes**
//! (the browser shell reads them with the File API) and replaces a live session.
//!
//! # The shell surface: the shared React chrome without Tauri IPC
//!
//! The desktop hosts serve the chrome (`apps/desktop-ui`) through Tauri commands.
//! A browser page (and an Android WebView) has no IPC, so [`shell`] exposes the
//! **same four wire answers** directly: `shell_status()` is the frozen `cad_status`
//! snapshot (`cad_ui_core::session::CadStatus`) overlaid with this session's **real**
//! facts — its label, resident batch/draw counts, presented-frame count, camera,
//! adapter, installed-document count and its `canFit` (a live session with retained
//! bounds) — serialized with `surfaceMode: "dom-canvas"`, `palettesAvailable: false`
//! and the session's own `canvasSession` descriptor; `shell_commands()`/
//! `shell_aliases()` are the selectors this crate really executes (`FIT`, `ZOOMIN`,
//! `ZOOMOUT`, plus the desktop alias rows that point at them); `shell_command(name)`
//! runs one of those on the live session and answers `{"accepted":true}`, and refuses
//! every other name with the stable `unsupported-runtime: cad_command` prefix. The
//! chrome therefore renders with every capability it does not have turned **off**
//! rather than with fabricated state (see [`shell`] for the per-field table).
//! `OPEN` is not in this crate's vocabulary: the page's file picker owns it
//! (`apps/desktop-ui/src/host/web-adapter.ts`), because a picker is a DOM capability
//! and the bytes are the transport's.
//!
//! # Failure vocabulary (stable)
//!
//! - `import` — the importer rejected the bytes; the message is the importer's
//!   own diagnostic, prefixed with the file label.
//! - `empty-scene` — the file parsed but produced zero draw batches; a blank
//!   canvas is never presented as a successful open.
//! - `webgl2` — no usable WebGL2 context/surface (carries the refusal marker).
//!   This includes the measured WebKitGTK worker case: an `OffscreenCanvas` in a
//!   Worker has no WebGL2 context there.
//! - `isolation` — `start_offscreen`/`open_bytes_offscreen` was called outside a
//!   Worker, i.e. where it would be a second owner of the single session (see
//!   [`session::surface`]).
//! - `canvas-element` — the page has no such element / it is not a `<canvas>`.
//! - `budget` — a **named memory budget** refused the file (see [`budget`]): the
//!   input is larger than [`budget::MAX_INPUT_BYTES`], or the wasm heap after the
//!   scene build exceeds [`budget::MAX_WASM_HEAP_BYTES`]. The message names the
//!   budget, its limit and the measured value, and the status carries a structured
//!   `budget: {name, limit_bytes, measured_bytes}`. The open document stays intact
//!   and navigable — a refusal is never a blank canvas and never a silent
//!   degradation.
//!
//! `open_bytes` runs the geometry load (and both budgets) **before** it touches the
//! session registry, so a bad/unsupported/empty/over-budget file leaves a live
//! session (and its canvas pixels) intact; the failure is reported on the
//! still-open session instead.
//!
//! # Measurement surfaces (evidence, not performance claims)
//!
//! - `last_import_ms()` — the last completed import + scene build + GPU upload,
//!   measured with `web_time::Instant` (never `std::time::Instant`, which panics
//!   on `wasm32-unknown-unknown`). Negative until one open has completed.
//! - `wasm_heap_bytes` — the module's own `WebAssembly.Memory` byte length at the
//!   time of the call. It is the **wasm heap**, not process RSS: there is no
//!   process to measure in a browser tab. It is also a high-water mark (wasm
//!   memory never shrinks), which is why the release work below can only lower it
//!   by keeping the *peak* down.
//! - `heap_after_open` — the same measurement taken when an open completed (scene
//!   uploaded, first frame presented); the number an improvement has to move.
//! - `upload_chunks` / `peak_batches_per_chunk` — how many bounded
//!   `prepare_upload_batches` calls the scene upload used, and the largest number
//!   of batches any single call staged. A whole-scene upload would report
//!   `upload_chunks: 1` with `peak_batches_per_chunk == batches`.
//! - `db_retained` — whether the imported database was kept after the open. It is
//!   `false` (see [`budget::DB_RETAINED`]); if a future capability needs the
//!   database it must flip that and say `true`, never retain silently.
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
//! - **Worker isolation is Chromium-only in the measured set.** Chromium 141 renders
//!   on a worker-side `OffscreenCanvas`; WebKitGTK 2.52.6 returns no WebGL2 context
//!   there, so it runs the main-thread carrier and says so
//!   (`isolation=main-thread (no WebGL2 in worker)`). No other engine is measured;
//!   neither statement may be generalised to Firefox/Safari/Android or to a
//!   different GPU/driver.
//! - **Isolation does not make the wasm heap smaller.** The Worker has its own
//!   `WebAssembly.Memory` (the page's is untouched), and the per-open peak is the
//!   same work. `heap_after_open` therefore measures the Worker's heap when the
//!   isolated carrier is used; it is not a claim about the tab's total memory.
//! - **A shell status surface and the commands the session really owns.** [`shell`]
//!   serves the shared chrome's frozen `cad_status` snapshot from wasm with no host
//!   process behind it (see the section above), and it says exactly what this
//!   carrier cannot do: `canNew`/`canOpen`/`canSave`/`canPlot` are `false`,
//!   `entityCount` is `null` (the database is released), and the command vocabulary
//!   is the three camera selectors it implements (`FIT`, `ZOOMIN`, `ZOOMOUT`).
//!   Pan/zoom/fit are camera-only (a uniform change: no re-parse and no re-upload),
//!   which is exactly what a viewport needs. There is no measurement, annotation,
//!   editing or wire-protocol work here — `shell_command` refuses every other name
//!   instead of faking one. The imported database stays released
//!   ([`budget::DB_RETAINED`] is `false`); the only thing retained for a later
//!   capability is the pair of world points the initial fit used, which is what
//!   `FIT` refits to.
//! - **Chunked upload, one atomic commit.** The scene is staged in bounded
//!   `prepare_upload_batches` chunks (`upload_chunks`,
//!   `peak_batches_per_chunk`) but published by a single `commit_upload`, so the
//!   first presented frame is always the whole drawing. There is deliberately no
//!   "present the uploaded part" path: a partial scene presented as success would
//!   be a fake open. Chunk progress is a console trace
//!   (`yacr: upload-progress:`), not a painted frame.
//! - **A budget refusal is a refusal, not a fallback.** Over
//!   [`budget::MAX_INPUT_BYTES`] (before parsing) or over
//!   [`budget::MAX_WASM_HEAP_BYTES`] (after the build, before the upload) is
//!   `step: "budget"` with the limit, the measurement and the budget name; the
//!   previous document keeps its pixels and stays navigable. Neither check can
//!   *predict* the footprint from the file size (~450× on the measured baseline),
//!   so a file inside the byte cap can still be refused by the heap cap.
//! - **No second session.** One process-global session: `start` refuses to build
//!   a second one, and `open_bytes` disposes the previous renderer/presenter
//!   before creating the replacement. The isolated carrier does not relax this: a
//!   Worker's wasm module has its own session slot, the page never creates one on
//!   that path (it runs no wasm at all), and `start_offscreen` is refused outside a
//!   Worker (`step: "isolation"`).
//! - **No browser run evidence from this lane.** The crate compiles for
//!   `wasm32-unknown-unknown` and the shell under `apps/app-web/web/` is the
//!   minimal page, but nothing here was executed in a browser for this change;
//!   the heap/pixel evidence run belongs to the evidence lane. The `budget`
//!   arithmetic and the chunk planner are unit-tested on the host instead
//!   (`cargo test -p app-web`), which proves the numbers, not the browser.
//! - **What this lane does not lower.** The measured 1.9 GB post-open heap for
//!   `fixtures/complex-test.dwg` is dominated by the import/build peak *inside*
//!   `cad-import-acadrust` + `cad-app::render_scene` (database + display
//!   representation + `SceneDelta`), which this crate may not modify. What is
//!   released here is what this crate owns: the imported database after the build
//!   (`db_retained: false`), each chunk's CPU geometry after its buffers are on
//!   the device, and whole-scene upload staging. Wasm memory never shrinks, so
//!   `heap_after_open` can only improve if the *peak* improves — an honest partial
//!   improvement, not a claim that a 4.2 MB file is now cheap.
//!
//! Native builds cannot present on a DOM canvas, so the session entry points below
//! return the explicit NOT IMPLEMENTED diagnostic instead of a stub success. The
//! **pure** answers keep their honest value there instead of refusing: the
//! capability report ([`worker_capability`]) reports the native limitations it can
//! see, and the shell surface ([`shell`]) reports the no-session status, the empty
//! command vocabulary and the same `unsupported-runtime: cad_command` refusal —
//! none of which needs a browser to be true.

/// The stable reason a native build has no canvas host.
///
/// This is a platform constraint, not a placeholder: presenting into a DOM
/// `<canvas>` requires a wasm module running in the page.
pub const NOT_IMPLEMENTED: &str =
    "web canvas host: NOT IMPLEMENTED on this target — build for wasm32-unknown-unknown";

/// Named memory budgets and the bounded upload-chunk planner.
///
/// Target-independent (no `web-sys`, no `wgpu`, no CAD type) so its arithmetic is
/// unit-tested on the host, not only observable as a dead module in a browser.
pub mod budget;

/// The camera arithmetic of an open and of the `FIT`/`ZOOMIN`/`ZOOMOUT` selectors:
/// the retained drawing bounds, **one** fit rule, and the fixed zoom step about the
/// viewport centre.
///
/// Target-independent on purpose (no DOM, no GPU, no session state), so a refit
/// restoring the framing a zoom step moved away from is asserted by
/// `cargo test -p app-web` rather than only in a browser. See the module docs.
pub mod view;

/// The four platform contracts the browser supplies to the **shared desktop CAD
/// session** (`cad_ui_core::session::CadSession`), plus the bytes-based entry it
/// is built with (ADR 0024 stage ⑧ phase 1).
///
/// Target-independent on purpose (no DOM, no wgpu, no session state): every
/// refusal and every honest absence (no filesystem, non-persistent config,
/// unmapped surface, no font/image package) is asserted by
/// `cargo test -p app-web` on the host, including a session built from the
/// synthetic fixture's **bytes** whose `LINE` command reaches the real router.
///
/// Nothing here is wired to an export yet: the existing `dom-canvas` viewer
/// session ([`session`]) stays the fallback until phase 2 replaces it behind the
/// same exports. See [`platform`] for the capability table and the honest limits.
pub mod platform;

/// Which isolation a browser can actually deliver (`worker_capability()`).
///
/// Target-independent model ([`capability::Capability`]) plus a wasm-only probe, so
/// the decision matrix (Chromium 141 vs WebKitGTK 2.52.6) is unit tested on the
/// host. Read the module docs for the measured capability split.
pub mod capability;

/// The **host-agnostic shell surface**: `shell_status`/`shell_commands`/
/// `shell_aliases`/`shell_command`, i.e. the `cad_status` snapshot and command
/// vocabulary the shared React chrome (`apps/desktop-ui`) polls, served without
/// Tauri IPC.
///
/// Target-independent on purpose: the projection is a pure function of observed
/// session facts ([`shell::status_json`]), so its exact wire shape — including that
/// every capability the carrier does not have is `false` — is asserted by host unit
/// tests rather than only in a browser. The vocabulary is exactly the selectors
/// [`crate::session`] executes (`FIT`, `ZOOMIN`, `ZOOMOUT`) plus the alias rows that
/// point at them; read the module docs for the per-field table.
pub mod shell;

#[cfg(target_arch = "wasm32")]
mod session;

/// **SPIKE (ADR 0024 stage ⑧ phase 2): the real desktop session inside wasm.**
///
/// Additive debug surface only: it builds `cad_ui_core::session::CadSession` from
/// the compiled-in fixture bytes over [`platform`], attaches the page canvas
/// presenter's device to the session's `RenderSessionHandle`, and renders/presents
/// through the existing `CanvasPresenter` blit. The production viewer
/// ([`session`]) is untouched and both sessions coexist in this binary; nothing
/// here is wired to the production exports. See the module docs for the exact
/// seam and for what the spike does **not** prove.
#[cfg(all(target_arch = "wasm32", feature = "debug-session"))]
mod debug_session;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Start the wasm CAD session on the **isolated carrier**: an `OffscreenCanvas`
/// transferred into this Worker.
///
/// This is the worker-side sibling of [`start`]: the same session, the same
/// compiled-in synthetic fixture, the same budgets and upload — only the carrier
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
/// the probe must run in a worker, because a page-side context says nothing about a
/// worker's.
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
/// (`step: "import"`), a file with no drawable geometry (`step: "empty-scene"`) or
/// a file a named memory budget refuses (`step: "budget"`, see [`budget`]) is
/// reported **without** disturbing a live session: the open document keeps its
/// pixels and stays navigable.
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

/// Refit the camera to the drawing extents retained by the open, then present.
///
/// This is the `FIT` selector's implementation: the same fit rule the open used
/// (centre, larger axis ratio, 1.05 margin, validated orthographic projection) run
/// again at the session's **current** CSS size, from the `(min, max)` bounds the
/// import kept. The imported database itself is still released
/// ([`budget::DB_RETAINED`]), so this is not a second query path.
///
/// Fails explicitly when the session retained no bounds (nothing to frame) — never
/// a silent no-op that would look like a refit.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn fit_view() -> Result<String, JsValue> {
    session::fit_view_impl()
}

/// The JSON status object of the current session (or of the last failure).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn status() -> String {
    session::status_impl()
}

/// The JSON summary of the open document:
/// `{ ok, label, batches, draws, presented, bytes, import_ms, wasm_heap_bytes,
/// heap_after_open, upload_chunks, peak_batches_per_chunk, db_retained }`.
///
/// With no live session it is `ok: false` with null/zero values — never a
/// made-up file. `wasm_heap_bytes` is the module's own wasm linear memory (not
/// process RSS) at the time of the call; `heap_after_open` is the same measurement
/// taken when the open completed. `upload_chunks`/`peak_batches_per_chunk` prove
/// the scene upload was staged in bounded chunks; `db_retained` states whether the
/// imported database was kept (it is `false`, see [`budget::DB_RETAINED`]).
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

// ---------------------------------------------------------------------------
// SPIKE (ADR 0024 stage ⑧ phase 2): the real desktop session in wasm
// ---------------------------------------------------------------------------
//
// Additive debug exports. They are **not** the production surface: `start`,
// `open_bytes`, `pan`, `zoom_at`, `resize`, `status`, `document` and the
// `shell_*` calls keep their exact behaviour, and the chrome does not know this
// section exists. Everything below is a spike and is documented as such; the
// production flip is a separate change (see `docs/handoff.md`).
//
// ```text
// debug_session_start(canvas_id, css_w, css_h, dpr) -> JSON status (async)
// debug_session_tick()                              -> JSON status
// debug_session_command(text)                       -> JSON status
// debug_session_pointer(phase, x, y)                -> JSON status
// debug_session_status()                            -> JSON status
// ```

/// **SPIKE.** Start the real desktop CAD session
/// (`cad_ui_core::session::CadSession`, built from the compiled-in fixture
/// **bytes**) on the page canvas `canvas_id`, and attach the canvas presenter's
/// device to the session's render handle.
///
/// This is the phase-2 question of ADR 0024 stage ⑧: can the session the native
/// wayland host drives own a browser `<canvas>` and actually draw? The session is
/// the real one — real command router, real `cad-db` authority, real
/// `RenderSessionHandle` — over the browser's four platform contracts
/// ([`platform`]); the only substitutions are the two seams the carrier forces:
/// the device comes from `CanvasPresenter::device_queue()` and the surface
/// metrics are the canvas' CSS size / `devicePixelRatio`.
///
/// Returns the debug status object (see the module docs). A second start while
/// one is live is refused: one canvas has exactly one presenter.
#[cfg(all(target_arch = "wasm32", feature = "debug-session"))]
#[wasm_bindgen]
pub async fn debug_session_start(
    canvas_id: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
) -> Result<String, JsValue> {
    debug_session::start_impl(canvas_id, css_w, css_h, dpr).await
}

/// **SPIKE.** One frame attempt on the live debug session: the host tick, the
/// staged scene upload (`apply_ready_updates`), the render, and the blit through
/// the existing `CanvasPresenter`.
///
/// A `render` that returns no frame is a real wait (the scene is not committed
/// yet) and is reported as `waited`, never as a presented frame.
#[cfg(all(target_arch = "wasm32", feature = "debug-session"))]
#[wasm_bindgen]
pub fn debug_session_tick() -> Result<String, JsValue> {
    debug_session::tick_impl()
}

/// **SPIKE.** Run one CAD command through the **real** router
/// (`CadSession::command`) and answer the debug status.
#[cfg(all(target_arch = "wasm32", feature = "debug-session"))]
#[wasm_bindgen]
pub fn debug_session_command(text: &str) -> Result<String, JsValue> {
    debug_session::command_impl(text)
}

/// **SPIKE.** Feed one pointer event through the **real** input seam
/// (`CadSession::handle_input` with a `ProbeInput`, the same wire shape the
/// desktop `probe_input` IPC carries).
///
/// `phase` is `down` / `move` / `up`; `x`/`y` are canvas-local logical (CSS)
/// pixels.
#[cfg(all(target_arch = "wasm32", feature = "debug-session"))]
#[wasm_bindgen]
pub fn debug_session_pointer(phase: &str, x: f64, y: f64) -> Result<String, JsValue> {
    debug_session::pointer_impl(phase, x, y)
}

/// **SPIKE.** The live debug session's status object, without advancing it.
#[cfg(all(target_arch = "wasm32", feature = "debug-session"))]
#[wasm_bindgen]
pub fn debug_session_status() -> String {
    debug_session::status_impl()
}

/// Native builds have no page canvas, so the debug session refuses explicitly
/// rather than reporting a session that does not exist.
#[cfg(all(not(target_arch = "wasm32"), feature = "debug-session"))]
pub fn debug_session_start(
    _canvas_id: &str,
    _css_w: f64,
    _css_h: f64,
    _dpr: f64,
) -> Result<String, String> {
    Err(NOT_IMPLEMENTED.to_string())
}

/// Native builds have no debug session to advance (see [`debug_session_start`]).
#[cfg(all(not(target_arch = "wasm32"), feature = "debug-session"))]
pub fn debug_session_tick() -> Result<String, String> {
    Err(NOT_IMPLEMENTED.to_string())
}

/// Native builds have no debug session to command (see [`debug_session_start`]).
#[cfg(all(not(target_arch = "wasm32"), feature = "debug-session"))]
pub fn debug_session_command(_text: &str) -> Result<String, String> {
    Err(NOT_IMPLEMENTED.to_string())
}

/// Native builds have no debug session to feed input (see [`debug_session_start`]).
#[cfg(all(not(target_arch = "wasm32"), feature = "debug-session"))]
pub fn debug_session_pointer(_phase: &str, _x: f64, _y: f64) -> Result<String, String> {
    Err(NOT_IMPLEMENTED.to_string())
}

/// Native builds have no debug session to report (see [`debug_session_start`]).
#[cfg(all(not(target_arch = "wasm32"), feature = "debug-session"))]
pub fn debug_session_status() -> String {
    NOT_IMPLEMENTED.to_string()
}

#[cfg(all(test, not(target_arch = "wasm32"), feature = "debug-session"))]
mod spike_tests {
    //! Contract test for the **native** side of the phase-2 spike surface.
    //!
    //! The spike itself can only run in a browser, so what is testable here is the
    //! honesty rule: on a target with no page canvas every debug export refuses
    //! with the explicit NOT IMPLEMENTED diagnostic. None of them may answer as if
    //! a session existed, and the production exports keep their own answer.

    use super::*;

    #[test]
    fn the_debug_surface_never_fabricates_a_session_off_wasm() {
        let start = debug_session_start("cad-canvas", 800.0, 600.0, 1.0).unwrap_err();
        let tick = debug_session_tick().unwrap_err();
        let command = debug_session_command("LINE").unwrap_err();
        let pointer = debug_session_pointer("down", 1.0, 2.0).unwrap_err();
        for refusal in [&start, &tick, &command, &pointer] {
            assert_eq!(refusal, NOT_IMPLEMENTED, "a native debug call must refuse");
        }
        assert_eq!(debug_session_status(), NOT_IMPLEMENTED);
        // The production surface is unchanged by this spike.
        assert!(status().contains("NOT IMPLEMENTED"));
        assert!(document().contains("NOT IMPLEMENTED"));
    }
}
