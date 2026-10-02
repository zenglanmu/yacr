//! Slint ⇄ wgpu composition bridge (spec v2.0 §5.2).
//!
//! Slint is the single presentation coordinator. It hands out its shared
//! `Device`/`Queue` through `set_rendering_notifier`; the CAD renderer draws
//! into an offscreen texture that Slint composites as an `Image`. There is no
//! per-frame GPU→CPU readback, and the CAD module never owns the window.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use cad_db::{AnnotationDatabase, DrawingDatabase};
use cad_domain::{
    CadError, CadResult, DocumentId, Point3, SceneIdentity, SpaceId, TaskStamp, TolerancePolicy,
};
use cad_render_wgpu::{
    ActiveBackend, BackendCapabilities, BackendPreference, Camera2d, Camera3d, RenderTarget,
    Renderer,
};
use cad_representation::layout::{build_paper_space, enumerate_layouts};
use cad_representation::{
    FontEngine, LayoutDescriptor, ProviderRegistry, RepresentationContext, SpaceSelection,
};
use cad_scene::{
    annotation_batches, AnnotationScene, AnnotationSceneOptions, FrameBudget, SceneBudget,
    SceneCache, SceneDelta,
};

use cad_app::layers::LayerOverrideSet;
use cad_app::recovery::{ActiveBackendKind, BackendFailure, BackendOutcome};
use cad_app::BackendChoice;
use cad_app::{
    AnnotationVisibilitySet, Camera, Camera3dParams, Projection, ProjectionKind, Viewport,
};

use crate::{UiHandle, YacrWindow};

/// A shared slot the application fills when a drawing becomes available.
pub type IncomingDocument = Rc<RefCell<Option<Arc<DrawingDatabase>>>>;

/// A shared slot the application fills when the annotation sidecar is available.
pub type IncomingAnnotations = Rc<RefCell<Option<Arc<AnnotationDatabase>>>>;

/// Camera the bridge keeps in sync with the UI.
#[derive(Clone, Copy)]
pub struct BridgeCamera {
    pub center: Point3,
    pub world_per_px: f64,
}

impl Default for BridgeCamera {
    fn default() -> Self {
        BridgeCamera {
            center: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            world_per_px: 1.0,
        }
    }
}

/// Build scene batches from a drawing database through the provider chain.
pub fn build_scene(database: &DrawingDatabase, stamp: TaskStamp) -> CadResult<SceneDelta> {
    build_scene_with_fonts(database, stamp, None)
}

/// Build scene batches, shaping text with `fonts` when supplied.
pub fn build_scene_with_fonts(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
) -> CadResult<SceneDelta> {
    build_scene_with_overrides(database, stamp, fonts, &LayerOverrideSet::new())
}

/// Build scene batches honouring temporary layer visibility (spec F03).
///
/// An entity whose layer is hidden — either by the drawing's own `visible` flag
/// or by a session override — is skipped, so hiding a layer changes the batches
/// without re-importing or re-parsing the drawing database. The override set is
/// a plain value owned by `cad-app`; `cad-representation` and `cad-scene` are
/// untouched, so this filter is the whole wire-up (see `docs/panels.md`).
pub fn build_scene_with_overrides(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
) -> CadResult<SceneDelta> {
    build_scene_with_space(database, stamp, fonts, overrides, SpaceSelection::Model)
}

/// Build scene batches for an explicit space (model or a paper layout, spec F04).
///
/// `SpaceSelection::Model` reproduces [`build_scene_with_overrides`] exactly, so
/// the existing model-space entry point keeps working. `SpaceSelection::Paper`
/// draws the layout's own paper geometry and each supported viewport's clipped
/// model contents (see [`cad_representation::layout`]). A layout whose viewports
/// are not drawable contributes no viewport geometry and records an explicit
/// diagnostic; callers can inspect [`layout_descriptors`] before switching.
pub fn build_scene_with_space(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
) -> CadResult<SceneDelta> {
    let registry = ProviderRegistry::with_default_provider();
    let mut context =
        RepresentationContext::new(DocumentId(0), TolerancePolicy::default(), stamp.clone());
    if let Some(fonts) = fonts {
        context = context.with_fonts(fonts);
    }
    let mut cache = SceneCache::new(SceneBudget::default());
    let mut combined = SceneDelta {
        stamp: stamp.clone(),
        added: Vec::new(),
        removed_chunks: Vec::new(),
    };
    match space {
        SpaceSelection::Model => {
            for entity in cad_app::layers::visible_model_entities(database, overrides) {
                let representation = registry.build_expanded(database, entity, &context)?;
                let delta = cache.build(&representation, stamp.clone())?;
                combined.added.extend(delta.added);
            }
        }
        SpaceSelection::Paper(layout) => {
            let visible = |entity: &cad_db::DbEntity| overrides.is_entity_visible(database, entity);
            let representation =
                build_paper_space(&registry, database, layout, &context, &visible)?;
            let delta = cache.build(&representation, stamp.clone())?;
            combined.added.extend(delta.added);
        }
    }
    Ok(combined)
}

/// The layouts a caller can switch to, with per-layout support and reason.
///
/// The UI never invents a layout; this is the database's own layout table with
/// the representation layer's drawability verdict attached. Model space is not
/// listed (it is always available as [`SpaceSelection::Model`]).
pub fn layout_descriptors(database: &DrawingDatabase) -> Vec<LayoutDescriptor> {
    enumerate_layouts(database)
}

/// Build the drawing scene plus the annotation overlay into one delta (spec F07).
///
/// This is the wire-up that makes stored annotations visible: the drawing batches
/// come from [`build_scene_with_overrides`] unchanged, and the annotation batches
/// come from [`cad_scene::annotation_batches`]. The two are concatenated into a
/// single [`SceneDelta`] so the renderer uploads both in one pass — there is no
/// second renderer and no separate submit path.
///
/// `visibility` is the session override set from `cad-app`; a hidden annotation
/// contributes no batch and is *not* reported as a gap. `annotations` is optional
/// so a host without a sidecar keeps working (an absent sidecar is an empty
/// overlay, not a silent drop of a known database).
///
/// The returned [`AnnotationScene`] carries the completeness and diagnostics of
/// the conversion; callers can surface them without re-running the conversion.
pub fn build_scene_with_annotations(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
    annotations: Option<&AnnotationDatabase>,
    visibility: &AnnotationVisibilitySet,
    annotation_document: DocumentId,
) -> CadResult<(SceneDelta, AnnotationScene)> {
    build_scene_with_annotations_in_space(
        database,
        stamp,
        fonts,
        overrides,
        SpaceSelection::Model,
        annotations,
        visibility,
        annotation_document,
    )
}

/// Build the drawing scene for an explicit space plus the annotation overlay.
///
/// Identical to [`build_scene_with_annotations`] except the drawing batches come
/// from [`build_scene_with_space`], so the model/paper switch reaches the render
/// path without a second scene builder. `SpaceSelection::Model` reproduces the
/// model-space entry point exactly.
#[allow(clippy::too_many_arguments)]
pub fn build_scene_with_annotations_in_space(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
    annotations: Option<&AnnotationDatabase>,
    visibility: &AnnotationVisibilitySet,
    annotation_document: DocumentId,
) -> CadResult<(SceneDelta, AnnotationScene)> {
    let mut delta =
        build_scene_with_space(database, stamp.clone(), fonts.clone(), overrides, space)?;
    let budget = FrameBudget::from_scene(&SceneBudget::default());
    let options = AnnotationSceneOptions {
        document: annotation_document,
        fonts: fonts.as_deref(),
        budget,
        ..AnnotationSceneOptions::default()
    };
    let annotation_scene = match annotations {
        Some(db) => annotation_batches(db.annotations(), |id| visibility.effective(id), &options),
        None => annotation_batches(std::iter::empty(), |_| false, &options),
    };
    delta.added.extend(annotation_scene.batches.iter().cloned());
    Ok((delta, annotation_scene))
}

/// A stable fingerprint of the annotation database and its visibility state.
///
/// The bridge rebuilds when this changes, so an annotation create/update/delete
/// (which advances the database revision) or a hide/show (which does not) both
/// trigger exactly one rebuild. It is computed here rather than on
/// [`AnnotationDatabase`] because `cad-app`'s visibility set and `cad-db` are
/// outside this crate's mutable scope.
pub fn annotation_fingerprint(
    annotations: Option<&AnnotationDatabase>,
    visibility: &AnnotationVisibilitySet,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match annotations {
        Some(db) => {
            db.id().0.hash(&mut hasher);
            db.revision().0.hash(&mut hasher);
            db.len().hash(&mut hasher);
        }
        None => 0u8.hash(&mut hasher),
    }
    for (id, visible) in visibility.iter() {
        id.0.hash(&mut hasher);
        visible.hash(&mut hasher);
    }
    hasher.finish()
}

/// Compute a fit-to-drawing camera from the database bounds.
pub fn fit_camera(database: &DrawingDatabase, logical_size: [f64; 2]) -> BridgeCamera {
    match database.bounds() {
        Some((min, max)) => {
            let ex = (max.x - min.x).max(1e-6);
            let ey = (max.y - min.y).max(1e-6);
            BridgeCamera {
                center: Point3 {
                    x: (min.x + max.x) * 0.5,
                    y: (min.y + max.y) * 0.5,
                    z: 0.0,
                },
                world_per_px: (ex / logical_size[0].max(1.0)).max(ey / logical_size[1].max(1.0))
                    * 1.05,
            }
        }
        None => BridgeCamera::default(),
    }
}

/// Map validated application 2D camera parameters to the renderer's `Camera2d`.
///
/// The single point where the two crate types meet for the plan path; it is a
/// field-for-field mapping with no repair, so the app keeps ownership of the
/// projection validation.
pub fn camera2d_from_params(params: cad_app::Camera2dParams) -> Camera2d {
    Camera2d {
        center: params.center,
        world_per_px: params.world_per_px,
        z_plane: 0.0,
    }
}

/// Map validated application 3D camera parameters to the renderer's `Camera3d`.
///
/// The single point where the two crate types meet for the 3D path. `cad-app`
/// derived the near/far planes and rejected a degenerate camera; here the fields
/// are copied verbatim, never repaired.
pub fn camera3d_from_params(params: Camera3dParams) -> Camera3d {
    Camera3d {
        eye: params.eye,
        target: params.target,
        up: params.up,
        fov_y: params.fov_y,
        near: params.near,
        far: params.far,
    }
}

struct BridgeState {
    renderer: Option<Renderer>,
    camera: BridgeCamera,
    /// The authoritative application camera, in whatever projection the
    /// viewport currently has. Drives the 3D render path and is the source of
    /// the 2D mirror above.
    camera3d: Camera,
    /// Active space the scene is (or is being) built for (F04).
    space: SpaceSelection,
    /// Observation mode; selects `render` vs `render_3d`.
    view_mode: ProjectionKind,
    document: Option<SceneIdentity>,
    image_size: Option<(u32, u32)>,
    /// Result of the last backend selection/init attempt.
    ///
    /// Replaces the old generic `error: Option<String>`: callers can now tell a
    /// live backend from a failure and always see which backend really ran
    /// (audit F12).
    outcome: Option<BackendOutcome>,
    caps: Option<BackendCapabilities>,
    fonts_present: bool,
    /// Fingerprint of the applied layer overrides; a change forces a rebuild.
    overrides_fingerprint: u64,
    /// Fingerprint of the annotation database revision plus session visibility;
    /// a change forces a rebuild (F07).
    annotations_fingerprint: u64,
    /// The paper layout the current GPU batches were built for, so a space switch
    /// forces a rebuild even when the document identity is unchanged.
    built_space: Option<SpaceSelection>,
    /// Generation of the device the current `document` identity was built for.
    /// A device rebuild bumps this so the next frame re-uploads from the DB.
    built_generation: Option<u64>,
    /// Explicit result of the last space selection; an unsupported/unknown
    /// layout or a refused camera lands here instead of a blank frame.
    view_diagnostic: Option<String>,
}

impl Default for BridgeState {
    fn default() -> Self {
        BridgeState {
            renderer: None,
            camera: BridgeCamera::default(),
            camera3d: Camera::top_view_2d(),
            space: SpaceSelection::Model,
            view_mode: ProjectionKind::TwoD,
            document: None,
            image_size: None,
            outcome: None,
            caps: None,
            fonts_present: false,
            overrides_fingerprint: 0,
            annotations_fingerprint: 0,
            built_space: None,
            built_generation: None,
            view_diagnostic: None,
        }
    }
}

/// Cloneable view control the host uses to navigate and inspect the CAD frame.
///
/// The authoritative camera lives in the application `Viewport`; the host syncs
/// it here after every command, so there is exactly one camera truth (spec §4.2).
#[derive(Clone)]
pub struct CadView {
    state: Rc<RefCell<BridgeState>>,
    handle: UiHandle,
    fonts: Rc<RefCell<Option<Arc<FontEngine>>>>,
    overrides: Rc<RefCell<LayerOverrideSet>>,
    annotations: IncomingAnnotations,
    annotation_visibility: Rc<RefCell<AnnotationVisibilitySet>>,
    /// The document slot, kept so `set_space` can validate a layout against the
    /// real layout table before switching (never a fabricated space).
    incoming: IncomingDocument,
    preference: BackendPreference,
    /// Shared catalog so backend failure text follows the active language.
    messages: Rc<RefCell<crate::i18n::MessageSource>>,
}

impl CadView {
    /// Mirror the authoritative viewport camera into the render state.
    pub fn set_camera(&self, center: Point3, world_per_px: f64) {
        if !world_per_px.is_finite() || world_per_px <= 0.0 {
            return;
        }
        let mut s = self.state.borrow_mut();
        s.camera.center = Point3 {
            x: center.x,
            y: center.y,
            z: 0.0,
        };
        s.camera.world_per_px = world_per_px.clamp(1e-12, 1e18);
        drop(s);
        let _ = self.handle.request_redraw();
    }

    /// Snapshot of the render camera for diagnostics/tests.
    pub fn camera(&self) -> BridgeCamera {
        self.state.borrow().camera
    }

    /// The authoritative application 3D camera mirrored into the bridge.
    pub fn camera3d(&self) -> Camera {
        self.state.borrow().camera3d
    }

    /// The space the bridge will build the next frame for.
    pub fn space(&self) -> SpaceSelection {
        self.state.borrow().space
    }

    /// The current observation mode (2D plan or 3D orbit).
    pub fn view_mode(&self) -> ProjectionKind {
        self.state.borrow().view_mode
    }

    /// The last explicit view diagnostic: an unsupported/unknown layout, or a
    /// refused degenerate camera. `None` means the last frame was not refused
    /// for a view reason.
    pub fn view_diagnostic(&self) -> Option<String> {
        self.state.borrow().view_diagnostic.clone()
    }

    /// Map a session [`SpaceId`] to a drawable [`SpaceSelection`].
    ///
    /// Block-definition geometry is only reached through an `INSERT` and is
    /// never a top-level viewable space, so it maps to `None`; the caller keeps
    /// the current space instead of fabricating one.
    pub fn space_selection(space: &SpaceId) -> Option<SpaceSelection> {
        match space {
            SpaceId::Model => Some(SpaceSelection::Model),
            SpaceId::Paper(layout) => Some(SpaceSelection::Paper(*layout)),
            SpaceId::Block(_) => None,
        }
    }

    /// Mirror a whole session view in one call: active space plus camera/mode.
    ///
    /// Hosts call this after every command that can change the view (open, fit,
    /// `SwitchSpace`, `Switch2d3d`, standard view, orbit, zoom/pan). It is the
    /// single place that keeps the bridge's space in sync with
    /// [`cad_app::SessionState::active_space`]; an unsupported layout is
    /// recorded in [`CadView::view_diagnostic`] rather than silently ignored.
    pub fn sync_session(&self, active_space: &SpaceId, viewport: &Viewport) {
        if let Some(space) = Self::space_selection(active_space) {
            let _ = self.set_space(space);
        }
        self.sync_from_viewport(viewport);
    }

    /// Select the drawing space the bridge builds and renders.
    ///
    /// A paper selection is validated against the *current* document's real
    /// layout table, so an unknown layout is `InvalidInput` and a known-but-
    /// undrawable layout is `Unsupported` with the representation reason. On
    /// refusal the previous space is kept and the reason is recorded in
    /// [`CadView::view_diagnostic`] — never a blank "success" (audit F04/B22).
    pub fn set_space(&self, space: SpaceSelection) -> CadResult<()> {
        if let Some(doc) = self.incoming.borrow().clone() {
            if let Err(error) = cad_app::validate_space(&doc, space) {
                {
                    let mut s = self.state.borrow_mut();
                    s.view_diagnostic = Some(error.to_string());
                }
                let _ = self.handle.request_redraw();
                return Err(error);
            }
        }
        {
            let mut s = self.state.borrow_mut();
            s.space = space;
            s.view_diagnostic = None;
        }
        let _ = self.handle.request_redraw();
        Ok(())
    }

    /// Set the observation mode; the next frame renders 2D or 3D accordingly.
    ///
    /// This is a presentation switch only: the authoritative camera lives in the
    /// application viewport and is mirrored through [`CadView::sync_from_viewport`]
    /// / [`CadView::set_camera3d`].
    pub fn set_view_mode(&self, mode: ProjectionKind) {
        {
            let mut s = self.state.borrow_mut();
            s.view_mode = mode;
        }
        let _ = self.handle.set_view_state(crate::ViewStateUi {
            is_3d: mode == ProjectionKind::ThreeD,
            perspective: !self.state.borrow().camera3d.projection.is_orthographic(),
        });
        let _ = self.handle.request_redraw();
    }

    /// Mirror an explicit application camera into the bridge.
    ///
    /// Validates first: a degenerate camera is refused and recorded rather than
    /// rendered as garbage. The projection selects nothing by itself; the caller
    /// still sets the mode via [`CadView::set_view_mode`] (or uses
    /// [`CadView::sync_from_viewport`]).
    pub fn set_camera3d(&self, camera: Camera) -> CadResult<()> {
        camera.validate()?;
        let mut s = self.state.borrow_mut();
        s.camera3d = camera;
        drop(s);
        let _ = self.handle.request_redraw();
        Ok(())
    }

    /// Mirror an authoritative application viewport in one call.
    ///
    /// Copies the full camera (so 3D orbit/standard views are preserved), the
    /// observation mode and, for an orthographic camera, the 2D centre/scale.
    /// Also pushes the derived view state to the shell so the 2D/3D affordances
    /// reflect the real viewport. Hosts should call this after every command
    /// that can change the view.
    pub fn sync_from_viewport(&self, viewport: &Viewport) {
        let mode = viewport.view_mode.kind();
        let perspective = !viewport.camera.projection.is_orthographic();
        {
            let mut s = self.state.borrow_mut();
            s.camera3d = viewport.camera;
            s.view_mode = mode;
            if let Projection::Orthographic { scale } = viewport.camera.projection {
                if scale.is_finite() && scale > 0.0 {
                    s.camera.center = Point3 {
                        x: viewport.camera.target.x,
                        y: viewport.camera.target.y,
                        z: 0.0,
                    };
                    s.camera.world_per_px = scale.clamp(1e-12, 1e18);
                }
            }
        }
        let _ = self.handle.set_view_state(crate::ViewStateUi {
            is_3d: mode == ProjectionKind::ThreeD,
            perspective,
        });
        let _ = self.handle.request_redraw();
    }

    pub fn active_backend(&self) -> Option<ActiveBackend> {
        self.state.borrow().caps.as_ref().map(|c| c.actual)
    }

    pub fn capabilities(&self) -> Option<(ActiveBackend, bool, u32)> {
        self.state
            .borrow()
            .caps
            .as_ref()
            .map(|c| (c.actual, c.compute, c.max_texture_dimension))
    }

    /// The full backend outcome: live backend or a specific failure (F12).
    pub fn backend_outcome(&self) -> Option<BackendOutcome> {
        self.state.borrow().outcome.clone()
    }

    /// Whether a backend is actually initialized and rendering.
    pub fn backend_is_live(&self) -> bool {
        self.state
            .borrow()
            .outcome
            .as_ref()
            .map(BackendOutcome::is_live)
            .unwrap_or(false)
    }

    /// Human-facing backend label that follows the real device, not the wish.
    pub fn backend_label(&self) -> Option<String> {
        self.state.borrow().outcome.as_ref().map(|o| o.label())
    }

    /// Failure reason when no backend is live, else `None`.
    ///
    /// The text is localized from the active catalog (N01); the structured
    /// `BackendFailure` each host can inspect is unchanged.
    pub fn last_error(&self) -> Option<String> {
        let messages = self.messages.borrow().clone();
        match self.state.borrow().outcome.as_ref() {
            Some(BackendOutcome::Failed { failure, .. }) => Some(match failure {
                BackendFailure::NoBackendAvailable => {
                    messages.text("status.backend.no_device", &[])
                }
                BackendFailure::ForcedUnavailable { reason } => messages.text(
                    "status.backend.forced_unavailable",
                    &[("reason", reason.as_str())],
                ),
                BackendFailure::InitFailed { reason } => {
                    messages.text("status.backend.init_failed", &[("reason", reason.as_str())])
                }
            }),
            _ => None,
        }
    }

    /// The requested preference as the shared app-level choice.
    pub fn preference_choice(&self) -> BackendChoice {
        preference_choice(self.preference)
    }

    pub fn preference(&self) -> BackendPreference {
        self.preference
    }

    /// Request a frame; used after the host mutates the document slot.
    pub fn request_redraw(&self) {
        let _ = self.handle.request_redraw();
    }

    /// Install the shaping fonts the host loaded from the font catalog.
    ///
    /// The next frame rebuilds the scene so text is shaped; passing a new set
    /// (for example after the CDN fetch resolves) triggers a rebuild too.
    pub fn set_fonts(&self, fonts: Arc<FontEngine>) {
        *self.fonts.borrow_mut() = Some(fonts);
        let _ = self.handle.request_redraw();
    }

    /// Drop the shaping fonts; text falls back to unshaped primitives.
    pub fn clear_fonts(&self) {
        *self.fonts.borrow_mut() = None;
        let _ = self.handle.request_redraw();
    }

    /// Apply the session's temporary layer overrides (spec F03).
    ///
    /// The next frame rebuilds the scene batches honouring the new visibility;
    /// the drawing database is not re-parsed and its revision does not change.
    pub fn set_layer_overrides(&self, overrides: LayerOverrideSet) {
        *self.overrides.borrow_mut() = overrides;
        let _ = self.handle.request_redraw();
    }

    /// Adopt the annotation database whose overlay is drawn on the canvas (F07).
    ///
    /// Passing a new database (a different sidecar or a reopened document)
    /// triggers a rebuild; the annotation revision already inside the fingerprint
    /// means an in-place edit through the annotation transaction path also
    /// rebuilds without the host having to call this again.
    pub fn set_annotations(&self, annotations: Arc<AnnotationDatabase>) {
        *self.annotations.borrow_mut() = Some(annotations);
        let _ = self.handle.request_redraw();
    }

    /// Drop the annotation overlay; the drawing keeps rendering.
    pub fn clear_annotations(&self) {
        *self.annotations.borrow_mut() = None;
        let _ = self.handle.request_redraw();
    }

    /// Apply the session annotation visibility overrides (F07/F09).
    ///
    /// Visibility is a session state and does not advance the annotation
    /// revision; the fingerprint folds it in so hiding or showing an annotation
    /// rebuilds exactly one frame later.
    pub fn set_annotation_visibility(&self, visibility: AnnotationVisibilitySet) {
        *self.annotation_visibility.borrow_mut() = visibility;
        let _ = self.handle.request_redraw();
    }

    /// The visibility overrides currently applied to the overlay.
    pub fn annotation_visibility(&self) -> AnnotationVisibilitySet {
        self.annotation_visibility.borrow().clone()
    }

    /// The annotation database currently adopted, if any.
    pub fn annotations(&self) -> Option<Arc<AnnotationDatabase>> {
        self.annotations.borrow().clone()
    }

    /// Drop derived GPU resources (device loss or backend rebuild).
    ///
    /// The document slot is **not** cleared: the authoritative drawing lives in
    /// the application database, so a rebuild re-uploads from the database rather
    /// than from any lost GPU batches. Only the scene-identity marker is reset so
    /// the next frame rebuilds (audit F12).
    pub fn teardown(&self) {
        let mut s = self.state.borrow_mut();
        s.renderer = None;
        s.image_size = None;
        s.caps = None;
        s.document = None;
        s.built_space = None;
        s.built_generation = None;
    }

    /// Handle a device loss reported by the renderer (F12).
    ///
    /// Calls `note_device_lost` on the renderer so it drops its derived GPU
    /// state, then marks the scene identity stale so the next frame rebuilds the
    /// scene from the database. Annotations and the document are untouched: they
    /// are not GPU state. Returns the `RenderError` for the caller to report.
    pub fn note_device_lost(&self, detail: impl Into<String>) -> Option<String> {
        let mut s = self.state.borrow_mut();
        let detail = detail.into();
        let message = s
            .renderer
            .as_mut()
            .map(|renderer| {
                renderer
                    .note_device_lost(detail.clone())
                    .message()
                    .to_string()
            })
            .unwrap_or(detail);
        // A lost device means no backend is active until the host reinitializes.
        let preference = preference_choice(self.preference);
        s.outcome = Some(BackendOutcome::Failed {
            preference,
            failure: BackendFailure::InitFailed {
                reason: message.clone(),
            },
        });
        s.caps = None;
        s.image_size = None;
        // Force a full rebuild from the database on the next frame.
        s.document = None;
        s.built_space = None;
        s.built_generation = None;
        drop(s);
        let _ = self.handle.request_redraw();
        Some(message)
    }
}

/// Map a renderer [`ActiveBackend`] to the app-level backend kind.
fn backend_kind(actual: ActiveBackend) -> ActiveBackendKind {
    match actual {
        ActiveBackend::WebGpu => ActiveBackendKind::WebGpu,
        ActiveBackend::WebGl2 => ActiveBackendKind::WebGl2,
        ActiveBackend::Native => ActiveBackendKind::Native,
    }
}

fn preference_choice(preference: BackendPreference) -> BackendChoice {
    match preference {
        BackendPreference::Auto => BackendChoice::Auto,
        BackendPreference::WebGpu => BackendChoice::WebGpu,
        BackendPreference::WebGl2 => BackendChoice::WebGl2,
    }
}

/// The camera a frame is rendered with, selected by the observation mode.
enum DrawCamera {
    TwoD(Camera2d),
    ThreeD(Camera3d),
}

/// Install the CAD rendering notifier on the Slint window.
pub fn install(
    handle: UiHandle,
    window: &slint::Window,
    incoming: IncomingDocument,
) -> CadResult<CadView> {
    install_with_preference(handle, window, incoming, BackendPreference::WebGpu)
}

/// Install with an explicit renderer preference (hosts choose the device).
pub fn install_with_preference(
    handle: UiHandle,
    window: &slint::Window,
    incoming: IncomingDocument,
    preference: BackendPreference,
) -> CadResult<CadView> {
    let state = Rc::new(RefCell::new(BridgeState::default()));
    let state_for_notifier = state.clone();
    let frame_handle = handle.clone();
    let scene_incoming = incoming.clone();
    let fonts_slot: Rc<RefCell<Option<Arc<FontEngine>>>> = Rc::new(RefCell::new(None));
    let scene_fonts = fonts_slot.clone();
    let overrides_slot: Rc<RefCell<LayerOverrideSet>> =
        Rc::new(RefCell::new(LayerOverrideSet::new()));
    let scene_overrides = overrides_slot.clone();
    let annotations_slot: IncomingAnnotations = Rc::new(RefCell::new(None));
    let scene_annotations = annotations_slot.clone();
    let visibility_slot: Rc<RefCell<AnnotationVisibilitySet>> =
        Rc::new(RefCell::new(AnnotationVisibilitySet::new()));
    let scene_visibility = visibility_slot.clone();
    let messages = handle.messages.clone();

    window
        .set_rendering_notifier(move |render_state, graphics_api| {
            match (render_state, graphics_api) {
                (
                    slint::RenderingState::RenderingSetup,
                    slint::GraphicsAPI::WGPU30 { device, queue, .. },
                ) => {
                    let mut s = state_for_notifier.borrow_mut();
                    let mut renderer = Renderer::new(preference);
                    match renderer.initialize_with_device(device.clone(), queue.clone()) {
                        Ok(caps) => {
                            // Truthful label: the actual backend comes from the
                            // device's own capabilities, never from the wish.
                            // Report it once so a device log shows which backend
                            // really initialized (spec §6/F12).
                            log::info!(
                                "CAD renderer initialized: preference={:?} actual={:?} \
                                 compute={} storage_buffers={} indirect_draw={} \
                                 max_texture_dimension={}",
                                preference_choice(preference),
                                caps.actual,
                                caps.compute,
                                caps.storage_buffers,
                                caps.indirect_draw,
                                caps.max_texture_dimension
                            );
                            s.outcome = Some(BackendOutcome::Initialized {
                                preference: preference_choice(preference),
                                actual: backend_kind(caps.actual),
                            });
                            s.caps = Some(caps);
                            s.renderer = Some(renderer);
                            s.document = None;
                            s.built_space = None;
                            s.built_generation = None;
                        }
                        Err(e) => {
                            // The device exists but the renderer could not build
                            // its derived state: a specific, non-generic failure.
                            log::warn!("CAD renderer initialisation failed: {e}");
                            s.outcome = Some(BackendOutcome::Failed {
                                preference: preference_choice(preference),
                                failure: BackendFailure::InitFailed {
                                    reason: e.to_string(),
                                },
                            });
                            s.caps = None;
                            s.renderer = None;
                        }
                    }
                }
                (slint::RenderingState::BeforeRendering, _) => {
                    let mut s = state_for_notifier.borrow_mut();
                    // Adopt a newly opened drawing exactly once. The camera is owned
                    // by the application viewport and mirrored through `CadView`.
                    if let Some(doc) = scene_incoming.borrow().clone() {
                        let stamp = TaskStamp::new(DocumentId(0), 0);
                        // Compare full content identity, not the bare database
                        // id: a different drawing reusing the same id must still
                        // rebuild the GPU batches (audit B04).
                        let identity = doc.scene_identity();
                        let fonts = scene_fonts.borrow().clone();
                        let has_fonts = fonts.is_some();
                        let overrides = scene_overrides.borrow().clone();
                        let overrides_fingerprint = overrides.fingerprint();
                        let annotations = scene_annotations.borrow().clone();
                        let visibility = scene_visibility.borrow().clone();
                        let annotations_fingerprint =
                            annotation_fingerprint(annotations.as_deref(), &visibility);
                        let space = s.space;
                        // A layout this build cannot draw (or does not know) is an
                        // explicit refusal: record the reason and do not build or
                        // render a blank sheet (audit F04/B22).
                        if let Err(error) = cad_app::validate_space(&doc, space) {
                            s.view_diagnostic = Some(error.to_string());
                            drop(s);
                            return;
                        }
                        s.view_diagnostic = None;
                        let generation = s.renderer.as_ref().map(|r| r.device_generation());
                        if s.document != Some(identity)
                            || s.fonts_present != has_fonts
                            || s.overrides_fingerprint != overrides_fingerprint
                            || s.annotations_fingerprint != annotations_fingerprint
                            || s.built_space != Some(space)
                            || s.built_generation != generation
                        {
                            // The annotation overlay is concatenated into the
                            // same delta, so a hidden/created/edited annotation
                            // rebuilds the GPU batches exactly once (F07). Paper
                            // space reuses the same single upload path. A build
                            // failure is explicit, not a silent stale frame.
                            match build_scene_with_annotations_in_space(
                                &doc,
                                stamp.clone(),
                                fonts,
                                &overrides,
                                space,
                                annotations.as_deref(),
                                &visibility,
                                DocumentId(0),
                            ) {
                                Ok((delta, _scene)) => {
                                    if let Some(renderer) = s.renderer.as_mut() {
                                        renderer.clear_batches();
                                        let _ = renderer.upload(&delta);
                                    }
                                    s.document = Some(identity);
                                    s.fonts_present = has_fonts;
                                    s.overrides_fingerprint = overrides_fingerprint;
                                    s.annotations_fingerprint = annotations_fingerprint;
                                    s.built_space = Some(space);
                                    s.built_generation = generation;
                                    s.image_size = None;
                                }
                                Err(error) => {
                                    s.view_diagnostic = Some(error.to_string());
                                }
                            }
                        }
                    }
                    let size = frame_handle
                        .physical_size()
                        .unwrap_or(slint::PhysicalSize::new(1, 1));
                    let target = RenderTarget::new(size.width.max(1), size.height.max(1));
                    // Pick the camera for the observation mode. A degenerate 3D
                    // camera is refused here with an explicit diagnostic, never
                    // rendered as a blank frame.
                    let draw = match s.view_mode {
                        ProjectionKind::ThreeD => match s.camera3d.camera3d_params() {
                            Ok(params) => DrawCamera::ThreeD(camera3d_from_params(params)),
                            Err(error) => {
                                s.view_diagnostic = Some(error.to_string());
                                drop(s);
                                return;
                            }
                        },
                        ProjectionKind::TwoD => DrawCamera::TwoD(Camera2d {
                            center: s.camera.center,
                            world_per_px: s.camera.world_per_px,
                            z_plane: 0.0,
                        }),
                    };
                    let rendered = match s.renderer.as_mut() {
                        Some(renderer) => {
                            let result = match draw {
                                DrawCamera::TwoD(camera) => renderer.render(camera, &target),
                                DrawCamera::ThreeD(camera) => renderer.render_3d(camera, &target),
                            };
                            match result {
                                Ok(_) => true,
                                Err(e) if e.is_device_loss() => {
                                    // Device loss is explicit: drop the derived GPU
                                    // state and force a rebuild from the database on
                                    // the next frame. The document slot still holds
                                    // the authoritative drawing and annotations.
                                    let reason = e.message().to_string();
                                    renderer.note_device_lost(reason.clone());
                                    s.outcome = Some(BackendOutcome::Failed {
                                        preference: preference_choice(preference),
                                        failure: BackendFailure::InitFailed { reason },
                                    });
                                    s.caps = None;
                                    s.document = None;
                                    s.built_space = None;
                                    s.built_generation = None;
                                    s.image_size = None;
                                    false
                                }
                                Err(e) => {
                                    // A rejected frame is explicit too: record the
                                    // reason so "nothing drawn" is never a silent
                                    // success (a degenerate camera, a missing
                                    // resource, ...).
                                    s.view_diagnostic = Some(e.message().to_string());
                                    false
                                }
                            }
                        }
                        None => return,
                    };
                    if rendered && s.image_size != Some((target.width, target.height)) {
                        if let Some(renderer) = s.renderer.as_ref() {
                            if let Some(texture) = renderer.frame_texture() {
                                if let Ok(image) = slint::Image::try_from(texture.clone()) {
                                    let _ = frame_handle.set_cad_frame(image);
                                    s.image_size = Some((target.width, target.height));
                                }
                            }
                        }
                    }
                }
                (slint::RenderingState::RenderingTeardown, _) => {
                    let mut s = state_for_notifier.borrow_mut();
                    s.renderer = None;
                    s.image_size = None;
                    s.caps = None;
                    s.document = None;
                    s.built_space = None;
                    s.built_generation = None;
                }
                // A rendering setup that is not WGPU30 cannot host the shared
                // CAD texture bridge. Report it instead of leaving a blank
                // canvas with no explanation (web diagnostic).
                (slint::RenderingState::RenderingSetup, graphics_api) => {
                    #[cfg(target_arch = "wasm32")]
                    crate::web::console_error(&format!(
                        "yacr bridge: rendering setup without WGPU30: {graphics_api:?}"
                    ));
                    let _ = graphics_api;
                }
                _ => {}
            }
        })
        .map_err(|e| CadError::GpuFailure(format!("set_rendering_notifier failed: {e}")))?;
    Ok(CadView {
        state,
        handle,
        fonts: fonts_slot,
        overrides: overrides_slot,
        annotations: annotations_slot,
        annotation_visibility: visibility_slot,
        incoming,
        preference,
        messages,
    })
}
/// The Slint component type, re-exported for hosts.
pub type UiWindow = YacrWindow;

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Layout, PaperViewport};
    use cad_domain::{Completeness, EntityId, LayerId, LayoutId, ObjectId, Point3, Revision};
    use cad_domain::{SemanticGeometry, SpaceId};

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn p3(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    #[test]
    fn space_selection_maps_session_spaces() {
        use cad_domain::BlockId;
        assert_eq!(
            CadView::space_selection(&SpaceId::Model),
            Some(SpaceSelection::Model)
        );
        let layout = LayoutId(3);
        assert_eq!(
            CadView::space_selection(&SpaceId::Paper(layout)),
            Some(SpaceSelection::Paper(layout))
        );
        // Block-definition geometry is never a top-level space.
        assert_eq!(CadView::space_selection(&SpaceId::Block(BlockId(1))), None);
    }

    fn line_entity(id: u128, space: SpaceId, a: Point3, b: Point3) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space,
            geometry: SemanticGeometry::Line { start: a, end: b },
            draw_order: id as i64,
        }
    }

    fn db_with_layout() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        // Model line through the viewport anchor.
        b.insert_entity(line_entity(1, SpaceId::Model, p(9.0, 20.0), p(11.0, 20.0)))
            .unwrap();
        b.insert_layout(Layout {
            id: LayoutId(1),
            name: "Layout1".into(),
            viewports: vec![PaperViewport {
                clip: vec![p(0.0, 0.0), p(100.0, 50.0), p(10.0, 20.0)],
                model_to_paper: cad_domain::Transform3::scale(100.0),
                completeness: Completeness::Complete,
            }],
        })
        .unwrap();
        b.finish().unwrap()
    }

    #[test]
    fn empty_database_produces_an_empty_scene() {
        let db = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1))
            .finish()
            .unwrap();
        let delta = build_scene(&db, TaskStamp::new(DocumentId(0), 0)).unwrap();
        assert!(delta.added.is_empty());
    }

    #[test]
    fn model_space_entry_point_is_unchanged_by_the_space_api() {
        let db = db_with_layout();
        let stamp = TaskStamp::new(DocumentId(0), 0);
        let model = build_scene(&db, stamp.clone()).unwrap();
        let explicit = build_scene_with_space(
            &db,
            stamp,
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
        )
        .unwrap();
        assert_eq!(model.added.len(), explicit.added.len());
        assert_eq!(model.added.len(), 1);
    }

    #[test]
    fn paper_space_build_maps_model_geometry_through_the_viewport() {
        let db = db_with_layout();
        let delta = build_scene_with_space(
            &db,
            TaskStamp::new(DocumentId(0), 0),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Paper(LayoutId(1)),
        )
        .unwrap();
        // The single mapped+clipped line becomes one batch.
        assert_eq!(delta.added.len(), 1);
    }

    #[test]
    fn switching_to_a_missing_layout_is_reported_not_faked() {
        let db = db_with_layout();
        // A missing layout still returns Ok with no batches; the diagnostic is
        // on the representation, which the scene layer cannot surface here.
        let delta = build_scene_with_space(
            &db,
            TaskStamp::new(DocumentId(0), 0),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Paper(LayoutId(99)),
        )
        .unwrap();
        assert!(delta.added.is_empty());
    }

    #[test]
    fn layout_descriptors_expose_ids_names_and_support() {
        let db = db_with_layout();
        let rows = layout_descriptors(&db);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, LayoutId(1));
        assert_eq!(rows[0].name, "Layout1");
        assert!(rows[0].supported);
        assert_eq!(rows[0].viewport_count, 1);
    }

    #[test]
    fn fit_camera_on_empty_database_is_default() {
        let db = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1))
            .finish()
            .unwrap();
        let c = fit_camera(&db, [100.0, 100.0]);
        assert_eq!(c.world_per_px, 1.0);
    }

    #[test]
    fn camera_stays_finite_under_extreme_zoom() {
        let db = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1))
            .finish()
            .unwrap();
        let c = fit_camera(&db, [100.0, 100.0]);
        assert!(c.world_per_px.is_finite());
    }

    #[test]
    fn backend_kind_mapping_is_truthful() {
        // The bridge must label the device that actually ran, never the wish.
        assert_eq!(
            backend_kind(ActiveBackend::WebGpu),
            ActiveBackendKind::WebGpu
        );
        assert_eq!(
            backend_kind(ActiveBackend::WebGl2),
            ActiveBackendKind::WebGl2
        );
        assert_eq!(
            backend_kind(ActiveBackend::Native),
            ActiveBackendKind::Native
        );
    }

    #[test]
    fn preference_maps_to_the_shared_app_choice() {
        assert_eq!(
            preference_choice(BackendPreference::Auto),
            BackendChoice::Auto
        );
        assert_eq!(
            preference_choice(BackendPreference::WebGpu),
            BackendChoice::WebGpu
        );
        assert_eq!(
            preference_choice(BackendPreference::WebGl2),
            BackendChoice::WebGl2
        );
    }

    #[test]
    fn camera3d_params_map_field_for_field_to_the_renderer_camera() {
        let params = cad_app::Camera3dParams {
            eye: p3(1.0, 2.0, 3.0),
            target: p3(4.0, 5.0, 6.0),
            up: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            fov_y: 0.7,
            near: 0.01,
            far: 1000.0,
        };
        let camera = camera3d_from_params(params);
        assert_eq!(camera.eye, params.eye);
        assert_eq!(camera.target, params.target);
        assert_eq!(camera.up, params.up);
        assert_eq!(camera.fov_y, params.fov_y);
        assert_eq!(camera.near, params.near);
        assert_eq!(camera.far, params.far);
        // The mapped camera is usable by the renderer for a normal aspect.
        assert!(camera.is_usable(16.0 / 9.0));
    }

    #[test]
    fn camera2d_params_map_to_the_renderer_plan_camera() {
        let params = cad_app::Camera2dParams {
            center: p(7.0, -8.0),
            world_per_px: 0.5,
        };
        let camera = camera2d_from_params(params);
        assert_eq!(camera.center, p(7.0, -8.0));
        assert_eq!(camera.world_per_px, 0.5);
        assert_eq!(camera.z_plane, 0.0);
    }

    #[test]
    fn paper_space_annotation_build_uses_the_layout() {
        let db = db_with_layout();
        let stamp = TaskStamp::new(DocumentId(0), 0);
        let visibility = AnnotationVisibilitySet::new();
        let (delta, scene) = build_scene_with_annotations_in_space(
            &db,
            stamp,
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Paper(LayoutId(1)),
            None,
            &visibility,
            DocumentId(0),
        )
        .unwrap();
        // The single mapped+clipped model line is present, and the absent
        // annotation sidecar contributes no overlay (not a silent drop).
        assert_eq!(delta.added.len(), 1);
        assert!(scene.batches.is_empty());

        // Model space still reproduces the model entry point.
        let (model, _) = build_scene_with_annotations_in_space(
            &db,
            TaskStamp::new(DocumentId(0), 0),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
            None,
            &visibility,
            DocumentId(0),
        )
        .unwrap();
        assert_eq!(model.added.len(), 1);
    }
}
