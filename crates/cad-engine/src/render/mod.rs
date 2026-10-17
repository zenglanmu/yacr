//! Owner-thread render session state and CPU scene preparation.
use cad_app::layers::LayerOverrideSet;
use cad_app::recovery::{ActiveBackendKind, BackendFailure, BackendOutcome};
use cad_app::render_scene as controller;
use cad_app::render_scene::{CadSceneController, DecodedImageCache, OverlayInputs, SnapHint};
use cad_app::{Camera, MeasurementPreview, ProjectionKind, SelectionSet};
use cad_db::DrawingDatabase;
use cad_domain::{CadError, CadResult, DocumentId, Point3, SpaceId};
/// Re-exported so platform adapters (for example Android) can name a render
/// session's backend preference without depending on `cad-render-wgpu`.
pub use cad_render_wgpu::BackendPreference;
use cad_render_wgpu::{wgpu, ActiveBackend, RenderError};
use cad_representation::{FontEngine, SpaceSelection};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(not(target_arch = "wasm32"))]
mod preparation;
#[cfg(not(target_arch = "wasm32"))]
use preparation::{NativePreparation, PreparationInput};
mod camera;
mod runtime;
pub use camera::{camera2d_from_params, camera3d_from_params, fit_camera, BridgeCamera};
use runtime::CadRenderRuntime;
pub use runtime::{
    FrameBinding, FrameCompleteness, PresentedFrame, RenderLifecycle, RenderStatsObservation,
    RuntimeDiagnostics, UploadDiagnostics, UploadTimingObservation,
};

fn backend_kind(actual: ActiveBackend) -> ActiveBackendKind {
    match actual {
        ActiveBackend::WebGpu => ActiveBackendKind::WebGpu,
        ActiveBackend::WebGl2 => ActiveBackendKind::WebGl2,
        ActiveBackend::Native => ActiveBackendKind::Native,
    }
}

fn preference_choice(preference: BackendPreference) -> cad_app::BackendChoice {
    match preference {
        BackendPreference::Auto => cad_app::BackendChoice::Auto,
        BackendPreference::WebGpu => cad_app::BackendChoice::WebGpu,
        BackendPreference::WebGl2 => cad_app::BackendChoice::WebGl2,
    }
}

pub type IncomingDocument = Rc<RefCell<Option<Arc<DrawingDatabase>>>>;

pub fn space_selection(space: &SpaceId) -> Option<SpaceSelection> {
    match space {
        SpaceId::Model => Some(SpaceSelection::Model),
        SpaceId::Paper(layout) => Some(SpaceSelection::Paper(*layout)),
        SpaceId::Block(_) => None,
    }
}

pub use cad_app::viewer_config::OverlayVisibility;

/// Host scheduling boundary. Implementations may enqueue a redraw or arrange a
/// CPU preparation poll, but the engine never owns or invokes a UI event loop.
pub trait RenderWake {
    fn request_redraw(&self);
    fn request_preparation(&self);
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewSnapshot {
    pub document: DocumentId,
    pub space: SpaceSelection,
    pub camera: Camera,
    pub mode: ProjectionKind,
    pub canvas_size: [f64; 2],
}
impl Default for ViewSnapshot {
    fn default() -> Self {
        Self {
            document: DocumentId(0),
            space: SpaceSelection::Model,
            camera: Camera::top_view_2d(),
            mode: ProjectionKind::TwoD,
            canvas_size: [1280.0, 800.0],
        }
    }
}

/// Honest label for the deferred camera-only reference-grid refresh (never a fake
/// final-commit claim).
fn grid_refresh_label(pending: bool) -> &'static str {
    if pending {
        "grid-refresh-pending"
    } else {
        "grid-refresh-idle"
    }
}

fn viewport_world_bounds(snapshot: &ViewSnapshot) -> Option<(Point3, Point3)> {
    if snapshot.mode != ProjectionKind::TwoD
        || !snapshot
            .canvas_size
            .iter()
            .all(|v| v.is_finite() && *v > 0.0)
    {
        return None;
    }
    let params = snapshot.camera.camera2d_params().ok()?;
    if !params.world_per_px.is_finite() || params.world_per_px <= 0.0 {
        return None;
    }
    let half_width = snapshot.canvas_size[0] * params.world_per_px * 0.5;
    let half_height = snapshot.canvas_size[1] * params.world_per_px * 0.5;
    let bounds = (
        Point3 {
            x: params.center.x - half_width,
            y: params.center.y - half_height,
            z: params.center.z,
        },
        Point3 {
            x: params.center.x + half_width,
            y: params.center.y + half_height,
            z: params.center.z,
        },
    );
    [bounds.0.x, bounds.0.y, bounds.1.x, bounds.1.y]
        .iter()
        .all(|v| v.is_finite())
        .then_some(bounds)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BaseInputIdentity {
    drawing: usize,
    /// Authoritative database revision at the moment the identity was taken.
    ///
    /// Mixed in as an O(1) discriminator on top of the pointer so a base built
    /// from a different revision is a different base. Database edits go through
    /// the transaction/copy-on-write path (revision-bumping); the pointer alone
    /// is not treated as sufficient identity.
    revision: Option<cad_domain::Revision>,
    document: DocumentId,
    space: SpaceSelection,
    fonts_generation: u64,
    images_generation: u64,
    layers: LayerOverrideSet,
}

impl BaseInputIdentity {
    fn from_state(state: &State) -> Self {
        let incoming = state.incoming.borrow();
        let drawing = incoming.as_ref();
        Self {
            drawing: drawing.map_or(0, |drawing| Arc::as_ptr(drawing) as usize),
            revision: drawing.map(|drawing| drawing.revision()),
            document: state.view.document,
            space: state.view.space,
            fonts_generation: state.fonts_generation,
            images_generation: state.images_generation,
            layers: state.layers.clone(),
        }
    }
}

/// Default camera-settle timeout for the deferred reference-grid refresh
/// (camera-only pan/zoom/rotate). See [`State::grid_refresh_pending`].
const CAMERA_SETTLE_TIMEOUT: Duration = Duration::from_millis(250);

struct State {
    incoming: IncomingDocument,
    controller: CadSceneController,
    view: ViewSnapshot,
    fonts: Option<Arc<FontEngine>>,
    images: Option<Arc<DecodedImageCache>>,
    fonts_generation: u64,
    images_generation: u64,
    layers: LayerOverrideSet,
    overlays: OverlayInputs,
    overlay_revision: u64,
    /// Monotonic revision of the **scene identity**. Advances only when an input
    /// that changes the published scene advances (drawing, document, space, layer
    /// overrides, font/image resources, or overlay batches). Camera-only changes
    /// (pan/zoom, mode, canvas size) deliberately do **not** advance it: the
    /// published scene stays valid because every overlay batch is world-space
    /// geometry that renders correctly under any camera.
    input_revision: u64,
    /// Monotonic revision of the **full preparation input**, including the
    /// camera-derived `viewport_bounds`. It is the async preparation token and
    /// publication gate: it advances on every input change (scene-identity
    /// changes *and* camera-only viewport-bounds changes), so a pan still
    /// schedules a background re-prepare of the reference grid without
    /// invalidating the current ready scene.
    content_revision: u64,
    /// Monotonic revision of the authoritative view (camera/space). Advances only
    /// when the view actually changes, and tags every presented frame.
    view_revision: u64,
    /// Content revision of the last published scene; guards against publishing
    /// the same preparation input twice.
    prepared_token: Option<u64>,
    /// Scene-identity revision (`input_revision`) at the moment the last scene was
    /// published. The published scene is current iff this equals the live
    /// `input_revision`, which is what carves camera-only changes out of scene
    /// invalidation.
    prepared_identity: Option<u64>,
    observed_drawing: usize,
    scene_publication_revision: u64,
    base_publication_revision: u64,
    published_base_identity: Option<BaseInputIdentity>,
    runtime: CadRenderRuntime,
    preference: BackendPreference,
    redraw_requests: u64,
    stopped: bool,
    view_diagnostic: Option<String>,
    /// A camera-only (pan/zoom/rotate) change moved the viewport world bounds and
    /// its reference-grid re-prepare is **deferred** until the navigation settles:
    /// during a gesture the published scene keeps its old grid extent instead of
    /// re-preparing on every step. Cleared only when the content revision advances
    /// (a settle commit, or any immediate/non-camera input), so it can never report
    /// a fake final commit.
    grid_refresh_pending: bool,
    /// When the deferred camera-only refresh was last (re)armed.
    last_camera_change: Option<Instant>,
    /// Camera-settle timeout. `Duration::ZERO` disables the deferral entirely
    /// (today's immediate behaviour); it is also the test kill-switch.
    settle_timeout: Duration,
    #[cfg(not(target_arch = "wasm32"))]
    preparation: NativePreparation,
}
impl Default for State {
    fn default() -> Self {
        Self {
            incoming: Rc::new(RefCell::new(None)),
            controller: CadSceneController::default(),
            view: ViewSnapshot::default(),
            fonts: None,
            images: None,
            fonts_generation: 0,
            images_generation: 0,
            layers: LayerOverrideSet::new(),
            overlays: OverlayInputs::default(),
            overlay_revision: 0,
            input_revision: 0,
            content_revision: 0,
            view_revision: 0,
            prepared_token: None,
            prepared_identity: None,
            observed_drawing: 0,
            scene_publication_revision: 0,
            base_publication_revision: 0,
            published_base_identity: None,
            runtime: CadRenderRuntime::default(),
            preference: BackendPreference::Auto,
            redraw_requests: 0,
            stopped: false,
            view_diagnostic: None,
            grid_refresh_pending: false,
            last_camera_change: None,
            settle_timeout: CAMERA_SETTLE_TIMEOUT,
            #[cfg(not(target_arch = "wasm32"))]
            preparation: NativePreparation::default(),
        }
    }
}

impl State {
    fn advance_input(&mut self) {
        self.input_revision = self
            .input_revision
            .checked_add(1)
            .expect("render input revision exhausted");
        self.advance_content();
    }

    /// Advance only the preparation content revision (camera-derived reference
    /// geometry). Never invalidates the published scene.
    ///
    /// Advancing the content revision is exactly "something covering the current
    /// bounds is being advanced", so it also clears a deferred camera-only refresh:
    /// `grid_refresh_pending` is only ever true while the current bounds are *not*
    /// yet re-prepared.
    fn advance_content(&mut self) {
        self.content_revision = self
            .content_revision
            .checked_add(1)
            .expect("render content revision exhausted");
        self.clear_camera_settle();
    }

    /// Arm/refresh the deferred camera-only reference-grid refresh.
    fn arm_camera_settle(&mut self, now: Instant) {
        self.grid_refresh_pending = true;
        self.last_camera_change = Some(now);
    }

    /// Forget a deferred camera-only refresh (it was covered by an advance, or a
    /// non-transient input superseded it).
    fn clear_camera_settle(&mut self) {
        self.grid_refresh_pending = false;
        self.last_camera_change = None;
    }

    /// Whether the camera-only deferral is active: never on wasm (there is no
    /// async pump to settle it) and never with a zero timeout (kill-switch).
    fn camera_settle_enabled(&self) -> bool {
        cfg!(not(target_arch = "wasm32")) && !self.settle_timeout.is_zero()
    }

    /// One `Instant` compare per pump: settle a deferred camera-only refresh once
    /// the navigation has been quiet for `settle_timeout`. Returns whether the
    /// content revision advanced.
    fn settle_camera_if_due(&mut self, now: Instant) -> bool {
        if !self.grid_refresh_pending {
            return false;
        }
        let due = match self.last_camera_change {
            Some(last) => now.saturating_duration_since(last) >= self.settle_timeout,
            None => true,
        };
        if due {
            self.advance_content();
            true
        } else {
            false
        }
    }

    fn observe_drawing(&mut self) {
        let drawing = self
            .incoming
            .borrow()
            .as_ref()
            .map_or(0, |drawing| Arc::as_ptr(drawing) as usize);
        if drawing != self.observed_drawing {
            self.observed_drawing = drawing;
            self.advance_input();
        }
    }
}

fn publish_controller(state: &mut State, mut candidate: CadSceneController, token: u64) -> bool {
    if token != state.content_revision || state.prepared_token == Some(token) {
        return false;
    }
    let Some(mut ready) = candidate.ready.clone() else {
        return false;
    };
    let base_identity = BaseInputIdentity::from_state(state);
    if state.published_base_identity.as_ref() != Some(&base_identity) {
        state.base_publication_revision = state
            .base_publication_revision
            .checked_add(1)
            .expect("base publication revision exhausted");
        state.published_base_identity = Some(base_identity);
    }
    state.scene_publication_revision = state
        .scene_publication_revision
        .checked_add(1)
        .expect("scene publication revision exhausted");
    ready.revision = state.scene_publication_revision;
    ready.base_revision = state.base_publication_revision;
    candidate.ready = Some(ready);
    state.controller = candidate;
    state.prepared_token = Some(token);
    // Record the scene identity this publication was built from. A later
    // camera-only change advances `content_revision` but not `input_revision`, so
    // the scene stays current while the refinement republishes.
    state.prepared_identity = Some(state.input_revision);
    state.runtime.dirty.invalidate();
    true
}

fn current_ready_scene(state: &State) -> Option<&controller::PreparedScene> {
    (!state.stopped && state.prepared_identity == Some(state.input_revision))
        .then_some(())
        .and(state.controller.ready.as_ref())
}

/// Why the render gate did (not) produce a CAD frame. Diagnostics only — it is
/// derived from the same read-only facts `RenderSessionHandle::render` uses and
/// never relaxes the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderGateReason {
    /// No renderer attached (device detached / never attached).
    Detached,
    /// Terminal backend failure/loss.
    Failed,
    /// CPU scene preparation has not published the current scene yet (the
    /// `prepared_identity`/`content_revision` path is still busy or has not run).
    Preparing,
    /// The current scene is published but its **base** upload has not committed.
    UploadingBase,
    /// The current scene is published but an optional overlay-suffix upload has
    /// not committed.
    UploadingOverlay,
    /// The current scene is published and no upload is pending, but the GPU base
    /// resident for its revision does not match the live base identity (for
    /// example after a device rebuild) — the gate cannot draw the current scene.
    BaseIdentityMismatch,
    /// Gate met: the current scene and its base are GPU-committed.
    Ready,
}

/// The facts the gate decision is made from (pure, so it is unit-testable
/// without a GPU).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GateFacts {
    detached: bool,
    ready_lifecycle: bool,
    scene_current: bool,
    committed: bool,
    /// `Some(base_changed)` when an upload is in flight.
    pending: Option<bool>,
    base_resident: bool,
}

fn classify_gate(facts: GateFacts) -> RenderGateReason {
    if facts.detached {
        return RenderGateReason::Detached;
    }
    if !facts.ready_lifecycle {
        return RenderGateReason::Failed;
    }
    if !facts.scene_current {
        return RenderGateReason::Preparing;
    }
    if facts.committed {
        return RenderGateReason::Ready;
    }
    match facts.pending {
        Some(true) => RenderGateReason::UploadingBase,
        Some(false) => RenderGateReason::UploadingOverlay,
        None if facts.base_resident => RenderGateReason::Preparing,
        None => RenderGateReason::BaseIdentityMismatch,
    }
}

/// Read-only structured snapshot of the render session's gate/waiting state.
///
/// Projected from the same state `render` reads; producing it never advances a
/// revision, an upload or the view, and never forces engine work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderDiagnostics {
    /// The gate reason the next `render` would see.
    pub gate: RenderGateReason,
    /// `"detached"` / `"ready"` / `"lost"` / `"failed"`.
    pub lifecycle: &'static str,
    /// Monotonic scene-identity revision (advances on identity changes only).
    pub input_revision: u64,
    /// Monotonic preparation token: the **most recently requested** input.
    pub requested_token: u64,
    /// Monotonic authoritative view revision.
    pub view_revision: u64,
    /// Content revision of the last **published** scene.
    pub prepared_token: Option<u64>,
    /// Scene-identity revision the published scene was built from.
    pub prepared_identity: Option<u64>,
    /// Published scene revision, if a current ready scene exists.
    pub ready_revision: Option<u64>,
    /// Published scene's base revision.
    pub ready_base_revision: Option<u64>,
    /// Cumulative scene publication revision.
    pub scene_publication_revision: u64,
    /// Cumulative base publication revision.
    pub base_publication_revision: u64,
    /// Whether async CPU preparation is busy.
    pub preparation_busy: bool,
    /// Whether async CPU preparation exists on this target (false on wasm).
    pub preparation_available: bool,
    /// Last real scene/runtime diagnostic (never a converted success).
    pub result_status: Option<String>,
    /// Whether a camera-only (pan/zoom/rotate) reference-grid refresh is deferred
    /// until the navigation settles. `true` only while the current bounds have not
    /// been re-prepared yet, so it can never claim a fake final commit.
    pub grid_refresh_pending: bool,
    /// Human-readable companion for [`Self::grid_refresh_pending`].
    pub grid_refresh: &'static str,
    /// Runtime upload/render bookkeeping.
    pub runtime: RuntimeDiagnostics,
}

impl RenderSessionHandle {
    /// Read-only render-gate diagnostics (see [`RenderDiagnostics`]).
    ///
    /// Deliberately does **not** call `observe_drawing` (which can advance the
    /// input revision): a telemetry read must not mutate readiness.
    pub fn diagnostics(&self) -> RenderDiagnostics {
        let state = self.0.borrow();
        let scene = current_ready_scene(&state);
        let committed = scene.is_some_and(|scene| {
            state
                .runtime
                .scene_committed(scene.revision, scene.base_revision)
        });
        let facts = GateFacts {
            detached: matches!(state.runtime.lifecycle, RenderLifecycle::Detached),
            ready_lifecycle: matches!(state.runtime.lifecycle, RenderLifecycle::Ready { .. }),
            scene_current: scene.is_some(),
            committed,
            pending: state.runtime.pending_upload_kind(),
            base_resident: scene
                .is_some_and(|scene| state.runtime.base_fully_resident(scene.base_revision)),
        };
        #[cfg(not(target_arch = "wasm32"))]
        let preparation_busy = state.preparation.busy();
        #[cfg(target_arch = "wasm32")]
        let preparation_busy = false;
        let runtime = state.runtime.diagnostics();
        RenderDiagnostics {
            gate: classify_gate(facts),
            lifecycle: runtime.lifecycle,
            input_revision: state.input_revision,
            requested_token: state.content_revision,
            view_revision: state.view_revision,
            prepared_token: state.prepared_token,
            prepared_identity: state.prepared_identity,
            ready_revision: scene.map(|scene| scene.revision),
            ready_base_revision: scene.map(|scene| scene.base_revision),
            scene_publication_revision: state.scene_publication_revision,
            base_publication_revision: state.base_publication_revision,
            preparation_busy,
            preparation_available: !cfg!(target_arch = "wasm32"),
            result_status: state
                .controller
                .diagnostic
                .clone()
                .or_else(|| state.runtime.diagnostic.clone()),
            grid_refresh_pending: state.grid_refresh_pending,
            grid_refresh: grid_refresh_label(state.grid_refresh_pending),
            runtime,
        }
    }
}

/// Narrow qualifier for the 2D navigation fallback.
///
/// It holds only when the *base* drawing is provably unchanged and fully
/// resident while preparation lags: the view is a valid 2D camera, the latest
/// base input identity equals the last published one (so no new database, space,
/// layer, font, image or resource identity is pending), and the GPU holds the
/// latest published base with no pending base replacement. Everything that could
/// make the resident geometry stale (new/unuploaded base, device change) fails
/// this predicate, so the fallback can only ever draw the authoritative base.
fn base_only_navigation_eligible(state: &State) -> bool {
    if state.stopped {
        return false;
    }
    if state.view.mode != ProjectionKind::TwoD || state.view.camera.camera2d_params().is_err() {
        return false;
    }
    let latest = BaseInputIdentity::from_state(state);
    if state.published_base_identity.as_ref() != Some(&latest) {
        return false;
    }
    state
        .runtime
        .base_fully_resident(state.base_publication_revision)
}

#[cfg(not(target_arch = "wasm32"))]
fn preparation_input(state: &State) -> PreparationInput {
    PreparationInput {
        token: state.content_revision,
        drawing: state.incoming.borrow().clone(),
        document: state.view.document,
        space: state.view.space,
        fonts: state.fonts.clone(),
        images: state.images.clone(),
        fonts_generation: state.fonts_generation,
        images_generation: state.images_generation,
        layers: state.layers.clone(),
        overlays: state.overlays.clone(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn prepare_synchronously(state: &mut State) -> CadResult<()> {
    state.observe_drawing();
    let input = preparation_input(state);
    let mut candidate = state.controller.clone();
    candidate.prepare_shared_with_overlays_and_images(
        input.drawing.clone(),
        input.document,
        input.fonts.clone(),
        &input.layers,
        input.space,
        &input.overlays,
        input.images.clone(),
    )?;
    publish_controller(state, candidate, input.token);
    state.preparation.mark_synchronous(&input);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn prepare_synchronously(state: &mut State) -> CadResult<()> {
    state.observe_drawing();
    let token = state.content_revision;
    let mut candidate = state.controller.clone();
    candidate.prepare_shared_with_overlays_and_images(
        state.incoming.borrow().clone(),
        state.view.document,
        state.fonts.clone(),
        &state.layers,
        state.view.space,
        &state.overlays,
        state.images.clone(),
    )?;
    publish_controller(state, candidate, token);
    Ok(())
}

/// Cloneable, deliberately single-owner-thread handle shared by the engine and
/// its UI adapter. All authoritative CAD/render-session mirrors live here.
#[derive(Clone, Default)]
pub struct RenderSessionHandle(Rc<RefCell<State>>);

impl RenderSessionHandle {
    /// Whether both handles refer to the exact same owner-thread session state.
    pub fn same_session(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub fn new(preference: BackendPreference) -> Self {
        let state = State {
            preference,
            ..State::default()
        };
        Self(Rc::new(RefCell::new(state)))
    }

    pub fn with_incoming(incoming: IncomingDocument, preference: BackendPreference) -> Self {
        let session = Self::new(preference);
        session.0.borrow_mut().incoming = incoming;
        session
    }

    pub fn sync_drawing(&self, drawing: Option<Arc<DrawingDatabase>>) -> bool {
        let mut state = self.0.borrow_mut();
        let mut incoming = state.incoming.borrow_mut();
        let changed = match (incoming.as_ref(), &drawing) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        if changed {
            *incoming = drawing;
        }
        drop(incoming);
        if changed {
            state.observe_drawing();
        }
        changed
    }

    pub fn validate_snapshot(&self, view: &ViewSnapshot) -> CadResult<()> {
        view.camera.validate()?;
        if view.mode == ProjectionKind::TwoD {
            view.camera.camera2d_params()?;
        }
        if let Some(doc) = self.0.borrow().incoming.borrow().as_ref() {
            cad_app::validate_space(doc, view.space)?;
        }
        Ok(())
    }

    pub fn apply_view_snapshot(&self, view: ViewSnapshot) -> CadResult<bool> {
        self.validate_snapshot(&view)?;
        let mut state = self.0.borrow_mut();
        let previous_input_revision = state.input_revision;
        state.observe_drawing();
        if state.view == view {
            return Ok(state.input_revision != previous_input_revision);
        }
        // A document/space switch changes the authoritative scene identity and
        // must invalidate the published scene. A camera-only change (pan/zoom,
        // rotation, mode, canvas size) must not: every overlay batch is world-space
        // geometry that renders correctly under any camera, so only
        // `view_revision` advances and the reference-grid bounds are re-scoped.
        let identity_changed =
            view.document != state.view.document || view.space != state.view.space;
        // Deferral is admitted **only** when the two snapshots differ solely in
        // camera fields (center/zoom/rotation): a canvas-size, mode, document or
        // space change keeps today's immediate path.
        let camera_only = state.view.document == view.document
            && state.view.space == view.space
            && state.view.mode == view.mode
            && state.view.canvas_size == view.canvas_size
            && state.view.camera != view.camera;
        state.view = view;
        state.view_revision = state
            .view_revision
            .checked_add(1)
            .expect("view revision exhausted");
        let bounds = viewport_world_bounds(&state.view);
        let bounds_changed = state.overlays.viewport_bounds != bounds;
        if bounds_changed {
            state.overlays.viewport_bounds = bounds;
            state.overlay_revision += 1;
        }
        if identity_changed {
            // Never defer across a scene-identity change: this clears any pending
            // camera refresh as well (`advance_input` -> `advance_content`).
            state.advance_input();
        } else if bounds_changed {
            if camera_only && state.camera_settle_enabled() {
                // Defer: the published scene keeps the old grid extent and one
                // refresh is committed when the gesture settles.
                state.arm_camera_settle(Instant::now());
            } else {
                // Today's immediate path (non-camera bounds change, or the
                // zero-timeout kill-switch). Clears any pending refresh.
                state.advance_content();
            }
        } else if !camera_only {
            // A non-camera view change that did not move the bounds schedules
            // nothing by itself. It must NOT drop a pending refresh for the
            // **current** bounds (reachable through a fit-on-resize, where canvas
            // size and camera change together while the world extent stays
            // identical): cover those bounds with one content advance, which also
            // clears the pending flag. A genuine immediate path never lands here.
            if state.grid_refresh_pending {
                state.advance_content();
            }
        }
        state.runtime.dirty.invalidate();
        Ok(true)
    }

    pub fn set_fonts(&self, fonts: Option<Arc<FontEngine>>) -> bool {
        let mut state = self.0.borrow_mut();
        if state.fonts.as_ref().map(Arc::as_ptr) != fonts.as_ref().map(Arc::as_ptr) {
            state.fonts = fonts;
            state.fonts_generation = state
                .fonts_generation
                .checked_add(1)
                .expect("font resource generation exhausted");
            state.controller.fonts_changed();
            state.advance_input();
            return true;
        }
        false
    }

    pub fn set_images(&self, images: Option<Arc<DecodedImageCache>>) -> bool {
        let mut state = self.0.borrow_mut();
        if state.images.as_ref().map(Arc::as_ptr) != images.as_ref().map(Arc::as_ptr) {
            state.images = images;
            state.images_generation = state
                .images_generation
                .checked_add(1)
                .expect("image resource generation exhausted");
            state.advance_input();
            return true;
        }
        false
    }

    pub fn set_layers(&self, layers: LayerOverrideSet) -> bool {
        let mut state = self.0.borrow_mut();
        if state.layers == layers {
            return false;
        }
        state.layers = layers;
        state.advance_input();
        true
    }
    pub fn set_overlays(&self, overlays: OverlayInputs) -> bool {
        let mut state = self.0.borrow_mut();
        if state.overlays == overlays {
            return false;
        }
        // A change confined to the camera-derived `viewport_bounds` only re-scopes
        // the reference grid: it must schedule a background re-prepare but never
        // invalidate the published scene. Any other overlay input (visibility,
        // selection, previews, snap hints) changes real overlay batches and keeps
        // invalidating exactly as before.
        let bounds_only = {
            let mut probe = state.overlays.clone();
            probe.viewport_bounds = overlays.viewport_bounds;
            probe == overlays
        };
        state.overlays = overlays;
        state.overlay_revision += 1;
        if bounds_only {
            state.advance_content();
        } else {
            state.advance_input();
        }
        true
    }

    /// CPU-only scene preparation; safe to drive without constructing a UI.
    pub fn prepare(&self) -> CadResult<()> {
        let mut state = self.0.borrow_mut();
        if state.stopped {
            return Err(CadError::InvalidInput("render session stopped".into()));
        }
        prepare_synchronously(&mut state)
    }

    /// Register the newest immutable preparation input before a UI frame. The
    /// returned bool indicates that a completion/poll should be scheduled.
    pub fn request_preparation(&self) -> CadResult<bool> {
        let mut state = self.0.borrow_mut();
        if state.stopped {
            return Ok(false);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            state.observe_drawing();
            state.settle_camera_if_due(Instant::now());
            let input = preparation_input(&state);
            match state.preparation.update(input) {
                Ok(Some((token, controller))) => {
                    publish_controller(&mut state, controller, token);
                }
                Ok(None) => {}
                Err(error) => state.controller.diagnostic = Some(error.to_string()),
            }
            Ok(true)
        }
        #[cfg(target_arch = "wasm32")]
        {
            prepare_synchronously(&mut state)?;
            Ok(false)
        }
    }

    /// Poll asynchronous native preparation and publish only the latest input.
    pub fn poll_preparation(&self) -> CadResult<bool> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut state = self.0.borrow_mut();
            if state.stopped {
                return Ok(false);
            }
            state.observe_drawing();
            state.settle_camera_if_due(Instant::now());
            let input = preparation_input(&state);
            match state.preparation.update(input) {
                Ok(Some((token, controller))) => {
                    publish_controller(&mut state, controller, token);
                }
                Ok(None) => {}
                Err(error) => state.controller.diagnostic = Some(error.to_string()),
            }
            Ok(state.preparation.busy())
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(false)
        }
    }

    pub fn incoming_document(&self) -> IncomingDocument {
        self.0.borrow().incoming.clone()
    }
    pub fn view_snapshot(&self) -> ViewSnapshot {
        self.0.borrow().view.clone()
    }
    pub fn camera(&self) -> Camera {
        self.0.borrow().view.camera
    }
    pub fn space(&self) -> SpaceSelection {
        self.0.borrow().view.space
    }
    pub fn view_mode(&self) -> ProjectionKind {
        self.0.borrow().view.mode
    }
    pub fn view_diagnostic(&self) -> Option<String> {
        let state = self.0.borrow();
        state
            .view_diagnostic
            .clone()
            .or_else(|| state.controller.diagnostic.clone())
            .or_else(|| state.runtime.diagnostic.clone())
            .or_else(|| {
                state
                    .controller
                    .ready
                    .as_ref()
                    .and_then(|s| s.highlight_diagnostics.first().map(|d| d.message.clone()))
            })
    }
    pub fn scene_error(&self) -> Option<String> {
        let state = self.0.borrow();
        state
            .controller
            .diagnostic
            .clone()
            .or_else(|| state.runtime.diagnostic.clone())
    }
    pub fn set_view_diagnostic(&self, value: Option<String>) {
        self.0.borrow_mut().view_diagnostic = value;
    }
    pub fn set_runtime_diagnostic(&self, value: Option<String>) {
        self.0.borrow_mut().runtime.diagnostic = value;
    }
    pub fn fonts(&self) -> Option<Arc<FontEngine>> {
        self.0.borrow().fonts.clone()
    }
    pub fn overlays(&self) -> OverlayInputs {
        self.0.borrow().overlays.clone()
    }
    pub fn overlay_revision(&self) -> u64 {
        self.0.borrow().overlay_revision
    }
    pub fn cad_frame_dirty(&self) -> bool {
        self.0.borrow().runtime.dirty.dirty()
    }
    pub fn current_drawing_presented(&self) -> bool {
        let mut state = self.0.borrow_mut();
        state.observe_drawing();
        if !Self::preparation_current_state(&state) {
            return false;
        }
        // A base-only navigation frame is never the presented current drawing.
        if state.runtime.overlays_suppressed() {
            return false;
        }
        state.runtime.presented_revision.is_some_and(|revision| {
            state
                .controller
                .ready
                .as_ref()
                .is_some_and(|scene| scene.revision == revision)
        }) && !state.runtime.uploading()
    }
    fn preparation_current_state(state: &State) -> bool {
        current_ready_scene(state).is_some()
    }
    pub fn preparation_current(&self) -> bool {
        let mut state = self.0.borrow_mut();
        state.observe_drawing();
        current_ready_scene(&state).is_some()
    }
    pub fn input_revision(&self) -> u64 {
        self.0.borrow().input_revision
    }
    /// Monotonic revision of the authoritative view (camera/space). Advances only
    /// on an actual view change; presented frames are tagged with it.
    pub fn view_revision(&self) -> u64 {
        self.0.borrow().view_revision
    }
    /// Whether the 2D navigation fallback is currently withholding overlays.
    pub fn overlays_suppressed(&self) -> bool {
        self.0.borrow().runtime.overlays_suppressed()
    }
    /// Whether a camera-only (pan/zoom/rotate) reference-grid refresh is deferred
    /// until the navigation settles. Honest: it is `true` only while the current
    /// bounds have **not** been re-prepared yet, and it clears exactly when the
    /// content revision advances (settle commit, or any immediate input).
    pub fn grid_refresh_pending(&self) -> bool {
        self.0.borrow().grid_refresh_pending
    }
    /// Human-readable companion for [`Self::grid_refresh_pending`]; never a fake
    /// final-commit claim.
    pub fn grid_refresh_diagnostic(&self) -> &'static str {
        grid_refresh_label(self.grid_refresh_pending())
    }
    /// The camera-settle timeout currently in force.
    pub fn grid_settle_timeout(&self) -> Duration {
        self.0.borrow().settle_timeout
    }
    /// Test hook: set the camera-settle timeout. `Duration::ZERO` disables the
    /// deferral (today's immediate behaviour) and flushes any pending refresh on
    /// the next pump.
    pub fn set_grid_settle_timeout(&self, timeout: Duration) {
        self.0.borrow_mut().settle_timeout = timeout;
    }
    /// Cumulative overlay-suffix commits (uploads or truncations) the runtime has
    /// performed. A camera-only navigation step must not advance this: it is the
    /// cheap churn signal for the "camera never invalidates the scene" contract.
    pub fn overlay_suffix_commits(&self) -> u64 {
        self.0.borrow().runtime.overlay_suffix_commits()
    }
    pub fn current_prepared_scene(&self) -> Option<controller::PreparedScene> {
        let mut state = self.0.borrow_mut();
        state.observe_drawing();
        current_ready_scene(&state).cloned()
    }
    pub fn active_backend(&self) -> Option<ActiveBackend> {
        self.0
            .borrow()
            .runtime
            .caps
            .as_ref()
            .map(|caps| caps.actual)
    }
    pub fn capabilities(&self) -> Option<(ActiveBackend, bool, u32)> {
        self.0
            .borrow()
            .runtime
            .caps
            .as_ref()
            .map(|caps| (caps.actual, caps.compute, caps.max_texture_dimension))
    }
    pub fn backend_label(&self) -> Option<String> {
        self.0
            .borrow()
            .runtime
            .caps
            .as_ref()
            .map(|caps| caps.api.into())
    }
    pub fn preparation_busy(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.0.borrow().preparation.busy()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }
    pub fn drawing_progress(&self) -> Option<(usize, usize)> {
        self.0.borrow().runtime.drawing_progress()
    }
    pub fn uploading(&self) -> bool {
        self.0.borrow().runtime.uploading()
    }
    pub fn drawing_pages(&self) -> bool {
        self.0.borrow().runtime.drawing_pages()
    }
    pub fn preference(&self) -> BackendPreference {
        self.0.borrow().preference
    }
    pub fn backend_outcome(&self) -> Option<BackendOutcome> {
        self.0.borrow().runtime.outcome.clone()
    }
    pub fn controller_ready(&self) -> Option<controller::PreparedScene> {
        self.0.borrow().controller.ready.clone()
    }
    pub fn set_overlay_visibility(&self, visibility: OverlayVisibility) -> bool {
        let mut overlays = self.overlays();
        if overlays.visibility == visibility {
            return false;
        }
        overlays.visibility = visibility;
        self.set_overlays(overlays)
    }
    pub fn overlay_visibility(&self) -> OverlayVisibility {
        self.0.borrow().overlays.visibility
    }
    pub fn set_selection(&self, selection: SelectionSet) -> bool {
        let mut overlays = self.overlays();
        if overlays.selection == selection {
            return false;
        }
        overlays.selection = selection;
        self.set_overlays(overlays)
    }
    pub fn selection(&self) -> SelectionSet {
        self.0.borrow().overlays.selection.clone()
    }
    pub fn set_measurement_preview(&self, preview: Option<MeasurementPreview>) -> bool {
        let mut overlays = self.overlays();
        if overlays.measurement == preview {
            return false;
        }
        overlays.measurement = preview;
        self.set_overlays(overlays)
    }
    pub fn set_snap_hints(&self, hints: Vec<SnapHint>) -> bool {
        let mut overlays = self.overlays();
        if overlays.snap_hints_input == hints {
            return false;
        }
        overlays.snap_hints_input = hints;
        self.set_overlays(overlays)
    }
    pub fn snap_hints(&self) -> Vec<SnapHint> {
        self.0.borrow().overlays.snap_hints_input.clone()
    }
    pub fn set_draw_preview(&self, preview: Option<cad_app::DrawPreview>) -> bool {
        let mut overlays = self.overlays();
        if overlays.draw_preview == preview {
            return false;
        }
        overlays.draw_preview = preview;
        self.set_overlays(overlays)
    }
    pub fn draw_preview(&self) -> Option<cad_app::DrawPreview> {
        self.0.borrow().overlays.draw_preview.clone()
    }
    pub fn set_viewport_bounds(&self, bounds: Option<(Point3, Point3)>) {
        let mut overlays = self.overlays();
        if overlays.viewport_bounds == bounds {
            return;
        }
        overlays.viewport_bounds = bounds;
        self.set_overlays(overlays);
    }

    pub fn attach_device(&self, device: wgpu::Device, queue: wgpu::Queue) -> Result<(), String> {
        self.attach_device_with_gpu_time(
            device,
            queue,
            cad_render_wgpu::gpu_time::GpuTimeConfig::disabled(),
        )
    }

    /// [`RenderSessionHandle::attach_device`] with the measurement-only GPU
    /// frame-time opt-in (ADR 0013 class 1). The config is threaded to the
    /// renderer through [`CadRenderRuntime::attach_with_gpu_time`]; the engine
    /// never reads the environment.
    pub fn attach_device_with_gpu_time(
        &self,
        device: wgpu::Device,
        queue: wgpu::Queue,
        gpu_time: cad_render_wgpu::gpu_time::GpuTimeConfig,
    ) -> Result<(), String> {
        let mut state = self.0.borrow_mut();
        let preference = state.preference;
        state
            .runtime
            .attach_with_gpu_time(device, queue, preference, gpu_time);
        match &state.runtime.lifecycle {
            RenderLifecycle::Ready { .. } => Ok(()),
            RenderLifecycle::Failed { reason } | RenderLifecycle::Lost { reason } => {
                Err(reason.clone())
            }
            RenderLifecycle::Detached => {
                Err("renderer remained detached after device attach".into())
            }
        }
    }

    pub fn detach_device(&self) {
        self.0.borrow_mut().runtime.detach();
    }

    pub fn fail_device(&self, reason: impl Into<String>, lost: bool) {
        let mut state = self.0.borrow_mut();
        let preference = state.preference;
        state.runtime.fail(preference, reason.into(), lost);
    }

    pub fn apply_ready_updates(&self) -> CadResult<()> {
        let mut state = self.0.borrow_mut();
        state.observe_drawing();
        let ready = current_ready_scene(&state).cloned();
        state.runtime.apply_ready_updates(ready.as_ref())
    }

    pub fn render(
        &self,
        logical: [f64; 2],
        scale: f64,
    ) -> Result<Option<PresentedFrame>, RenderError> {
        let mut state = self.0.borrow_mut();
        state.observe_drawing();
        let view = state.view.clone();
        let view_revision = state.view_revision;
        if Self::preparation_current_state(&state) {
            // The CPU scene is current. Only render it as `CurrentScene` once the
            // GPU has actually committed that scene revision and its base; a
            // chunked `apply_ready_updates` upload may still be pending, and
            // rendering then would present the resident old scene (or a partially
            // uploaded one) under a `CurrentScene` label.
            let committed = match current_ready_scene(&state) {
                Some(scene) => state
                    .runtime
                    .scene_committed(scene.revision, scene.base_revision),
                None => false,
            };
            if committed {
                return state
                    .runtime
                    .render_if_dirty(&view, logical, scale, view_revision);
            }
        }
        // Strict gate is not met (or the ready scene is not yet GPU-committed).
        // Enter the narrow 2D navigation fallback only when the authoritative base
        // is unchanged and fully resident; render the latest camera over the base
        // while overlays wait. Never a new/uncommitted base.
        if base_only_navigation_eligible(&state) {
            return state
                .runtime
                .render_base_only(&view, logical, scale, view_revision);
        }
        Ok(None)
    }

    pub fn lifecycle(&self) -> RenderLifecycle {
        self.0.borrow().runtime.lifecycle.clone()
    }
    pub fn diagnostic(&self) -> Option<String> {
        self.0.borrow().runtime.diagnostic.clone()
    }
    /// Take the observation of the most recent **actual** renderer invocation.
    ///
    /// Delivered at most once per render sequence, so a host can never replay
    /// stale stats for a retained/reused or waiting frame. `None` when no new
    /// actual render happened since the previous take.
    pub fn take_render_stats_observation(&self) -> Option<RenderStatsObservation> {
        self.0.borrow_mut().runtime.take_render_observation()
    }
    /// Take the latest upload stage-timing observation (at most once per upload
    /// work unit). `None` when no new upload staged/committed since the last take.
    pub fn take_upload_timing_observation(&self) -> Option<UploadTimingObservation> {
        self.0.borrow_mut().runtime.take_upload_timing_observation()
    }
    /// The current device epoch (increments on every successful attach).
    pub fn device_epoch(&self) -> u64 {
        self.0.borrow().runtime.device_epoch()
    }
    pub fn frames_rendered(&self) -> u64 {
        self.0.borrow().runtime.frames_rendered
    }
    pub fn redraw_requests(&self) -> u64 {
        self.0.borrow().redraw_requests
    }
    pub fn note_redraw_request(&self) {
        self.0.borrow_mut().redraw_requests += 1;
    }
    pub fn request_redraw(&self, wake: &impl RenderWake) {
        let mut s = self.0.borrow_mut();
        s.redraw_requests += 1;
        if !s.stopped {
            drop(s);
            wake.request_redraw();
        }
    }
    pub fn wake_redraw(&self, wake: &impl RenderWake) {
        if !self.0.borrow().stopped {
            wake.request_redraw();
        }
    }
    pub fn schedule_preparation(&self, wake: &impl RenderWake) {
        if !self.0.borrow().stopped {
            wake.request_preparation();
        }
    }
    pub fn stop(&self) {
        let mut s = self.0.borrow_mut();
        s.stopped = true;
        #[cfg(not(target_arch = "wasm32"))]
        s.preparation.stop();
        s.runtime.detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_session_checks_shared_owner_state_identity() {
        let first = RenderSessionHandle::new(BackendPreference::Auto);
        let clone = first.clone();
        let other = RenderSessionHandle::new(BackendPreference::Auto);
        assert!(first.same_session(&clone));
        assert!(!first.same_session(&other));
    }

    #[test]
    fn session_is_ui_independent_and_identical_view_does_not_dirty() {
        let session = RenderSessionHandle::new(BackendPreference::Auto);
        let view = ViewSnapshot::default();
        assert!(!session.apply_view_snapshot(view).unwrap());
        assert_eq!(session.frames_rendered(), 0);
        assert!(matches!(session.lifecycle(), RenderLifecycle::Detached));
    }

    #[test]
    fn overlay_preparation_reuses_base_while_resource_replacement_invalidates_it() {
        let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
            .unwrap()
            .drawing()
            .unwrap();
        let session = RenderSessionHandle::new(BackendPreference::Auto);
        session.sync_drawing(Some(drawing));
        session.prepare().unwrap();
        let first = session.controller_ready().unwrap();
        let base = first.base.clone();
        let mut overlays = session.overlays();
        overlays.visibility.axes = false;
        assert!(session.set_overlays(overlays));
        session.prepare().unwrap();
        let overlay_scene = session.controller_ready().unwrap();
        assert!(Arc::ptr_eq(&base, &overlay_scene.base));
        assert_eq!(overlay_scene.base_revision, first.base_revision);
        let base_revision = overlay_scene.base_revision;
        assert!(session.set_images(Some(Arc::new(DecodedImageCache::new()))));
        session.prepare().unwrap();
        assert!(session.controller_ready().unwrap().base_revision > base_revision);
    }

    #[test]
    fn dirty_frame_errors_and_lost_lifecycle_never_mark_clean_or_retry() {
        let mut dirty = runtime::FrameInvalidation::default();
        assert!(dirty.dirty());
        dirty.complete();
        assert!(!dirty.dirty());
        dirty.invalidate();
        assert!(dirty.dirty());
        let failed_render: Result<(), &str> = Err("synthetic render failure");
        if failed_render.is_ok() {
            dirty.complete();
        }
        assert!(dirty.dirty(), "failed rendering must leave the frame dirty");
        let session = RenderSessionHandle::new(BackendPreference::Auto);
        session.fail_device("lost", true);
        assert!(matches!(session.lifecycle(), RenderLifecycle::Lost { .. }));
        assert!(session.render([0.0, 0.0], 1.0).unwrap().is_none());
        session.detach_device();
        assert_eq!(session.lifecycle(), RenderLifecycle::Detached);
    }

    #[test]
    fn drawing_replacement_and_equal_view_are_ui_free_inputs() {
        let session = RenderSessionHandle::new(BackendPreference::Auto);
        let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
            .unwrap()
            .drawing();
        assert!(session.sync_drawing(drawing.clone()));
        assert!(!session.sync_drawing(drawing));
        let mut changed = ViewSnapshot::default();
        changed.camera.target.x += 1.0;
        assert!(session.apply_view_snapshot(changed.clone()).unwrap());
        let dirty = session.cad_frame_dirty();
        assert!(!session.apply_view_snapshot(changed).unwrap());
        assert_eq!(session.cad_frame_dirty(), dirty);
        assert_eq!(session.redraw_requests(), 0);
    }

    #[test]
    fn sync_then_native_async_publication_uses_session_global_revisions() {
        use std::time::{Duration, Instant};

        let first = cad_app::host::HostController::with_demo_document([800.0, 600.0])
            .unwrap()
            .drawing()
            .unwrap();
        let second = cad_app::host::HostController::with_demo_document([800.0, 600.0])
            .unwrap()
            .drawing()
            .unwrap();
        let session = RenderSessionHandle::new(BackendPreference::Auto);
        session.sync_drawing(Some(first));
        session.prepare().unwrap();
        let sync_ready = session.current_prepared_scene().unwrap();
        assert_eq!((sync_ready.revision, sync_ready.base_revision), (1, 1));

        session.sync_drawing(Some(second));
        assert!(!session.preparation_current());
        session.request_preparation().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.preparation_current() {
            session.poll_preparation().unwrap();
            assert!(Instant::now() < deadline, "native preparation timed out");
            std::thread::yield_now();
        }
        let async_ready = session.current_prepared_scene().unwrap();
        assert!(async_ready.revision > sync_ready.revision);
        assert!(async_ready.base_revision > sync_ready.base_revision);
    }

    #[test]
    fn input_mutation_invalidates_current_scene_until_reprepared() {
        let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
            .unwrap()
            .drawing()
            .unwrap();
        let session = RenderSessionHandle::new(BackendPreference::Auto);
        session.sync_drawing(Some(drawing));
        session.prepare().unwrap();
        let previous = session.controller_ready().unwrap();
        assert!(session.preparation_current());

        let images = Arc::new(DecodedImageCache::new());
        assert!(session.set_images(Some(images)));
        assert!(!session.preparation_current());
        assert!(session.current_prepared_scene().is_none());
        assert_eq!(
            session.controller_ready().unwrap().revision,
            previous.revision
        );
        session.apply_ready_updates().unwrap();

        assert!(session.set_overlay_visibility(OverlayVisibility {
            axes: false,
            ..OverlayVisibility::default()
        }));
        assert!(!session.preparation_current());
        assert!(session.current_prepared_scene().is_none());
        session.apply_ready_updates().unwrap();
    }

    #[test]
    fn invalid_space_snapshot_is_rejected_atomically() {
        use cad_domain::LayoutId;

        let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
            .unwrap()
            .drawing()
            .unwrap();
        let session = RenderSessionHandle::new(BackendPreference::Auto);
        session.sync_drawing(Some(drawing));
        let before = session.view_snapshot();
        let input_revision = session.input_revision();
        let dirty = session.cad_frame_dirty();
        let invalid = ViewSnapshot {
            space: SpaceSelection::Paper(LayoutId(999_999)),
            ..before.clone()
        };
        assert!(session.apply_view_snapshot(invalid).is_err());
        assert_eq!(session.view_snapshot(), before);
        assert_eq!(session.input_revision(), input_revision);
        assert_eq!(session.cad_frame_dirty(), dirty);
    }

    #[test]
    fn repeated_equal_setters_keep_input_token_stable() {
        let session = RenderSessionHandle::new(BackendPreference::Auto);
        let images = Arc::new(DecodedImageCache::new());
        assert!(session.set_images(Some(images.clone())));
        let token = session.input_revision();
        assert!(!session.set_images(Some(images)));
        assert!(!session.set_fonts(None));
        assert!(!session.set_layers(LayerOverrideSet::new()));
        assert!(!session.set_selection(SelectionSet::new()));
        assert!(!session.set_overlay_visibility(OverlayVisibility::default()));
        assert_eq!(session.input_revision(), token);
    }

    #[test]
    fn resource_generations_survive_unpublished_clear_and_replacement() {
        let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
            .unwrap()
            .drawing()
            .unwrap();
        let session = RenderSessionHandle::new(BackendPreference::Auto);
        session.sync_drawing(Some(drawing));
        session.prepare().unwrap();
        let before = session.current_prepared_scene().unwrap().base_revision;

        let first_cache = Arc::new(DecodedImageCache::new());
        assert!(session.set_images(Some(first_cache.clone())));
        let first_generation = session.0.borrow().images_generation;
        assert!(!session.set_images(Some(first_cache)));
        assert_eq!(session.0.borrow().images_generation, first_generation);
        assert!(session.set_images(None));
        assert_eq!(session.0.borrow().images_generation, first_generation + 1);
        assert!(session.set_images(Some(Arc::new(DecodedImageCache::new()))));
        assert_eq!(session.0.borrow().images_generation, first_generation + 2);

        let first_fonts = Arc::new(FontEngine::new());
        assert!(session.set_fonts(Some(first_fonts.clone())));
        let font_generation = session.0.borrow().fonts_generation;
        assert!(!session.set_fonts(Some(first_fonts)));
        assert_eq!(session.0.borrow().fonts_generation, font_generation);
        assert!(session.set_fonts(None));
        assert_eq!(session.0.borrow().fonts_generation, font_generation + 1);
        assert!(session.set_fonts(Some(Arc::new(FontEngine::new()))));
        assert_eq!(session.0.borrow().fonts_generation, font_generation + 2);
        assert!(!session.preparation_current());

        session.prepare().unwrap();
        let after = session.current_prepared_scene().unwrap().base_revision;
        assert!(after > before, "resource generations must invalidate the base even without an intermediate publication");
    }

    fn facts(
        detached: bool,
        ready_lifecycle: bool,
        scene_current: bool,
        committed: bool,
        pending: Option<bool>,
        base_resident: bool,
    ) -> GateFacts {
        GateFacts {
            detached,
            ready_lifecycle,
            scene_current,
            committed,
            pending,
            base_resident,
        }
    }

    #[test]
    fn render_gate_reason_covers_every_waiting_stage() {
        // Detached: no renderer at all.
        assert_eq!(
            classify_gate(facts(true, false, false, false, None, false)),
            RenderGateReason::Detached
        );
        // Attached-but-terminal.
        assert_eq!(
            classify_gate(facts(false, false, false, false, None, false)),
            RenderGateReason::Failed
        );
        // CPU scene preparation has not published the current scene.
        assert_eq!(
            classify_gate(facts(false, true, false, false, None, false)),
            RenderGateReason::Preparing
        );
        // Published scene, base upload still in flight.
        assert_eq!(
            classify_gate(facts(false, true, true, false, Some(true), false)),
            RenderGateReason::UploadingBase
        );
        // Published scene, overlay-suffix upload in flight.
        assert_eq!(
            classify_gate(facts(false, true, true, false, Some(false), true)),
            RenderGateReason::UploadingOverlay
        );
        // Published scene, nothing pending, base not resident (identity mismatch /
        // device reset) — never reported Ready.
        assert_eq!(
            classify_gate(facts(false, true, true, false, None, false)),
            RenderGateReason::BaseIdentityMismatch
        );
        // Gate met.
        assert_eq!(
            classify_gate(facts(false, true, true, true, None, true)),
            RenderGateReason::Ready
        );
        // A committed scene is Ready regardless of a stale pending flag.
        assert_eq!(
            classify_gate(facts(false, true, true, true, Some(true), true)),
            RenderGateReason::Ready
        );
    }

    #[test]
    fn diagnostics_read_is_pure_and_never_labels_a_success() {
        let session = RenderSessionHandle::new(BackendPreference::Auto);
        let input_before = session.input_revision();
        let view_before = session.view_revision();
        let frames_before = session.frames_rendered();
        let first = session.diagnostics();
        let second = session.diagnostics();
        // Repeated reads are identical (no time/state dependent drift).
        assert_eq!(first, second);
        // Reading diagnostics must not advance revisions or render anything.
        assert_eq!(session.input_revision(), input_before);
        assert_eq!(session.view_revision(), view_before);
        assert_eq!(session.frames_rendered(), frames_before);
        // A fresh, device-less session is Detached and never a fabricated Ready /
        // current scene.
        assert_eq!(first.gate, RenderGateReason::Detached);
        assert_eq!(first.lifecycle, "detached");
        assert_eq!(first.ready_revision, None);
        assert_eq!(first.ready_base_revision, None);
        assert_eq!(first.prepared_token, None);
        assert_eq!(first.prepared_identity, None);
        assert!(!first.runtime.uploading);
        assert_eq!(first.runtime.upload, None);
        assert_eq!(first.runtime.frames_rendered, 0);
        assert_eq!(first.runtime.uploads_started, 0);
        assert_eq!(first.runtime.upload_revision_resets, 0);
        assert_eq!(first.runtime.ready_none_clears, 0);
    }
}

#[cfg(test)]
mod contracts;
