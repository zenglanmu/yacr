//! Owner-thread render session state and CPU scene preparation.
use cad_app::layers::LayerOverrideSet;
use cad_app::recovery::{ActiveBackendKind, BackendFailure, BackendOutcome};
use cad_app::render_scene as controller;
use cad_app::render_scene::{
    CadSceneController, DecodedImageCache, DrawingEditReceipt, OverlayInputs, SnapHint,
};
use cad_app::vendor::VendorRepresentationStore;
use cad_app::{Camera, MeasurementPreview, ProjectionKind, SelectionSet};
use cad_db::DrawingDatabase;
use cad_domain::{CadError, CadResult, DocumentId, Point3, SpaceId};
/// Re-exported so platform adapters (for example Android) can name a render
/// session's backend preference without depending on `cad-render-wgpu`.
pub use cad_render_wgpu::BackendPreference;
use cad_render_wgpu::{
    wgpu, ActiveBackend, GpuBudgetConfig, GpuMemoryRefusal, GpuMemoryReport, RenderError,
};
use cad_representation::{FontEngine, SpaceSelection};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

// `std::time::Instant::now()` panics on wasm32-unknown-unknown ("time not
// implemented on this platform"); `web-time` re-exports std off wasm, so this is
// a no-op for every native host and the only monotonic source here.
use web_time::Instant;

#[cfg(not(target_arch = "wasm32"))]
mod preparation;
#[cfg(not(target_arch = "wasm32"))]
use preparation::{NativePreparation, PreparationInput};
mod camera;
mod runtime;
pub use camera::{
    camera2d_from_params, camera3d_from_params, fit_camera, place_canvas_camera2d, BridgeCamera,
};
use runtime::CadRenderRuntime;
pub use runtime::{
    render_target_size, FrameBinding, FrameCompleteness, PresentedFrame, RenderLifecycle,
    RenderStatsObservation, RuntimeDiagnostics, UploadDiagnostics, UploadTimingObservation,
};

mod diagnostics;
mod session_impl;
pub use diagnostics::{RenderDiagnostics, RenderGateReason};

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
    /// Placement of the camera canvas inside the render target, in logical px
    /// from the target's top-left.
    ///
    /// `None` means "the canvas is the target" (legacy/direct renderer callers
    /// and every host whose native child surface is exactly the CAD rect).
    /// `Some(origin)` means `canvas_size` is a visible sub-rect of a larger
    /// target (Android's full-window surface under the WebView chrome); the
    /// orthographic camera centre is translated so the rect, not the target,
    /// is framed centred. A `Some` origin of `[0,0]` with equal sizes is
    /// accepted and is a no-op.
    pub canvas_origin: Option<[f64; 2]>,
    /// The colour this view's frames clear the canvas to, as linear channel
    /// values (the renderer's own colour space). It is a display preference the
    /// host resolves per active space — a plain value, not a configuration
    /// dependency — and the default is the renderer's documented opaque black.
    pub clear_color: [f32; 4],
}
impl Default for ViewSnapshot {
    fn default() -> Self {
        Self {
            document: DocumentId(0),
            space: SpaceSelection::Model,
            camera: Camera::top_view_2d(),
            mode: ProjectionKind::TwoD,
            canvas_size: [1280.0, 800.0],
            canvas_origin: None,
            clear_color: [0.0, 0.0, 0.0, 1.0],
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
    /// Display `WIPEOUTFRAME` state: whether each base mask draws its boundary
    /// frame (`true` for `ON`/`Display but not plot`). Session state, but part of
    /// this identity because it changes the base geometry.
    wipeout_frames: bool,
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
            wipeout_frames: state.wipeout_frames,
        }
    }
}

/// Default camera-settle timeout for the deferred reference-grid refresh
/// (camera-only pan/zoom/rotate). See [`State::grid_refresh_pending`].
const CAMERA_SETTLE_TIMEOUT: Duration = Duration::from_millis(250);
/// Default navigation-pause timeout: while camera changes keep arriving, the
/// progressive walk draws at most the first page per change and pauses; after
/// this much quiet it resumes to completion. Kept separate from the 250 ms grid
/// settle so the final image starts filling before the grid refresh commits.
const NAVIGATION_SETTLE_TIMEOUT: Duration = Duration::from_millis(120);

struct State {
    incoming: IncomingDocument,
    controller: CadSceneController,
    view: ViewSnapshot,
    fonts: Option<Arc<FontEngine>>,
    images: Option<Arc<DecodedImageCache>>,
    /// Derived vendor display snapshot (display-only; never `cad-db` state).
    vendor: Option<Arc<VendorRepresentationStore>>,
    fonts_generation: u64,
    images_generation: u64,
    layers: LayerOverrideSet,
    /// Display `WIPEOUTFRAME` state, fed to the scene controller. A flip
    /// rebuilds the base (the mask boundary frame is base geometry).
    wipeout_frames: bool,
    overlays: OverlayInputs,
    overlay_revision: u64,
    /// Monotonic revision of the **scene identity**. Advances only when an input
    /// that changes the published scene advances (drawing, document, space, layer
    /// overrides, font/image resources, or overlay batches). Camera-only changes
    /// (pan/zoom, mode, canvas size) deliberately do **not** advance it: the
    /// published scene stays valid because every overlay batch is world-space
    /// geometry that renders correctly under any camera.
    input_revision: u64,
    /// Monotonic revision of the **flags publication** (entity-level
    /// selection/hover/hidden). It advances only when the resolved flag set
    /// changes and is stamped onto the published scene as
    /// [`controller::PreparedScene::flags_revision`]. It deliberately does not
    /// touch `input_revision` (scene identity) or `content_revision` (preparation
    /// token): a flags-only change keeps the published scene current and never
    /// schedules a worker preparation. A global counter (not the controller's)
    /// keeps it monotonic across worker candidates that carry stale counters.
    flags_publication_revision: u64,
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
    /// Latest admitted drawing-edit receipt (see [`DrawingEditReceipt`]).
    ///
    /// Retained until the authoritative drawing `Arc` changes (a new/unrelated
    /// drawing clears it) or a newer receipt replaces it. A same-`Arc`
    /// `sync_render` never erases it: the receipt is an append hint for the
    /// *next* preparation, not a one-shot side effect of a sync call.
    latest_edit: Option<DrawingEditReceipt>,
    scene_publication_revision: u64,
    base_publication_revision: u64,
    published_base_identity: Option<BaseInputIdentity>,
    runtime: CadRenderRuntime,
    /// The GPU memory budget policy a host configured through
    /// [`RenderSessionHandle::set_gpu_budget_config`].
    ///
    /// `None` until a host sets one: the renderer then keeps its own
    /// environment-derived policy ([`GpuBudgetConfig::from_env`]). A stored
    /// policy is re-applied after every device attach, because
    /// [`CadRenderRuntime::attach_with_gpu_time`] constructs a **fresh**
    /// renderer that would otherwise silently fall back to the environment.
    gpu_budget_config: Option<GpuBudgetConfig>,
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
    /// A camera-only navigation gesture is active: the progressive walk is
    /// paused after the first page of each camera change and resumes once the
    /// view has been quiet for `navigation_settle_timeout`.
    navigation_pause_pending: bool,
    /// Navigation-pause timeout. `Duration::ZERO` disables the pause (immediate
    /// continuation) and is the test kill-switch.
    navigation_settle_timeout: Duration,
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
            vendor: None,
            fonts_generation: 0,
            images_generation: 0,
            layers: LayerOverrideSet::new(),
            wipeout_frames: false,
            overlays: OverlayInputs::default(),
            overlay_revision: 0,
            input_revision: 0,
            flags_publication_revision: 0,
            content_revision: 0,
            view_revision: 0,
            prepared_token: None,
            prepared_identity: None,
            observed_drawing: 0,
            latest_edit: None,
            scene_publication_revision: 0,
            base_publication_revision: 0,
            published_base_identity: None,
            runtime: CadRenderRuntime::default(),
            gpu_budget_config: None,
            preference: BackendPreference::Auto,
            redraw_requests: 0,
            stopped: false,
            view_diagnostic: None,
            grid_refresh_pending: false,
            last_camera_change: None,
            settle_timeout: CAMERA_SETTLE_TIMEOUT,
            navigation_pause_pending: false,
            navigation_settle_timeout: NAVIGATION_SETTLE_TIMEOUT,
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
        // A content advance is a real input/publication, not an idle pause.
        self.clear_navigation_pause();
    }

    /// Publish a flags-only change: re-resolve the staged flags from the current
    /// [`OverlayInputs`] via the controller's single flags resolver, stamp the
    /// global flags publication revision and invalidate the frame.
    ///
    /// Neither `input_revision` nor `content_revision` moves, so the published
    /// scene stays current (`current_ready_scene` keeps returning it), no worker
    /// preparation is scheduled, and a pending camera-settle grid refresh
    /// survives. The scene revision and both geometry `Arc`s are untouched: the
    /// renderer receives the new flags as a delta over the committed geometry.
    fn refresh_flags_only(&mut self) {
        let drawing = self.incoming.borrow().clone();
        let changed = self.controller.refresh_flags_only(
            drawing.as_deref(),
            self.view.document,
            &self.overlays,
        );
        if changed {
            self.publish_flags_revision();
        }
    }

    /// Advance the global flags publication revision and stamp it on the
    /// published scene. The scene revision/base stay untouched.
    fn publish_flags_revision(&mut self) -> u64 {
        self.flags_publication_revision = self
            .flags_publication_revision
            .checked_add(1)
            .expect("render flags revision exhausted");
        if let Some(ready) = self.controller.ready.as_mut() {
            ready.flags_revision = self.flags_publication_revision;
        }
        self.runtime.dirty.invalidate();
        self.flags_publication_revision
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
            // The camera has stopped: use the full page budget for the remaining
            // progressive pages so the final image completes quickly. The
            // measured cost model (EMA/seeded) is retained; only the scale is
            // raised, and the controller shrinks it again if a boosted page is
            // genuinely over target.
            self.runtime.boost_page_budget();
            self.advance_content();
            true
        } else {
            false
        }
    }

    /// Arm/refresh the navigation pause (camera-only pan/zoom/rotate).
    fn arm_navigation_pause(&mut self, now: Instant) {
        self.navigation_pause_pending = true;
        self.last_camera_change = Some(now);
        // Hide text glyphs while the camera is moving; a no-op once already on.
        self.runtime.set_navigation_lod(true);
    }

    /// Clear the pause because a non-camera input or a settle already covers it.
    /// This also restores full text (the LOD change invalidates the frame).
    fn clear_navigation_pause(&mut self) {
        self.navigation_pause_pending = false;
        self.runtime.set_navigation_lod(false);
    }

    fn navigation_settle_enabled(&self) -> bool {
        cfg!(not(target_arch = "wasm32")) && !self.navigation_settle_timeout.is_zero()
    }

    /// Resume the paused progressive walk once the view has been quiet for the
    /// navigation timeout. Returns whether the pause was cleared.
    fn settle_navigation_if_due(&mut self, now: Instant) -> bool {
        if !self.navigation_pause_pending {
            return false;
        }
        let due = match self.last_camera_change {
            Some(last) => now.saturating_duration_since(last) >= self.navigation_settle_timeout,
            None => true,
        };
        if due {
            self.clear_navigation_pause();
            // Only a paused walk has more pages to draw; never invalidate a
            // completed scene just because a gesture ended. (`clear_navigation_pause`
            // already invalidated when it turned the text LOD off.)
            if self.runtime.drawing_pages() {
                self.runtime.dirty.invalidate();
            }
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
    // Race guard: a worker preparation captures its overlay inputs at submission
    // time. A flags-only change (hover/selection/hidden) is applied synchronously
    // to the state controller and does **not** submit a preparation, so a worker
    // completion that lands afterwards still carries the older flags. Re-resolve
    // them from the CURRENT `state.overlays` at every publication boundary: the
    // controller's fingerprints make this a no-op when nothing changed, and it is
    // O(refs) cheap. Publishing the stale set would regress the tint to an older
    // hover.
    let drawing = state.incoming.borrow().clone();
    candidate.refresh_flags_only(drawing.as_deref(), state.view.document, &state.overlays);
    let Some(mut ready) = candidate.ready.clone() else {
        return false;
    };
    let previous = state.controller.ready.clone();
    let flags_changed = previous
        .as_ref()
        .is_none_or(|published| published.entity_flags != ready.entity_flags);
    // A publication whose base and overlay geometry are the *same allocations* as
    // the already published scene is a flags-only publication: it must keep the
    // scene revision, otherwise a fresh un-applied `ready.revision` would make
    // `scene_committed` false and re-enter the base-only/suppression path (F4).
    let flags_only_publish = previous.as_ref().is_some_and(|published| {
        published.base_revision == ready.base_revision
            && published.base.same_as(&ready.base)
            && Arc::ptr_eq(&published.highlight, &ready.highlight)
            && same_arc_option(published.images.as_ref(), ready.images.as_ref())
    });
    if flags_only_publish {
        let mut published = previous.expect("checked above");
        published.entity_flags = ready.entity_flags;
        published.highlight_diagnostics = ready.highlight_diagnostics;
        published.highlight_completeness = ready.highlight_completeness;
        if flags_changed {
            published.flags_revision = state.publish_flags_revision();
        }
        candidate.ready = Some(published);
        state.controller = candidate;
        state.prepared_token = Some(token);
        state.runtime.dirty.invalidate();
        return true;
    }
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
    ready.flags_revision = if flags_changed {
        state.publish_flags_revision()
    } else {
        state.flags_publication_revision
    };
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

/// Whether two optional `Arc`s are the same allocation (`None == None`).
fn same_arc_option<T>(left: Option<&Arc<T>>, right: Option<&Arc<T>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        _ => false,
    }
}

fn current_ready_scene(state: &State) -> Option<&controller::PreparedScene> {
    (!state.stopped && state.prepared_identity == Some(state.input_revision))
        .then_some(())
        .and(state.controller.ready.as_ref())
}

/// Narrow qualifier for presenting the resident scene while newer overlay inputs
/// are still pending.
///
/// It holds only when the *base* drawing is provably unchanged and fully
/// resident: the view is a valid 2D camera, the latest base input identity equals
/// the last published one (so no new database, space, layer, font, image or
/// resource identity is pending), and the GPU holds the latest published base
/// with no pending base replacement. Everything that could make the resident
/// geometry stale (new/unuploaded base, device change) fails this predicate, so
/// the frame can only ever draw the authoritative base. Overlay-suffix freshness
/// is **not** a condition: the suffix is world-space geometry that stays valid
/// under any camera, so presenting it one publication behind is always better
/// than truncating it (a plain cursor move changes exactly that suffix).
fn resident_scene_presentable(state: &State) -> bool {
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
        vendor: state.vendor.clone(),
        layers: state.layers.clone(),
        wipeout_frames: state.wipeout_frames,
        overlays: state.overlays.clone(),
        edit: state.latest_edit.clone(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn prepare_synchronously(state: &mut State) -> CadResult<()> {
    state.observe_drawing();
    let input = preparation_input(state);
    let mut candidate = state.controller.clone();
    candidate.set_vendor_representations(input.vendor.clone());
    candidate.set_wipeout_frames(input.wipeout_frames);
    candidate.prepare_shared_with_overlays_and_images_and_edit(
        input.drawing.clone(),
        input.document,
        input.fonts.clone(),
        &input.layers,
        input.space,
        &input.overlays,
        input.images.clone(),
        input.edit.as_ref(),
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
    candidate.set_vendor_representations(state.vendor.clone());
    candidate.set_wipeout_frames(state.wipeout_frames);
    candidate.prepare_shared_with_overlays_and_images_and_edit(
        state.incoming.borrow().clone(),
        state.view.document,
        state.fonts.clone(),
        &state.layers,
        state.view.space,
        &state.overlays,
        state.images.clone(),
        state.latest_edit.as_ref(),
    )?;
    publish_controller(state, candidate, token);
    Ok(())
}

/// Cloneable, deliberately single-owner-thread handle shared by the engine and
/// its UI adapter. All authoritative CAD/render-session mirrors live here.
#[derive(Clone, Default)]
pub struct RenderSessionHandle(Rc<RefCell<State>>);

#[cfg(test)]
mod contracts;
#[cfg(test)]
mod tests;
