//! Host-facing snapshots and coalesced CPU preparation, outside render callbacks.
use super::*;
use cad_app::render_scene::OverlayInputs;
use cad_app::{AnnotationPreview, MeasurementPreview, SelectionSet};

#[derive(Clone, Debug, PartialEq)]
pub struct ViewSnapshot {
    pub document: DocumentId,
    pub space: SpaceSelection,
    pub camera: Camera,
    pub mode: ProjectionKind,
}
impl Default for ViewSnapshot {
    fn default() -> Self {
        Self {
            document: DocumentId(0),
            space: SpaceSelection::Model,
            camera: Camera::top_view_2d(),
            mode: ProjectionKind::TwoD,
        }
    }
}

#[derive(Clone)]
pub struct CadView {
    pub(super) state: Rc<RefCell<BridgeState>>,
    pub(super) handle: UiHandle,
    fonts: Rc<RefCell<Option<Arc<FontEngine>>>>,
    overrides: Rc<RefCell<LayerOverrideSet>>,
    annotations: IncomingAnnotations,
    visibility: Rc<RefCell<AnnotationVisibilitySet>>,
    /// Selection highlight input, kept separate from annotations so selecting an
    /// object never rebuilds the base drawing or the annotation overlay.
    selection: Rc<RefCell<SelectionSet>>,
    measurement_preview: Rc<RefCell<Option<MeasurementPreview>>>,
    annotation_preview: Rc<RefCell<Option<AnnotationPreview>>>,
    /// In-progress drawing/editing preview (drawing-edit §4); drawn through the
    /// existing annotation preview overlay path.
    draw_preview: Rc<RefCell<Option<cad_app::DrawPreview>>>,
    incoming: IncomingDocument,
    preference: BackendPreference,
    messages: Rc<RefCell<crate::i18n::MessageSource>>,
}
impl CadView {
    pub(super) fn new(
        handle: UiHandle,
        incoming: IncomingDocument,
        preference: BackendPreference,
    ) -> Self {
        Self {
            messages: handle.messages.clone(),
            handle,
            incoming,
            preference,
            state: Rc::new(RefCell::new(BridgeState::default())),
            fonts: Rc::new(RefCell::new(None)),
            overrides: Rc::new(RefCell::new(LayerOverrideSet::new())),
            annotations: Rc::new(RefCell::new(None)),
            visibility: Rc::new(RefCell::new(AnnotationVisibilitySet::new())),
            selection: Rc::new(RefCell::new(SelectionSet::new())),
            measurement_preview: Rc::new(RefCell::new(None)),
            annotation_preview: Rc::new(RefCell::new(None)),
            draw_preview: Rc::new(RefCell::new(None)),
        }
    }

    /// Validate the complete view before replacing any mirror; a refused space
    /// never installs the camera belonging to that space.
    pub fn apply_view_snapshot(&self, snapshot: ViewSnapshot) -> CadResult<()> {
        let result = self.validate_snapshot(&snapshot);
        if let Err(error) = result {
            self.state.borrow_mut().view_diagnostic = Some(error.to_string());
            self.handle.request_redraw()?;
            return Err(error);
        }
        let perspective = !snapshot.camera.projection.is_orthographic();
        let is_3d = snapshot.mode == ProjectionKind::ThreeD;
        {
            let mut state = self.state.borrow_mut();
            if state.view != snapshot {
                state.runtime.dirty.invalidate();
            }
            state.view = snapshot;
            state.view_diagnostic = None;
        }
        self.handle
            .set_view_state(crate::ViewStateUi { is_3d, perspective })?;
        self.request_redraw();
        Ok(())
    }

    fn validate_snapshot(&self, snapshot: &ViewSnapshot) -> CadResult<()> {
        snapshot.camera.validate()?;
        if snapshot.mode == ProjectionKind::TwoD {
            snapshot.camera.camera2d_params()?;
        }
        if let Some(doc) = self.incoming.borrow().as_ref() {
            cad_app::validate_space(doc, snapshot.space)?;
        }
        Ok(())
    }

    pub fn space_selection(space: &SpaceId) -> Option<SpaceSelection> {
        match space {
            SpaceId::Model => Some(SpaceSelection::Model),
            SpaceId::Paper(layout) => Some(SpaceSelection::Paper(*layout)),
            SpaceId::Block(_) => None,
        }
    }
    pub fn sync_session(&self, space: &SpaceId, viewport: &Viewport) {
        let Some(space) = Self::space_selection(space) else {
            self.state.borrow_mut().view_diagnostic =
                Some("block space is not a top-level view".into());
            return;
        };
        let snapshot = self.snapshot_from_viewport(viewport, space);
        if let Err(error) = self.apply_view_snapshot(snapshot) {
            log::warn!("view snapshot refused: {error}");
        }
    }
    fn snapshot_from_viewport(&self, viewport: &Viewport, space: SpaceSelection) -> ViewSnapshot {
        ViewSnapshot {
            document: viewport.document,
            space,
            camera: viewport.camera,
            mode: viewport.view_mode.kind(),
        }
    }
    pub fn camera(&self) -> BridgeCamera {
        let camera = self.state.borrow().view.camera;
        BridgeCamera {
            center: Point3 {
                z: 0.0,
                ..camera.target
            },
            world_per_px: match camera.projection {
                Projection::Orthographic { scale } => scale,
                Projection::Perspective { .. } => BridgeCamera::default().world_per_px,
            },
        }
    }
    pub fn camera3d(&self) -> Camera {
        self.state.borrow().view.camera
    }
    pub fn space(&self) -> SpaceSelection {
        self.state.borrow().view.space
    }
    pub fn view_mode(&self) -> ProjectionKind {
        self.state.borrow().view.mode
    }
    pub fn view_diagnostic(&self) -> Option<String> {
        let state = self.state.borrow();
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
                    .and_then(|s| s.annotation_diagnostics.first().map(|d| d.message.clone()))
            })
            .or_else(|| {
                state
                    .controller
                    .ready
                    .as_ref()
                    .and_then(|s| s.highlight_diagnostics.first().map(|d| d.message.clone()))
            })
    }
    pub fn lifecycle(&self) -> RenderLifecycle {
        self.state.borrow().runtime.lifecycle.clone()
    }
    pub fn frames_rendered(&self) -> u64 {
        self.state.borrow().runtime.frames_rendered
    }
    pub fn active_backend(&self) -> Option<ActiveBackend> {
        self.state.borrow().runtime.caps.as_ref().map(|c| c.actual)
    }
    pub fn capabilities(&self) -> Option<(ActiveBackend, bool, u32)> {
        self.state
            .borrow()
            .runtime
            .caps
            .as_ref()
            .map(|c| (c.actual, c.compute, c.max_texture_dimension))
    }
    pub fn backend_outcome(&self) -> Option<BackendOutcome> {
        self.state.borrow().runtime.outcome.clone()
    }
    pub fn backend_is_live(&self) -> bool {
        matches!(self.lifecycle(), RenderLifecycle::Ready { .. })
    }
    pub fn backend_label(&self) -> Option<String> {
        self.backend_outcome().map(|o| o.label())
    }
    pub fn last_error(&self) -> Option<String> {
        let messages = self.messages.borrow();
        match self.backend_outcome() {
            Some(BackendOutcome::Failed { failure, .. }) => Some(match failure {
                BackendFailure::NoBackendAvailable => {
                    messages.text("status.backend.no_device", &[])
                }
                BackendFailure::ForcedUnavailable { reason } => {
                    messages.text("status.backend.forced_unavailable", &[("reason", &reason)])
                }
                BackendFailure::InitFailed { reason } => {
                    messages.text("status.backend.init_failed", &[("reason", &reason)])
                }
            }),
            _ => None,
        }
    }
    pub fn preference(&self) -> BackendPreference {
        self.preference
    }
    pub fn preference_choice(&self) -> BackendChoice {
        super::preference_choice(self.preference)
    }

    /// Coalesce host changes into a CPU preparation task before presentation.
    /// This is outside `BeforeRendering`; no database walk occurs on a UI-only frame.
    /// Currently runs on the event loop, not a worker: large builds may still block it.
    pub fn request_redraw(&self) {
        {
            let mut state = self.state.borrow_mut();
            if state.preparation_scheduled {
                return;
            }
            state.preparation_scheduled = true;
        }
        // A single-shot UI timer can retain Rc snapshots without crossing threads.
        self.schedule_preparation();
    }

    fn schedule_preparation(&self) {
        let view = self.clone();
        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
            let mut state = view.state.borrow_mut();
            state.preparation_scheduled = false;
            let doc = view.incoming.borrow().clone();
            let snapshot = state.view.clone();
            // A real annotation tool preview wins; otherwise an in-progress
            // drawing preview is drawn through the same overlay path.
            let annotation_overlay = view.annotation_preview.borrow().clone().or_else(|| {
                view.draw_preview
                    .borrow()
                    .as_ref()
                    .map(crate::draw::draw_overlay_preview)
            });
            let overlays = OverlayInputs {
                selection: view.selection.borrow().clone(),
                measurement: view.measurement_preview.borrow().clone(),
                annotation: annotation_overlay,
            };
            let result = state.controller.prepare_shared_with_overlays(
                doc,
                snapshot.document,
                view.fonts.borrow().clone(),
                &view.overrides.borrow(),
                snapshot.space,
                view.annotations.borrow().as_deref(),
                &view.visibility.borrow(),
                &overlays,
            );
            if let Err(error) = result {
                log::warn!("scene preparation refused: {error}");
            }
            drop(state);
            if let Err(error) = view.handle.request_redraw() {
                log::warn!("CAD redraw failed: {error}");
            }
        });
    }
    pub fn set_fonts(&self, fonts: Arc<FontEngine>) {
        *self.fonts.borrow_mut() = Some(fonts);
        self.state.borrow_mut().controller.fonts_changed();
        self.request_redraw();
    }
    pub fn clear_fonts(&self) {
        *self.fonts.borrow_mut() = None;
        self.state.borrow_mut().controller.fonts_changed();
        self.request_redraw();
    }
    pub fn set_layer_overrides(&self, overrides: LayerOverrideSet) {
        *self.overrides.borrow_mut() = overrides;
        self.request_redraw();
    }
    pub fn set_annotations(&self, annotations: Arc<AnnotationDatabase>) {
        if !self
            .annotations
            .borrow()
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &annotations))
        {
            self.state.borrow_mut().controller.annotations_changed();
        }
        *self.annotations.borrow_mut() = Some(annotations);
        self.request_redraw();
    }
    pub fn clear_annotations(&self) {
        if self.annotations.borrow().is_some() {
            self.state.borrow_mut().controller.annotations_changed();
        }
        *self.annotations.borrow_mut() = None;
        self.request_redraw();
    }
    pub fn set_annotation_visibility(&self, visibility: AnnotationVisibilitySet) {
        *self.visibility.borrow_mut() = visibility;
        self.request_redraw();
    }
    pub fn annotation_visibility(&self) -> AnnotationVisibilitySet {
        self.visibility.borrow().clone()
    }
    pub fn annotations(&self) -> Option<Arc<AnnotationDatabase>> {
        self.annotations.borrow().clone()
    }

    /// Store the current selection highlight and request a redraw.
    ///
    /// This touches **only** the transient visual overlay: the controller keeps
    /// the base drawing `Arc` and the annotation overlay `Arc` unless their own
    /// inputs changed, so changing the selection never re-parses the drawing or
    /// rebuilds annotation batches. It also does not advance the annotation or
    /// font revisions.
    pub fn set_selection_highlight(&self, selection: SelectionSet) {
        if *self.selection.borrow() == selection {
            return;
        }
        *self.selection.borrow_mut() = selection;
        self.state.borrow_mut().overlay_revision += 1;
        self.request_redraw();
    }

    /// The selection currently drawn as a highlight.
    pub fn selection_highlight(&self) -> SelectionSet {
        self.selection.borrow().clone()
    }

    /// Store (or clear) the in-progress measurement preview and request a
    /// redraw. `None` cancels the preview overlay.
    pub fn set_measurement_preview(&self, preview: Option<MeasurementPreview>) {
        if *self.measurement_preview.borrow() == preview {
            return;
        }
        *self.measurement_preview.borrow_mut() = preview;
        self.state.borrow_mut().overlay_revision += 1;
        self.request_redraw();
    }

    /// Store (or clear) the in-progress annotation preview and request a redraw.
    pub fn set_annotation_preview(&self, preview: Option<AnnotationPreview>) {
        if *self.annotation_preview.borrow() == preview {
            return;
        }
        *self.annotation_preview.borrow_mut() = preview;
        self.state.borrow_mut().overlay_revision += 1;
        self.request_redraw();
    }

    /// Store (or clear) the in-progress drawing/editing preview (drawing-edit
    /// §4) and request a redraw. `None` cancels the preview overlay.
    ///
    /// The preview is drawn through the existing annotation preview overlay (a
    /// rubber band for line/move/trim, a full circle for circle); committing
    /// the actual geometry is the command layer's job, never this overlay's.
    pub fn set_draw_preview(&self, preview: Option<cad_app::DrawPreview>) {
        if *self.draw_preview.borrow() == preview {
            return;
        }
        *self.draw_preview.borrow_mut() = preview;
        self.state.borrow_mut().overlay_revision += 1;
        self.request_redraw();
    }

    /// The drawing preview currently drawn as an overlay.
    pub fn draw_preview(&self) -> Option<cad_app::DrawPreview> {
        self.draw_preview.borrow().clone()
    }

    /// A monotonic counter of transient overlay (selection/preview) changes,
    /// independent of `annotations_changed`/`fonts_changed`. A host or test can
    /// observe that selecting an object bumps this without rebuilding the base
    /// drawing.
    pub fn overlay_revision(&self) -> u64 {
        self.state.borrow().overlay_revision
    }
    pub fn teardown(&self) {
        let mut state = self.state.borrow_mut();
        state.runtime.detach();
        if let Err(error) = state.presenter.reset(&self.handle) {
            state.runtime.diagnostic = Some(error.to_string());
        }
    }
    /// A lost device is never retried by redraw. Only a fresh RenderingSetup
    /// from Slint can attach a device and re-upload the retained CPU scene.
    pub fn note_device_lost(&self, detail: impl Into<String>) -> Option<String> {
        let reason = detail.into();
        let mut state = self.state.borrow_mut();
        state.runtime.fail(self.preference, reason.clone(), true);
        if let Err(error) = state.presenter.reset(&self.handle) {
            state.runtime.diagnostic = Some(error.to_string());
        }
        Some(reason)
    }
}
