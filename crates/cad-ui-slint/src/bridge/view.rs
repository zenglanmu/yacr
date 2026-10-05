//! Host-facing snapshots and coalesced CPU preparation, outside render callbacks.
use super::*;
use cad_app::render_scene::{OverlayInputs, SnapHint};
use cad_app::{MeasurementPreview, SelectionSet};

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
    /// Selection highlight input, kept separate so selecting an object never
    /// rebuilds the base drawing.
    selection: Rc<RefCell<SelectionSet>>,
    measurement_preview: Rc<RefCell<Option<MeasurementPreview>>>,
    /// Resolved object-snap hints fed by the host (from the measurement snap
    /// engine). Drawn as distinct snap-kind markers, gated by
    /// `overlay_visibility.snap_hints` in the controller. Empty by default.
    snap_hints: Rc<RefCell<Vec<SnapHint>>>,
    /// Which derived overlays the host wants drawn, mirrored from
    /// `ViewerConfig.view.overlays`. Defaults to all-on.
    overlay_visibility: Rc<RefCell<OverlayVisibility>>,
    /// In-progress drawing/editing preview (drawing-edit §4).
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
            selection: Rc::new(RefCell::new(SelectionSet::new())),
            measurement_preview: Rc::new(RefCell::new(None)),
            snap_hints: Rc::new(RefCell::new(Vec::new())),
            overlay_visibility: Rc::new(RefCell::new(OverlayVisibility::default())),
            draw_preview: Rc::new(RefCell::new(None)),
        }
    }

    /// Publish the authoritative drawing after commands (including undo/redo).
    /// Navigation and overlay-only updates keep the existing Arc and caches.
    pub fn sync_drawing(&self, drawing: Option<Arc<DrawingDatabase>>) {
        let changed = replace_drawing_snapshot(&mut self.incoming.borrow_mut(), drawing);
        if changed {
            self.request_redraw();
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
                    .and_then(|s| s.highlight_diagnostics.first().map(|d| d.message.clone()))
            })
    }
    pub fn lifecycle(&self) -> RenderLifecycle {
        self.state.borrow().runtime.lifecycle.clone()
    }
    pub fn frames_rendered(&self) -> u64 {
        self.state.borrow().runtime.frames_rendered
    }

    /// True only when the current immutable drawing input has been prepared,
    /// atomically uploaded, and actually rendered. Old demo/redraw frames do
    /// not count as completion of a new open.
    pub fn current_drawing_presented(&self) -> bool {
        let state = self.state.borrow();
        #[cfg(not(target_arch = "wasm32"))]
        if !state.preparation.current(&self.incoming.borrow()) {
            return false;
        }
        state.controller.ready.as_ref().is_some_and(|scene| {
            state.runtime.presented_revision == Some(scene.revision) && !state.runtime.uploading()
        })
    }

    pub fn drawing_progress(&self) -> Option<(usize, usize)> {
        self.state.borrow().runtime.drawing_progress()
    }

    pub fn scene_error(&self) -> Option<String> {
        let state = self.state.borrow();
        state
            .controller
            .diagnostic
            .clone()
            .or_else(|| state.runtime.diagnostic.clone())
    }

    pub fn loading_phase(&self) -> Option<&'static str> {
        let state = self.state.borrow();
        #[cfg(not(target_arch = "wasm32"))]
        if !state.preparation.current(&self.incoming.borrow()) {
            return Some("preparing");
        }
        if state.runtime.uploading() {
            return Some("uploading");
        }
        if state.runtime.drawing_pages() {
            return Some("drawing");
        }
        None
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
        if let Some(caps) = self.state.borrow().runtime.caps.as_ref() {
            return Some(caps.api.into());
        }
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
    /// Native preparation runs on a bounded worker; WASM retains the event-loop path.
    pub fn request_redraw(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        let input = self.preparation_input();
        {
            let mut state = self.state.borrow_mut();
            if state.preparation_stopped {
                return;
            }
            // Register the newest input before the next render callback. A
            // completed upload for replaced layers/fonts/overlays is not current.
            #[cfg(not(target_arch = "wasm32"))]
            match state.preparation.update(input) {
                Ok(Some(controller)) => state.controller = controller,
                Ok(None) => {}
                Err(error) => state.controller.diagnostic = Some(error.to_string()),
            }
            if state.preparation_scheduled {
                return;
            }
            state.preparation_scheduled = true;
        }
        // A single-shot UI timer can retain Rc snapshots without crossing threads.
        self.schedule_preparation();
        // File API/font callbacks can arrive while winit is idle. Scheduling
        // a Slint timer alone does not wake that browser event loop. Post an
        // event (without moving the Rc snapshots across threads) so it services
        // the timer; the timer requests a frame after publishing the CPU scene.
        if let Err(error) = slint::invoke_from_event_loop(|| {}) {
            log::warn!("CAD preparation wake failed: {error}");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn preparation_input(&self) -> super::preparation::PreparationInput {
        let snapshot = self.state.borrow().view.clone();
        super::preparation::PreparationInput {
            drawing: self.incoming.borrow().clone(),
            document: snapshot.document,
            space: snapshot.space,
            fonts: self.fonts.borrow().clone(),
            layers: self.overrides.borrow().clone(),
            overlays: OverlayInputs {
                selection: self.selection.borrow().clone(),
                measurement: self.measurement_preview.borrow().clone(),
                draw_preview: self.draw_preview.borrow().clone(),
                snap_hints_input: self.snap_hints.borrow().clone(),
                visibility: *self.overlay_visibility.borrow(),
            },
        }
    }

    fn schedule_preparation(&self) {
        let view = self.clone();
        slint::Timer::single_shot(std::time::Duration::from_millis(16), move || {
            let mut state = view.state.borrow_mut();
            if state.preparation_stopped {
                return;
            }
            state.preparation_scheduled = false;
            let doc = view.incoming.borrow().clone();
            let snapshot = state.view.clone();
            let overlays = OverlayInputs {
                selection: view.selection.borrow().clone(),
                measurement: view.measurement_preview.borrow().clone(),
                draw_preview: view.draw_preview.borrow().clone(),
                snap_hints_input: view.snap_hints.borrow().clone(),
                visibility: *view.overlay_visibility.borrow(),
            };
            #[cfg(target_arch = "wasm32")]
            let result = state.controller.prepare_shared_with_overlays(
                doc,
                snapshot.document,
                view.fonts.borrow().clone(),
                &view.overrides.borrow(),
                snapshot.space,
                &overlays,
            );
            #[cfg(not(target_arch = "wasm32"))]
            let result = state
                .preparation
                .update(super::preparation::PreparationInput {
                    drawing: doc,
                    document: snapshot.document,
                    space: snapshot.space,
                    fonts: view.fonts.borrow().clone(),
                    layers: view.overrides.borrow().clone(),
                    overlays,
                })
                .map(|controller| {
                    if let Some(controller) = controller {
                        state.controller = controller;
                    }
                });
            if let Err(error) = result {
                log::warn!("scene preparation refused: {error}");
                state.controller.diagnostic = Some(error.to_string());
            }
            #[cfg(not(target_arch = "wasm32"))]
            let busy = state.preparation.busy();
            #[cfg(target_arch = "wasm32")]
            let busy = false;
            state.preparation_scheduled = busy;
            drop(state);
            if let Err(error) = view.handle.request_redraw() {
                log::warn!("CAD redraw failed: {error}");
            }
            if busy {
                view.schedule_preparation();
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
    /// Store the overlay visibility mirrored from `ViewerConfig.view.overlays`
    /// and request a redraw.
    ///
    /// Visibility is part of the transient overlay fingerprint, so a toggle
    /// rebuilds only the highlight/preview delta and reuses the base drawing.
    pub fn set_overlay_visibility(&self, visibility: OverlayVisibility) {
        if *self.overlay_visibility.borrow() == visibility {
            return;
        }
        *self.overlay_visibility.borrow_mut() = visibility;
        self.state.borrow_mut().overlay_revision += 1;
        self.request_redraw();
    }

    /// The overlay visibility currently applied to rendering.
    pub fn overlay_visibility(&self) -> OverlayVisibility {
        *self.overlay_visibility.borrow()
    }

    /// Store the current selection highlight and request a redraw.
    ///
    /// This touches **only** the transient visual overlay: the controller keeps
    /// the base drawing `Arc` unless its own inputs changed, so changing the
    /// selection never re-parses the drawing or rebuilds base batches. It also
    /// does not advance the font revision.
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

    /// Store the resolved object-snap hints and request a redraw.
    ///
    /// These are the markers the measurement snap engine reports (endpoint,
    /// midpoint, center, …), drawn as distinct shapes by the transient overlay.
    /// An empty vector clears them. Like the selection and previews, this
    /// touches **only** the transient visual overlay: the base drawing `Arc` and
    /// the base drawing `Arc` is reused unless its own inputs changed.
    /// Visibility is still gated by
    /// `view.overlays.snapHints`, so a host can feed hints while the user has the
    /// overlay off.
    pub fn set_snap_hints(&self, hints: Vec<SnapHint>) {
        if *self.snap_hints.borrow() == hints {
            return;
        }
        *self.snap_hints.borrow_mut() = hints;
        self.state.borrow_mut().overlay_revision += 1;
        self.request_redraw();
    }

    /// The object-snap hints currently drawn as markers.
    pub fn snap_hints(&self) -> Vec<SnapHint> {
        self.snap_hints.borrow().clone()
    }

    /// Store (or clear) the in-progress drawing/editing preview (drawing-edit
    /// §4) and request a redraw. `None` cancels the preview overlay.
    ///
    /// The preview is drawn through the shared transient overlay (a
    /// rubber-band chain for line/move/trim, a full circle for circle);
    /// committing the actual geometry is the command layer's job, never this
    /// overlay's.
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
    /// independent of `fonts_changed`. A host or test can
    /// observe that selecting an object bumps this without rebuilding the base
    /// drawing.
    pub fn overlay_revision(&self) -> u64 {
        self.state.borrow().overlay_revision
    }
    pub fn teardown(&self) {
        {
            let mut state = self.state.borrow_mut();
            state.preparation_stopped = true;
            #[cfg(not(target_arch = "wasm32"))]
            state.preparation.stop();
        }
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

fn replace_drawing_snapshot(
    incoming: &mut Option<Arc<DrawingDatabase>>,
    drawing: Option<Arc<DrawingDatabase>>,
) -> bool {
    let same = match (incoming.as_ref(), drawing.as_ref()) {
        (Some(current), Some(next)) => Arc::ptr_eq(current, next),
        (None, None) => true,
        _ => false,
    };
    if !same {
        *incoming = drawing;
    }
    !same
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn drawing_commands_publish_new_snapshots_but_navigation_reuses_arc() {
        let mut host = cad_app::host::HostController::with_demo_document([1280.0, 800.0]).unwrap();
        let original = host.drawing().unwrap();
        let mut incoming = Some(original.clone());
        assert!(!replace_drawing_snapshot(&mut incoming, host.drawing()));
        host.create_line(
            cad_domain::Point3 {
                x: 10.0,
                y: 20.0,
                z: 0.0,
            },
            cad_domain::Point3 {
                x: 30.0,
                y: 40.0,
                z: 0.0,
            },
        )
        .unwrap();
        assert!(replace_drawing_snapshot(&mut incoming, host.drawing()));
        assert_eq!(
            incoming.as_ref().unwrap().entity_count(),
            original.entity_count() + 1
        );
        assert!(!replace_drawing_snapshot(&mut incoming, host.drawing()));
        assert!(replace_drawing_snapshot(&mut incoming, None));
        assert!(!replace_drawing_snapshot(&mut incoming, None));
    }
}
