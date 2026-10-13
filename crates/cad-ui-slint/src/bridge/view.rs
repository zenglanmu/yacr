//! Slint-facing compatibility facade over the UI-independent render session.
use crate::UiHandle;
use cad_app::layers::LayerOverrideSet;
use cad_app::recovery::{BackendFailure, BackendOutcome};
use cad_app::render_scene::{DecodedImageCache, SnapHint};
use cad_app::viewer_config::OverlayVisibility;
use cad_app::{
    BackendChoice, Camera, MeasurementPreview, Projection, ProjectionKind, SelectionSet, Viewport,
};
use cad_db::DrawingDatabase;
use cad_domain::{CadResult, Point3, SpaceId};
use cad_engine::render::{BridgeCamera, RenderLifecycle, ViewSnapshot};
use cad_engine::render::{IncomingDocument, RenderSessionHandle, RenderWake};
use cad_render_wgpu::{ActiveBackend, BackendPreference};
use cad_representation::{FontEngine, SpaceSelection};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone)]
pub struct CadView {
    pub(super) session: RenderSessionHandle,
    pub(super) handle: UiHandle,
    preference: BackendPreference,
    messages: Rc<RefCell<crate::i18n::MessageSource>>,
    preparation_scheduled: Rc<Cell<bool>>,
}

impl CadView {
    pub(super) fn new(
        handle: UiHandle,
        incoming: IncomingDocument,
        preference: BackendPreference,
    ) -> Self {
        let session = RenderSessionHandle::with_incoming(incoming.clone(), preference);
        Self {
            session,
            messages: handle.messages.clone(),
            handle,
            preference,
            preparation_scheduled: Rc::new(Cell::new(false)),
        }
    }

    pub fn render_session(&self) -> RenderSessionHandle {
        self.session.clone()
    }

    pub fn sync_drawing(&self, drawing: Option<Arc<DrawingDatabase>>) {
        if self.session.sync_drawing(drawing) {
            self.request_redraw();
        }
    }

    pub fn apply_view_snapshot(&self, snapshot: ViewSnapshot) -> CadResult<()> {
        let perspective = !snapshot.camera.projection.is_orthographic();
        let is_3d = snapshot.mode == ProjectionKind::ThreeD;
        let standard_view = crate::state::standard_view_for_camera(&snapshot.camera);
        match self.session.apply_view_snapshot(snapshot) {
            Ok(true) => {
                self.handle.set_view_state(crate::ViewStateUi {
                    is_3d,
                    perspective,
                    standard_view,
                })?;
                self.request_redraw();
            }
            Ok(false) => {}
            Err(error) => {
                self.session.set_view_diagnostic(Some(error.to_string()));
                self.request_redraw();
                return Err(error);
            }
        }
        self.session.set_view_diagnostic(None);
        Ok(())
    }

    pub fn space_selection(space: &SpaceId) -> Option<SpaceSelection> {
        cad_engine::render::space_selection(space)
    }
    pub fn sync_session(&self, space: &SpaceId, viewport: &Viewport) {
        let Some(space) = Self::space_selection(space) else {
            self.session
                .set_view_diagnostic(Some("block space is not a top-level view".into()));
            return;
        };
        let snapshot = ViewSnapshot {
            document: viewport.document,
            space,
            camera: viewport.camera,
            mode: viewport.view_mode.kind(),
            canvas_size: viewport.logical_size,
        };
        if let Err(error) = self.apply_view_snapshot(snapshot) {
            log::warn!("view snapshot refused: {error}");
        }
    }
    pub fn camera(&self) -> BridgeCamera {
        let camera = self.session.camera();
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
        self.session.camera()
    }
    pub fn space(&self) -> SpaceSelection {
        self.session.space()
    }
    pub fn view_mode(&self) -> ProjectionKind {
        self.session.view_mode()
    }
    pub fn view_diagnostic(&self) -> Option<String> {
        self.session.view_diagnostic()
    }
    pub fn lifecycle(&self) -> RenderLifecycle {
        self.session.lifecycle()
    }
    pub fn frames_rendered(&self) -> u64 {
        self.session.frames_rendered()
    }
    pub fn current_drawing_presented(&self) -> bool {
        self.session.current_drawing_presented()
    }
    pub fn drawing_progress(&self) -> Option<(usize, usize)> {
        self.session.drawing_progress()
    }
    pub fn scene_error(&self) -> Option<String> {
        self.session.scene_error()
    }
    pub fn loading_phase(&self) -> Option<&'static str> {
        if !self.session.preparation_current() || self.session.preparation_busy() {
            Some("preparing")
        } else if self.session.uploading() {
            Some("uploading")
        } else if self.session.drawing_pages() {
            Some("drawing")
        } else {
            None
        }
    }
    pub fn active_backend(&self) -> Option<ActiveBackend> {
        self.session.active_backend()
    }
    pub fn capabilities(&self) -> Option<(ActiveBackend, bool, u32)> {
        self.session.capabilities()
    }
    pub fn backend_outcome(&self) -> Option<BackendOutcome> {
        self.session.backend_outcome()
    }
    pub fn backend_is_live(&self) -> bool {
        matches!(self.lifecycle(), RenderLifecycle::Ready { .. })
    }
    pub fn backend_label(&self) -> Option<String> {
        self.session
            .backend_label()
            .or_else(|| self.backend_outcome().map(|o| o.label()))
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

    pub fn request_redraw(&self) {
        self.session.note_redraw_request();
        if self.session.preparation_current() {
            self.session.wake_redraw(self);
            return;
        }
        if let Err(error) = self.session.request_preparation() {
            self.session.set_runtime_diagnostic(Some(error.to_string()));
        }
        if self.session.preparation_current() {
            self.session.wake_redraw(self);
            return;
        }
        self.session.schedule_preparation(self);
        if let Err(error) = slint::invoke_from_event_loop(|| {}) {
            log::warn!("CAD preparation wake failed: {error}");
        }
    }
    fn schedule_preparation(&self) {
        if self.preparation_scheduled.replace(true) {
            return;
        }
        let view = self.clone();
        slint::Timer::single_shot(std::time::Duration::from_millis(16), move || {
            view.preparation_scheduled.set(false);
            match view.session.poll_preparation() {
                Ok(true) => view.schedule_preparation(),
                Ok(false) => {}
                Err(error) => log::warn!("scene preparation refused: {error}"),
            }
            view.session.wake_redraw(&view);
        });
    }

    pub fn set_fonts(&self, fonts: Arc<FontEngine>) {
        if self.session.set_fonts(Some(fonts)) {
            self.request_redraw();
        }
    }
    pub fn fonts(&self) -> Option<Arc<FontEngine>> {
        self.session.fonts()
    }
    pub fn clear_fonts(&self) {
        if self.session.set_fonts(None) {
            self.request_redraw();
        }
    }
    pub fn set_images(&self, images: Arc<DecodedImageCache>) {
        if self.session.set_images(Some(images)) {
            self.request_redraw();
        }
    }
    pub fn clear_images(&self) {
        if self.session.set_images(None) {
            self.request_redraw();
        }
    }
    pub fn set_layer_overrides(&self, overrides: LayerOverrideSet) {
        if self.session.set_layers(overrides) {
            self.request_redraw();
        }
    }
    pub fn set_overlay_visibility(&self, visibility: OverlayVisibility) {
        if self.session.set_overlay_visibility(visibility) {
            self.request_redraw();
        }
    }
    pub fn overlay_visibility(&self) -> OverlayVisibility {
        self.session.overlay_visibility()
    }
    pub fn set_selection_highlight(&self, selection: SelectionSet) {
        let before = self.session.overlay_revision();
        self.session.set_selection(selection);
        if before != self.session.overlay_revision() {
            self.request_redraw();
        }
    }
    pub fn selection_highlight(&self) -> SelectionSet {
        self.session.selection()
    }
    pub fn set_measurement_preview(&self, preview: Option<MeasurementPreview>) {
        if self.session.set_measurement_preview(preview) {
            self.request_redraw();
        }
    }
    pub fn set_snap_hints(&self, hints: Vec<SnapHint>) {
        if self.session.set_snap_hints(hints) {
            self.request_redraw();
        }
    }
    pub fn snap_hints(&self) -> Vec<SnapHint> {
        self.session.snap_hints()
    }
    pub fn set_draw_preview(&self, preview: Option<cad_app::DrawPreview>) {
        if self.session.set_draw_preview(preview) {
            self.request_redraw();
        }
    }
    pub fn draw_preview(&self) -> Option<cad_app::DrawPreview> {
        self.session.draw_preview()
    }
    pub fn overlay_revision(&self) -> u64 {
        self.session.overlay_revision()
    }
    pub fn redraw_requests(&self) -> u64 {
        self.session.redraw_requests()
    }
    pub fn teardown(&self) {
        self.session.stop();
        let _ = self.handle.set_cad_frame(slint::Image::default());
    }
    pub fn note_device_lost(&self, detail: impl Into<String>) -> Option<String> {
        let reason = detail.into();
        self.session.fail_device(reason.clone(), true);
        let _ = self.handle.set_cad_frame(slint::Image::default());
        Some(reason)
    }

    pub(super) fn request_surface_redraw(&self) {
        let _ = self.handle.request_redraw();
    }
}

impl RenderWake for CadView {
    fn request_redraw(&self) {
        self.request_surface_redraw();
    }
    fn request_preparation(&self) {
        self.schedule_preparation();
    }
}
