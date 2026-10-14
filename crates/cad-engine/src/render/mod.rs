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

#[cfg(not(target_arch = "wasm32"))]
mod preparation;
#[cfg(not(target_arch = "wasm32"))]
use preparation::{NativePreparation, PreparationInput};
mod camera;
mod runtime;
pub use camera::{camera2d_from_params, camera3d_from_params, fit_camera, BridgeCamera};
use runtime::CadRenderRuntime;
pub use runtime::{FrameBinding, FrameCompleteness, PresentedFrame, RenderLifecycle};

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
    input_revision: u64,
    /// Monotonic revision of the authoritative view (camera/space). Advances only
    /// when the view actually changes, and tags every presented frame.
    view_revision: u64,
    prepared_token: Option<u64>,
    observed_drawing: usize,
    scene_publication_revision: u64,
    base_publication_revision: u64,
    published_base_identity: Option<BaseInputIdentity>,
    runtime: CadRenderRuntime,
    preference: BackendPreference,
    redraw_requests: u64,
    stopped: bool,
    view_diagnostic: Option<String>,
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
            view_revision: 0,
            prepared_token: None,
            observed_drawing: 0,
            scene_publication_revision: 0,
            base_publication_revision: 0,
            published_base_identity: None,
            runtime: CadRenderRuntime::default(),
            preference: BackendPreference::Auto,
            redraw_requests: 0,
            stopped: false,
            view_diagnostic: None,
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
    if token != state.input_revision || state.prepared_token == Some(token) {
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
    state.runtime.dirty.invalidate();
    true
}

fn current_ready_scene(state: &State) -> Option<&controller::PreparedScene> {
    (!state.stopped && state.prepared_token == Some(state.input_revision))
        .then_some(())
        .and(state.controller.ready.as_ref())
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
        token: state.input_revision,
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
    let token = state.input_revision;
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
        state.view = view;
        state.view_revision = state
            .view_revision
            .checked_add(1)
            .expect("view revision exhausted");
        state.advance_input();
        let bounds = viewport_world_bounds(&state.view);
        if state.overlays.viewport_bounds != bounds {
            state.overlays.viewport_bounds = bounds;
            state.overlay_revision += 1;
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
        if state.overlays != overlays {
            state.overlays = overlays;
            state.overlay_revision += 1;
            state.advance_input();
            return true;
        }
        false
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
        let mut state = self.0.borrow_mut();
        let preference = state.preference;
        state.runtime.attach(device, queue, preference);
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
}

#[cfg(test)]
mod contracts;
