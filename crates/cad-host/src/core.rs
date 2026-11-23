//! Shared host core types: options, platform contracts and the state handles
//! owned by [`HostCore`].
use super::*;
use cad_platform::DocumentArchive;
// Same monotonic clock alias as the crate root: std off wasm, `web-time` on wasm.
use crate::Instant;
use cad_app::grips::GripId;

/// Which document a host installs when the session is first built.
///
/// This only describes the window **before** the first open: a configured
/// `--open` drawing replaces the selected starting state in
/// [`HostCore::open_configured_drawing`], whatever the variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartDocument {
    /// No document at all — exactly the state `CLOSE` leaves behind.
    ///
    /// The user sees the start page (NEW/OPEN), not a CAD drawing: the canvas
    /// has no document, the status label is `None`, and the render side was
    /// never given a drawing. Production carriers use this so an unconfigured
    /// launch cannot present a freshly made empty drawing as if the user had
    /// asked for it.
    Empty,
    /// A real, empty `untitled` drawing (layer `0`, no geometry, default
    /// plan-view camera).
    ///
    /// The user sees an empty editable model space instead of the start page.
    Blank,
    /// The synthetic demo drawing (`yacr-demo`) with built-in geometry.
    ///
    /// The user sees demo geometry. Tests, perf baselines and headless fixtures
    /// keep this so their expectations do not move; it is never a production
    /// default.
    Demo,
}

/// Platform-general host options.
///
/// A host-specific options type (for example `LinuxOptions`, which also carries
/// a GPU selection and an acceptance output directory) projects onto this.
#[derive(Debug, Clone)]
pub struct HostOptions {
    /// Explicit `--font NAME=PATH` registrations.
    pub fonts: Vec<(String, PathBuf)>,
    /// Explicit CAD font directory (`--fonts-dir`).
    pub fonts_dir: Option<PathBuf>,
    /// Explicit raster-image directory (`--images-dir`).
    pub images_dir: Option<PathBuf>,
    pub headless: bool,
    /// A drawing to open at startup (`--open`) or the headless input.
    pub drawing: Option<PathBuf>,
    /// The document state the session starts in (see [`StartDocument`]).
    ///
    /// Production carriers use [`StartDocument::Empty`]: opening a session must
    /// not fabricate a document the user did not ask for, so the start page is
    /// what they see. [`StartDocument::Blank`] installs a real empty `untitled`
    /// drawing and [`StartDocument::Demo`] keeps the historical synthetic demo
    /// used by tests and headless fixtures. A configured `drawing` replaces the
    /// starting document in every case.
    pub document_at_start: StartDocument,
    pub size: [f64; 2],
}

/// The platform contracts a [`HostCore`] is built from.
///
/// All are single-threaded (`Rc`): the host core runs on one thread and the
/// traits carry no `Send`/`Sync` requirement.
pub struct HostPlatform {
    pub files: Rc<dyn PlatformFiles>,
    pub config_store: Rc<dyn ConfigStore>,
    pub surface: Rc<dyn SurfaceSource>,
    /// How the host resolves the drawing's shaping fonts and raster images
    /// (desktop sibling package, Android APK assets, browser fetch). The open
    /// funnel is platform-general; resource *resolution* is not.
    pub resources: Rc<dyn HostResources>,
    pub capabilities: HostCapabilities,
}

/// A cancel or an already-localized failure from a platform file interaction.
///
/// The reason string is already localized by the platform; the core shows it
/// verbatim rather than re-wrapping it in an error (which would double-format it).
#[derive(Debug, Clone)]
pub(crate) enum FileAbort {
    Cancelled,
    Failed(String),
}

/// A pending save result drained from the platform, with its purpose.
pub(crate) struct PendingSave {
    pub purpose: SavePurpose,
    pub result: Result<FileTarget, FileAbort>,
}

/// A live **freehand lasso** drag: the STRETCH crossing-window capture in its
/// lasso mode ([`cad_app::StretchWindowStep::LassoTrace`]).
///
/// Unlike [`BoxGesture`] — a rubber band between two corners — a lasso is the
/// free-form ring its drag traces, so the gesture records the sampled **path**.
/// It exists only for the window step of a lasso STRETCH capture: the press that
/// lands there starts it, every move appends a sample, and the release hands the
/// whole path to the capture ([`cad_host::HostCore::set_draw_window_ring`] as a
/// closed ring). A cancel or a multi-touch takeover drops it without committing —
/// nothing is written, no history is opened, and the capture keeps waiting for its
/// window.
#[derive(Debug, Clone)]
pub(crate) struct LassoGesture {
    /// The sampled pointer path in canvas logical pixels, in capture order.
    ///
    /// The first sample is the press and the last is the release; intermediate
    /// samples are decimated while dragging so a long freehand trace stays
    /// bounded without losing the shape (see
    /// [`LassoGesture::push_sample`]).
    pub path: Vec<[f64; 2]>,
}

/// Upper bound on the sampled points one lasso keeps.
///
/// A sample is appended only when the pointer moved at least
/// [`LassoGesture::MIN_SAMPLE_STEP_LOGICAL_PX`], and once the path reaches this
/// bound every second sample is dropped — so a long trace keeps its shape (the
/// ring is still closed and follows the hand) instead of growing without bound or
/// being truncated into a straight chord.
pub(crate) const LASSO_MAX_SAMPLES: usize = 1024;

impl LassoGesture {
    /// The minimum logical-pixel movement between two kept samples.
    ///
    /// Two logical pixels is below the canvas drag threshold and far finer than
    /// any hand movement that matters for a crossing window, so the ring follows
    /// the pointer without recording sub-pixel jitter.
    pub const MIN_SAMPLE_STEP_LOGICAL_PX: f64 = 2.0;

    /// Start a lasso at its press position.
    pub fn new(press: [f64; 2]) -> Self {
        Self { path: vec![press] }
    }

    /// Append one pointer position if it is a new sample.
    ///
    /// A sample closer than [`Self::MIN_SAMPLE_STEP_LOGICAL_PX`] to the last one is
    /// the same hand position and is skipped; a non-finite position is never
    /// stored (the capture would refuse it and the ring would be poisoned). At
    /// [`LASSO_MAX_SAMPLES`] every second sample is dropped, halving the
    /// resolution instead of truncating the ring.
    pub fn push_sample(&mut self, point: [f64; 2]) {
        if !point.iter().all(|value| value.is_finite()) {
            return;
        }
        let Some(last) = self.path.last().copied() else {
            self.path.push(point);
            return;
        };
        let dx = point[0] - last[0];
        let dy = point[1] - last[1];
        if (dx * dx + dy * dy).sqrt() < Self::MIN_SAMPLE_STEP_LOGICAL_PX {
            return;
        }
        if self.path.len() >= LASSO_MAX_SAMPLES {
            let mut decimated: Vec<[f64; 2]> = self.path.iter().copied().step_by(2).collect();
            let last = self.path.last().copied();
            if let Some(last) = last {
                if decimated.last().copied() != Some(last) {
                    decimated.push(last);
                }
            }
            self.path = decimated;
        }
        self.path.push(point);
    }
}

/// A live box-selection drag (ADR 0014 decision B).
///
/// Single-contact only: it is created when a 2D primary-button drag crosses the
/// drag threshold and cleared on release/cancel. It is pure transient gesture
/// state — the host never writes the database, the revision or the history for
/// it; the selection itself is computed by `cad_app`'s `SelectInBox` command on
/// release.
///
/// The same record carries the **`ZOOM WINDOW`** capture: an armed window zoom
/// ([`NavInner::zoom_window_armed`]) turns the drag into a two-corner zoom
/// rectangle instead of a selection box, so both gestures share one arbitration
/// site and one rubber band.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoxGesture {
    /// The press anchor in canvas logical pixels.
    pub anchor: [f64; 2],
    /// The latest pointer position in canvas logical pixels.
    pub current: [f64; 2],
    /// Whether Shift was held on the press (the release merges, i.e. `Add`).
    pub shift: bool,
    /// Whether this drag is a `ZOOM WINDOW` capture rather than a selection box.
    ///
    /// Recorded at gesture start from the armed flag, so the release can never
    /// be re-classified by a later command and the rectangle's crossing/merging
    /// semantics are not applied to a zoom.
    pub window_zoom: bool,
}

/// A live grip drag: one handle of one entity, from its stored position to the
/// current drag target.
///
/// Created by `HostCore::arm_grip_at` when a press lands on a handle of the
/// current selection, updated by `HostCore::track_grip_drag` and consumed by
/// `HostCore::commit_grip_drag` on release. It is pure transient gesture state:
/// nothing is written (no entity update, no revision bump, no undo entry) until
/// the commit, which is one `GripStretch` command — so a cancel is a pure state
/// drop.
///
/// `from` is the handle's **stored** position (all three coordinates, verbatim;
/// it is what the zero-displacement rule compares against) and `to` is the
/// current target. The host keeps `from.z` in `to`, because a plan-view drag
/// never changes an elevation (3D grip editing is NOT IMPLEMENTED).
#[derive(Debug, Clone, Copy)]
pub(crate) struct GripDrag {
    /// The dragged handle (entity, kind, ordinal).
    pub id: GripId,
    /// The handle's stored position at arm time.
    pub from: Point3,
    /// The current drag target; equal to `from` until the pointer moves.
    pub to: Point3,
}

/// Per-instance pointer navigation state.
///
/// The host owns the CAD gesture decisions (touch identity, drag threshold,
/// pan/orbit/box-select); the UI only delivers canvas-local logical positions
/// and the raw pointer phase. Because the typed request carries per-pointer
/// identity but no contact count, `active` is the contact set and its length is
/// the contact count the multi-touch rule keys off.
pub(crate) struct NavInner {
    /// Down-ordered active pointer ids.
    pub active: RefCell<Vec<PointerId>>,
    /// The single contact's down position, for the drag threshold.
    pub anchor: Cell<Option<[f64; 2]>>,
    /// Last position of the single-contact gesture (pan/orbit delta source).
    pub last: Cell<Option<[f64; 2]>>,
    /// Whether the single-contact gesture has crossed the drag threshold.
    pub dragged: Cell<bool>,
    /// Whether the current gesture navigates (pan/orbit) rather than taps.
    pub navigating: Cell<bool>,
    /// Last position of a multi-touch navigate gesture (pan delta source).
    pub nav_last: Cell<Option<[f64; 2]>>,
    /// The button the single contact went down with, for gesture arbitration
    /// (ADR 0014 B.2): a 2D primary drag boxes, a 3D primary drag orbits, and
    /// every other button pans. Recorded on the press because the UI reports no
    /// button change on a move.
    pub button: Cell<Option<PointerButton>>,
    /// The physical device of the single contact, recorded at press time (the
    /// wire `device` field; absent ⇒ [`PointerKind::Mouse`]).
    ///
    /// Gesture arbitration reads it: a `Touch` primary drag pans instead of
    /// drawing a box (one-finger pan), while `Mouse`/`Pen` keep the box rule
    /// (unless the `interaction.leftDragPans` preference is on). Recorded on the
    /// press for the same reason as `button`: a move need not repeat it, and a
    /// moving report could not change the device mid-gesture. Cleared with the
    /// gesture, and never inherited by a surviving contact after a partial
    /// release.
    pub contact_kind: Cell<Option<PointerKind>>,
    /// The modifiers of the press, kept for the gesture. Only `shift` is used
    /// today (ADR 0014 B.4): the release maps it to `SelectionMode::Add`.
    pub press_modifiers: Cell<Option<Modifiers>>,
    /// The live box-selection drag, if a 2D primary drag is in progress.
    pub box_gesture: RefCell<Option<BoxGesture>>,
    /// The live freehand lasso of a STRETCH crossing-window capture, if its
    /// polygon/lasso mode is waiting for the ring.
    ///
    /// Mutually exclusive with [`Self::box_gesture`]: a lasso press belongs to the
    /// active capture, so it never also starts a selection box.
    pub lasso_gesture: RefCell<Option<LassoGesture>>,
    /// The live grip drag, if a press landed on a handle of the selection.
    ///
    /// Armed at press time by the single gesture arbiter (`pointer_down`), so no
    /// later phase has to re-decide between grip, box, pan and orbit. `None`
    /// whenever no grip is under the press.
    pub grip_drag: RefCell<Option<GripDrag>>,
    /// One-shot click-suppression latch.
    ///
    /// A release that ends a grip drag must not also select: the session emits
    /// `Pointer Up` **followed by** `SelectAt` for a click, so `pointer_up` sets
    /// this and `select_at_mode` consumes it. It is cleared by the next press (a
    /// drag that produced no click must never swallow a later one) and by every
    /// cancel/gesture-clear path.
    pub suppress_click: Cell<bool>,
    /// Whether the one-shot `ZOOM WINDOW` two-corner capture is armed.
    ///
    /// Armed by the typed `ZOOM WINDOW` line ([`HostCore::begin_zoom_window`],
    /// which refuses a 3D view by name) and consumed by the next 2D primary drag,
    /// whose release dispatches exactly one `ZoomWindow` payload instead of a box
    /// selection. Dropped by a cancel, a gesture clear (document/metrics/config
    /// change), a multi-touch takeover, a capture or modal pan that owns the
    /// pointer, and any pan/orbit gesture — so a stale arm can never hijack a
    /// later selection. Pure transient gesture state: no command, no database
    /// write, no history entry.
    pub zoom_window_armed: Cell<bool>,
}

impl Default for NavInner {
    fn default() -> Self {
        Self {
            active: RefCell::new(Vec::new()),
            anchor: Cell::new(None),
            last: Cell::new(None),
            dragged: Cell::new(false),
            navigating: Cell::new(false),
            nav_last: Cell::new(None),
            button: Cell::new(None),
            contact_kind: Cell::new(None),
            press_modifiers: Cell::new(None),
            box_gesture: RefCell::new(None),
            lasso_gesture: RefCell::new(None),
            grip_drag: RefCell::new(None),
            suppress_click: Cell::new(false),
            zoom_window_armed: Cell::new(false),
        }
    }
}

/// A resolved-but-not-yet-applied hover, awaiting the hover dwell deadline.
///
/// This models a deliberate deviation from AutoCAD (which has **no** hover
/// timer): the highlight is deferred so a fast pointer sweep never tints
/// anything. It exists only while the session's dwell is non-zero.
#[derive(Debug, Clone)]
pub(crate) struct PendingHover {
    /// The freshly resolved top-ranked ref that is not yet applied.
    pub reference: cad_domain::SelectionRef,
    /// The instant the ref was resolved; the dwell is measured from here.
    pub resolved_at: Instant,
}

/// Transient, entity-level hover state (ADR-0012 §10 S2).
///
/// Hover is **pure session state**: it never touches the database, the history
/// or the selection. It exists only so the host can feed the renderer's
/// `OverlayInputs::hovered` flags input (the same entity-flags mechanism as the
/// selection, with a lighter tint). It is host-local because no CAD authority,
/// command or property view reads it — the application layer and the UI see
/// only the derived status, which carries no hover field.
#[derive(Default)]
pub(crate) struct HoverState {
    /// The current hovered ref (the top-ranked pick), empty when nothing is
    /// hovered. A miss always clears it, so a stale tint can never survive.
    pub hovered: RefCell<cad_app::SelectionSet>,
    /// The resolved ref awaiting the dwell deadline, if any. Applied by
    /// [`HostCore::apply_due_hover`] from the per-frame tick, never by a timer
    /// thread.
    pub pending: RefCell<Option<PendingHover>>,
    /// The logical cursor position of the last executed hover query, so a move
    /// that stays inside the pickbox aperture re-runs no pick. `None` after any
    /// clear/suppression.
    pub last_query: Cell<Option<[f64; 2]>>,
    /// The identity the last hover query was resolved against: the drawing's
    /// [`cad_domain::SceneIdentity`] **plus** the session layer-override
    /// fingerprint ([`cad_app::layers::LayerOverrideSet::fingerprint`]). The
    /// cached `last_query` position is only reusable while both are unchanged,
    /// so a stayed-in-place cursor after an edit/undo — or after a layer
    /// visibility toggle, which changes the pick answer without touching the
    /// database revision — re-resolves instead of keeping a stale tint. `None`
    /// after any clear/suppression.
    pub last_query_identity: Cell<Option<(cad_domain::SceneIdentity, u64)>>,
    /// Number of hover pick queries actually executed. Observable behaviour for
    /// the throttle contract (a counter, not a wall-clock measurement); it is
    /// also what tells the profile log apart from a cached move.
    pub queries: Cell<u64>,
}

/// Registered queue empty-to-nonempty wake callback.
pub(crate) type RequestWaker = Rc<dyn Fn()>;

/// Timing and one-report-per-document bookkeeping for the recovery snapshot
/// (ADR-0037 D12).
///
/// Pure host state: the store itself is a carrier capability
/// ([`HostCore::set_persistence`]) and nothing here is authoritative document
/// content.
#[derive(Default)]
pub(crate) struct RecoveryState {
    /// The last snapshot attempt (successful or not), for the interval.
    ///
    /// `None` while the document is clean: the next edit arms a fresh interval
    /// instead of inheriting one that expired while there was nothing to save.
    pub last_attempt: Option<Instant>,
    /// The key whose snapshot write last failed, so a broken store is reported
    /// once per document instead of once per interval.
    pub failed_key: Option<cad_platform::RecoveryKey>,
    /// Whether `recovery.resources_omitted` was already published for this
    /// document (ADR-0040 D11). The recovery write repeats every interval; the
    /// fact that the snapshot does not pack resources is one fact per document,
    /// so it is reported once instead of on every snapshot.
    pub resources_omitted_reported: bool,
}

/// Monotonic millisecond clock source for the pump time budget.
///
/// Public because a host with **no** default clock (wasm32: `std::time::Instant`
/// panics there, so [`HostCore::new`] installs none) must inject its own platform
/// clock through [`HostCore::set_request_clock`]. Without one, queued work is
/// refused explicitly instead of being given a fabricated elapsed time.
pub type RequestClock = Rc<dyn Fn() -> f64>;

/// Clone handle over the shared host orchestration state.
#[derive(Clone)]
pub struct HostCore {
    pub controller: Rc<RefCell<HostController>>,
    pub render: RenderSessionHandle,
    pub options: HostOptions,
    pub files: Rc<dyn PlatformFiles>,
    pub config_store: Rc<dyn ConfigStore>,
    pub surface: Rc<dyn SurfaceSource>,
    /// The host's font/image installation strategy (see [`HostResources`]).
    pub resources: Rc<dyn HostResources>,
    pub capabilities: HostCapabilities,
    pub(crate) viewer_config: Rc<RefCell<ViewerConfigStore>>,
    pub(crate) canvas_metrics: Rc<Cell<CanvasMetrics>>,
    pub(crate) metrics_revision: Rc<Cell<u64>>,
    pub(crate) full_dirty: Rc<Cell<bool>>,
    pub(crate) navigation_dirty: Rc<Cell<bool>>,
    pub(crate) config_dirty: Rc<Cell<bool>>,
    pub(crate) status_dirty: Rc<Cell<bool>>,
    /// Last published raw backend state, so a lifecycle/actual-backend change
    /// yields a `HostUpdates.backend` delta even when every other dirty flag is
    /// clear. `None` until the first publication.
    pub(crate) last_backend: Rc<RefCell<Option<RenderBackendState>>>,
    /// A picker/read is in flight (analogous to the old `pending_open`).
    pub(crate) open_pending: Rc<Cell<bool>>,
    /// The drawing captured when an open attempt began, so a superseded result
    /// is discarded.
    pub(crate) open_guard: Rc<RefCell<Option<Arc<DrawingDatabase>>>>,
    /// Set by `CancelLoading`; a delivery that arrives afterwards is discarded
    /// rather than installed (mirrors the old `pending_read.cancelled`).
    pub(crate) open_abort: Rc<Cell<bool>>,
    /// The platform target of the open in flight, retained until the async import
    /// publishes so the source-overwrite guard arms against the real handle.
    pub(crate) open_target: Rc<RefCell<Option<FileTarget>>>,
    /// A localized open failure delivered by the platform.
    pub(crate) open_failure: Rc<RefCell<Option<String>>>,
    /// Whether an open-cancel event was delivered this tick.
    pub(crate) open_cancelled: Rc<Cell<bool>>,
    /// Whether the open funnel graded a cancel/failure status this gesture/tick,
    /// so the generic "completed" must not overwrite it.
    pub(crate) open_status_graded: Rc<Cell<bool>>,
    /// A document open is presenting (bytes delivered, import/presentation).
    pub(crate) presenting_open: Rc<Cell<bool>>,
    /// A save dialog is in flight.
    pub(crate) save_pending: Rc<Cell<bool>>,
    /// A completed save interaction, awaiting export.
    pub(crate) pending_save: Rc<RefCell<Option<PendingSave>>>,
    /// The writer a typed `EXPORT <TOKEN>` forced for the save interaction that
    /// is in flight (a connector registry key). `None` for a bare `EXPORT` and
    /// for every other purpose: the chosen target's extension is then the only
    /// dispatch authority. Consumed by the one `SaveExport` poll it belongs to.
    pub(crate) pending_export_format: Rc<RefCell<Option<&'static str>>>,
    /// Path of the drawing being opened (or already open), used to resolve its
    /// sibling raster images. Armed only on a **successful** open.
    pub(crate) drawing_path: Rc<RefCell<Option<PathBuf>>>,
    /// The platform target of the **successfully opened** source drawing.
    ///
    /// Mirrors [`cad_platform::PlatformFiles::set_source`] on the host side so
    /// the save funnel can apply a platform-independent, normalized
    /// path/name equality backstop (the web has no filesystem identity and
    /// Android has only a SAF URI). Cleared by New; replaced only on a
    /// successful open.
    pub(crate) source_target: Rc<RefCell<Option<FileTarget>>>,
    /// The carrier's native document archive (ADR-0037 D1/D16).
    ///
    /// `None` on a carrier that cannot decode/encode `.ymodel`. The open funnel
    /// then refuses a `.ymodel` target **by name** (`unsupported-runtime:
    /// ymodel`) instead of feeding archive bytes to a connector importer, and
    /// the save funnel refuses the native save the same way. Carrier-owned: the
    /// shared layers may not link `cad-store-zip` (architecture gate), so the
    /// implementation arrives as a trait object.
    pub(crate) archive: Rc<RefCell<Option<Arc<dyn DocumentArchive>>>>,
    /// The carrier's recovery-snapshot store (ADR-0037 D12).
    ///
    /// `None` on a carrier with no durable private directory (browser, Android
    /// SAF): the host then writes no snapshot at all rather than inventing a
    /// place for one.
    pub(crate) persistence: Rc<RefCell<Option<Arc<dyn cad_platform::Persistence>>>>,
    /// Timing / one-report-per-document state for the recovery writer.
    pub(crate) recovery: Rc<RefCell<RecoveryState>>,
    /// The monotonic clock the recovery interval is measured against. Separate
    /// from [`Self::hover_now`] only so the two timing contracts can be driven
    /// independently; production installs the same `Instant::now` in both.
    pub(crate) recovery_now: Rc<dyn Fn() -> Instant>,
    /// Last semantic status update.
    pub(crate) last_status: Rc<RefCell<Option<StatusMessage>>>,
    /// Last raster-image loading outcome, surfaced by the diagnostics drawer.
    pub(crate) last_image_report: Rc<RefCell<Option<cad_platform::images::ImageLoadReport>>>,
    /// The normalised image-key set the last `install_images` pass ran for.
    ///
    /// A committed edit that changes this set (an `IMAGEATTACH`, an erase or an
    /// undo) reloads images without a reopen; an unchanged set never decodes
    /// again. Cleared with a document replacement so the next command rechecks.
    pub(crate) last_image_keys: Rc<RefCell<std::collections::BTreeSet<cad_resources::ResourceKey>>>,
    /// Last shaping-font loading outcome (the real per-face report), surfaced by
    /// the read-only Font Manager and the diagnostics drawer.
    ///
    /// Document-scoped: cleared when the authoritative document is replaced.
    pub(crate) last_font_report: Rc<RefCell<Option<cad_platform::fonts::FontLoadReport>>>,
    /// The real load that produced the currently installed font engine.
    ///
    /// Engine-scoped and deliberately kept across a document replacement: the
    /// installed engine is host-wide, so a document that reuses it must still be
    /// able to publish the faces that were actually loaded instead of an empty
    /// report (or invented rows).
    pub(crate) engine_font_report: Rc<RefCell<Option<cad_platform::fonts::FontLoadReport>>>,
    /// The real terminal snapshot of a *synchronous* deliver (no worker): the
    /// opened entity count or the original failure. Composed by
    /// [`Self::import_snapshot`] under the real async snapshot; cleared by a new
    /// open attempt. Reuses the shared `ImportProgressSnapshot` type.
    pub(crate) sync_terminal: Rc<RefCell<Option<ImportProgressSnapshot>>>,
    /// Monotonic Host open-attempt id; bumped by every `begin_open_attempt`.
    /// Scopes the snapshot to the current attempt so an old terminal never masks
    /// a new picker/read/sync deliver.
    pub(crate) open_attempt: Rc<Cell<u64>>,
    /// The attempt id for which a real async job was actually started. The
    /// controller's retained async snapshot is only eligible when this equals the
    /// current [`Self::open_attempt`].
    pub(crate) async_report_attempt: Rc<Cell<Option<u64>>>,
    /// The latest admitted drawing-edit receipt, handed to the render session
    /// by the next post-commit sync. `None` when the last confirm created no
    /// incremental transition (or it was already published).
    pub(crate) pending_edit: Rc<RefCell<Option<cad_app::render_scene::DrawingEditReceipt>>>,
    /// Render-publication batch depth (`0` = publishing normally).
    ///
    /// A caller that applies N inputs in one tick brackets them with
    /// [`Self::begin_render_batch`] / [`Self::end_render_batch`], so every
    /// business step still runs in order but the derived render publication
    /// (overlay clones/fingerprints/content advances/worker submissions) runs
    /// once for the whole batch.
    pub(crate) render_batch: Rc<Cell<u32>>,
    /// Edit receipt stashed while a render batch is open.
    ///
    /// A post-commit sync inside the batch cannot publish immediately, but the
    /// incremental append hint must not be lost: it is retained here and handed
    /// to the single flush at depth 0. `None` after every flush.
    pub(crate) deferred_edit: Rc<RefCell<Option<cad_app::render_scene::DrawingEditReceipt>>>,
    /// Whether the last confirmation supplied an admitted edit receipt to the
    /// render sync. Contract-test observation only; never read by behaviour.
    #[cfg(test)]
    pub(crate) last_edit_receipt_for_test: Rc<Cell<bool>>,
    /// Per-instance pointer navigation state.
    pub(crate) navigation: Rc<NavInner>,
    /// Transient hover state feeding `OverlayInputs::hovered` (ADR-0012 §10 S2).
    pub(crate) hover: Rc<HoverState>,
    /// The last pointer position in canvas-local logical pixels, feeding the
    /// renderer's pointer-cursor overlay (the crosshair + pickbox). `None` until
    /// the pointer is on the surface and after it leaves or cancels; it is pure
    /// session state and never touches the database, the revision or the history.
    pub(crate) cursor_logical: Rc<Cell<Option<[f64; 2]>>>,
    /// The monotonic clock the hover dwell is measured against. Defaults to
    /// `Instant::now` on every target; host tests install a fake to drive the
    /// dwell deterministically (no timer thread exists).
    pub(crate) hover_now: Rc<dyn Fn() -> Instant>,
    /// Diagnostics from reading the config / preference files at startup.
    pub(crate) startup_diagnostics: Rc<RefCell<Vec<String>>>,
    /// The durable renderer backend preference: `Some("auto"|"vulkan"|"dx12")`
    /// once the user chose one, `None` before that (the wire default is `auto`).
    ///
    /// It deliberately lives **outside** the `ViewerConfig` schema: the graphics
    /// backend is renderer state, not viewer configuration, and the closed
    /// allowed-path projection cannot carry it. The value is re-read at startup
    /// and merged back into every preference write (see
    /// `HostCore::persist_preference`), so an unrelated setting write can never
    /// drop it, and the host's presenter construction — which reads the same
    /// preference file — sees exactly what the Options dialog wrote.
    pub(crate) wgpu_backend: Rc<RefCell<Option<String>>>,
    /// The five host-owned appearance/navigation preferences (`theme`,
    /// `navbarVisible`, `navvcubeDisplay`, `viewCubeIn2d`, `viewCubeIn3d`).
    ///
    /// Like [`Self::wgpu_backend`] they are root-level `preference.json` leaves
    /// outside the `ViewerConfig` schema; the host re-emits them on every
    /// preference write (`HostCore::persist_preference`) so an unrelated write
    /// can never drop them. `None` = the user never chose one, which is the
    /// documented default.
    pub(crate) root_preferences: Rc<RefCell<crate::settings_root::RootPreferences>>,
    /// The durable recent-drawings list (root `recentFiles` array).
    ///
    /// User data, not an Options setting: it is outside the `ViewerConfig`
    /// schema and [`crate::HOST_ALLOWED_PATHS_JSON`], seeded from the preference
    /// document at startup and re-emitted on every preference write so an
    /// unrelated setting change cannot drop it. One entry is recorded only after
    /// a **successful** open; a real clear drops the leaf.
    pub(crate) recent_files: Rc<RefCell<Vec<crate::recent::RecentEntry>>>,
    /// The bounded P3 request queue. Admission/identity/ordering live in the
    /// engine; the host only drives it through `dispatch_immediate`.
    pub(crate) queue: Rc<RequestQueue>,
    /// The registered "queue became non-empty" callback. Invoked only after every
    /// queue borrow is released, and only to schedule (never to pump).
    pub(crate) request_waker: Rc<RefCell<Option<RequestWaker>>>,
    /// Independent host document epoch: incremented only when the authoritative
    /// document is actually replaced (never on edits/tools/camera).
    pub(crate) document_epoch: Rc<Cell<u64>>,
    /// The last authoritative drawing seen, compared by real `Arc` identity plus
    /// document identity so an address reuse is not mistaken for "unchanged".
    pub(crate) last_document: Rc<RefCell<Option<Arc<DrawingDatabase>>>>,
    /// Re-entrancy guard: a nested `pump` returns `Busy`.
    pub(crate) pump_active: Rc<Cell<bool>>,
    /// The monotonic millisecond clock used to enforce the pump time budget.
    /// Native hosts install a real `PumpClock`; wasm hosts must inject one (there
    /// is no platform clock), and pump with pending work refuses without a clock.
    pub(crate) request_clock: Rc<RefCell<Option<RequestClock>>>,
    /// Guard while a clock callback is being sampled, so a re-entrant
    /// `pump`/`dispatch`/`submit`/`set_request_clock` sees `Busy`.
    pub(crate) clock_sampling: Rc<Cell<bool>>,
    /// Guard while a public synchronous entry (`command`/`execute`/`dispatch`)
    /// runs, so a platform service callback cannot re-enter and jump the queue.
    pub(crate) sync_active: Rc<Cell<bool>>,
    /// Bounded engine event ring shared with adapters (P4).
    pub(crate) events: Rc<EventRing>,
    /// Per accepted queued request: its reserved trace permit and trace
    /// metadata. Removed when the request finishes (executed) or when its
    /// superseded completion is polled.
    pub(crate) permits: Rc<RefCell<p4::PermitMap>>,
    /// The single actual command trace collected while a request runs; drained
    /// into events by [`p4`] after the business call returns.
    pub(crate) pending_trace: Rc<RefCell<Vec<RawCommandTrace>>>,
    /// Last observed selection refs, for real `SelectionChanged` diffs.
    pub(crate) last_selection: Rc<RefCell<Vec<SelectionRef>>>,
    /// Monotonic selection revision, bumped only on a real refs change.
    pub(crate) selection_revision: Rc<Cell<u64>>,
    /// Last observed tool generation, for real `ToolChanged` diffs.
    pub(crate) last_tool_generation: Rc<Cell<u64>>,
    /// Typed, bounded command echo (observational only; never an input path).
    pub(crate) command_log: Rc<RefCell<crate::command_log::CommandLog>>,
}

/// RAII flag guard: sets the flag on construction and clears it on drop,
/// including on panic/unwind.
pub(crate) struct ActiveGuard<'a> {
    flag: &'a Cell<bool>,
}

impl<'a> ActiveGuard<'a> {
    pub(crate) fn new(flag: &'a Cell<bool>) -> Self {
        flag.set(true);
        Self { flag }
    }
}

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.flag.set(false);
    }
}

impl HostCore {
    /// Open a render-publication batch.
    ///
    /// While a batch is open, [`Self::sync_render_with_edit`] still installs the
    /// interaction tolerance (so every input reads the same pickbox/snap policy)
    /// but defers the actual render publication. Every open must be closed with
    /// [`Self::end_render_batch`]; the depth is counted, so nested brackets are
    /// safe.
    pub fn begin_render_batch(&self) {
        self.render_batch
            .set(self.render_batch.get().saturating_add(1));
    }

    /// Close a render-publication batch and flush at depth 0.
    ///
    /// Unconditional: the depth is always decremented and the outermost close
    /// always publishes (with any stashed edit receipt). The real flush error is
    /// returned rather than hidden, and the stashed receipt is consumed either
    /// way so a later batch cannot replay it.
    pub fn end_render_batch(&self) -> CadResult<()> {
        let depth = self.render_batch.get().saturating_sub(1);
        self.render_batch.set(depth);
        if depth > 0 {
            return Ok(());
        }
        let edit = self.deferred_edit.borrow_mut().take();
        self.sync_render_with_edit(edit.as_ref())
    }

    /// The current render-publication batch depth (`0` outside a batch).
    pub fn render_batch_depth(&self) -> u32 {
        self.render_batch.get()
    }

    /// Take the vendor records copied by the last successful import.
    ///
    /// The owner tick is the only caller: it stages them into the WebView pass.
    /// `Some([])` is a new document with no vendor objects (the caller clears the
    /// previous pass); `None` means no document was published since the last call.
    pub fn take_vendor_records(&self) -> Option<Vec<VendorRecord>> {
        self.controller.borrow_mut().take_vendor_records()
    }

    /// Install the vendor derived-representation snapshot and invalidate the
    /// scene when it changed. Display-only: `cad-db` is never touched.
    pub fn set_vendor_representations(
        &self,
        store: Option<Arc<cad_app::vendor::VendorRepresentationStore>>,
    ) {
        // Display and measurement/query share one snapshot. The application copy
        // is what `AREA OBJECT` and selection properties read; the render session
        // keeps the same `Arc`.
        self.controller
            .borrow_mut()
            .application
            .set_vendor_representations(store.clone());
        self.render.set_vendor_representations(store);
    }
}

/// The initial controller selected by [`HostOptions::document_at_start`].
///
/// Production hosts start with no document at all (the start page); tests and
/// headless fixtures keep the historical synthetic demo, and `Blank` is the
/// explicit "real empty untitled drawing" option.
pub(crate) fn initial_controller(options: &HostOptions) -> CadResult<HostController> {
    match options.document_at_start {
        StartDocument::Empty => HostController::with_no_document(options.size),
        StartDocument::Blank => HostController::with_blank_document(options.size),
        StartDocument::Demo => HostController::with_demo_document(options.size),
    }
}
