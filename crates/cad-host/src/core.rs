//! Shared host core types: options, platform contracts and the state handles
//! owned by [`HostCore`].
use super::*;
// Same monotonic clock alias as the crate root: std off wasm, `web-time` on wasm.
use crate::Instant;
use cad_app::grips::GripId;

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
    /// The modifiers of the press, kept for the gesture. Only `shift` is used
    /// today (ADR 0014 B.4): the release maps it to `SelectionMode::Add`.
    pub press_modifiers: Cell<Option<Modifiers>>,
    /// The live box-selection drag, if a 2D primary drag is in progress.
    pub box_gesture: RefCell<Option<BoxGesture>>,
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
            press_modifiers: Cell::new(None),
            box_gesture: RefCell::new(None),
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
    /// Path of the drawing being opened (or already open), used to resolve its
    /// sibling raster images. Armed only on a **successful** open.
    pub(crate) drawing_path: Rc<RefCell<Option<PathBuf>>>,
    /// Last semantic status update.
    pub(crate) last_status: Rc<RefCell<Option<StatusMessage>>>,
    /// Last raster-image loading outcome, surfaced by the diagnostics drawer.
    pub(crate) last_image_report: Rc<RefCell<Option<cad_platform::images::ImageLoadReport>>>,
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
