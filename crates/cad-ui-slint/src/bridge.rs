//! Shared-device CAD composition. Slint coordinates presentation, not scene building.
use crate::{UiHandle, YacrWindow};
use cad_app::layers::LayerOverrideSet;
use cad_app::recovery::{ActiveBackendKind, BackendFailure, BackendOutcome};
use cad_app::{
    AnnotationVisibilitySet, BackendChoice, Camera, Projection, ProjectionKind, Viewport,
};
use cad_db::{AnnotationDatabase, DrawingDatabase};
use cad_domain::{CadError, CadResult, DocumentId, Point3, SpaceId};
use cad_render_wgpu::{
    ActiveBackend, BackendCapabilities, BackendPreference, RenderTarget, Renderer,
};
use cad_representation::{FontEngine, SpaceSelection};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

mod camera;
use cad_app::render_scene as controller;
mod presenter;
mod runtime;
mod view;
pub use cad_app::render_scene::{
    annotation_fingerprint, build_scene, build_scene_with_annotations,
    build_scene_with_annotations_in_space, build_scene_with_fonts, build_scene_with_overrides,
    build_scene_with_space, layout_descriptors,
};
pub use camera::{camera2d_from_params, camera3d_from_params, fit_camera, BridgeCamera};
pub use runtime::{FrameBinding, RenderLifecycle};
pub use view::{CadView, ViewSnapshot};

/// Current document, not a one-shot mailbox. `None` means no open document.
pub type IncomingDocument = Rc<RefCell<Option<Arc<DrawingDatabase>>>>;
pub type IncomingAnnotations = Rc<RefCell<Option<Arc<AnnotationDatabase>>>>;

#[derive(Default)]
struct BridgeState {
    controller: controller::CadSceneController,
    runtime: runtime::CadRenderRuntime,
    presenter: presenter::SlintPresenter,
    view: ViewSnapshot,
    preparation_scheduled: bool,
    view_diagnostic: Option<String>,
}

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

pub fn install(
    handle: UiHandle,
    window: &slint::Window,
    incoming: IncomingDocument,
) -> CadResult<CadView> {
    install_with_preference(handle, window, incoming, BackendPreference::WebGpu)
}

pub fn install_with_preference(
    handle: UiHandle,
    window: &slint::Window,
    incoming: IncomingDocument,
    preference: BackendPreference,
) -> CadResult<CadView> {
    let view = CadView::new(handle, incoming, preference);
    let state = view.state.clone();
    let frame_handle = view.handle.clone();
    window
        .set_rendering_notifier(move |phase, api| {
            let mut state = state.borrow_mut();
            match (phase, api) {
                (
                    slint::RenderingState::RenderingSetup,
                    slint::GraphicsAPI::WGPU30 { device, queue, .. },
                ) => {
                    state
                        .runtime
                        .attach(device.clone(), queue.clone(), preference);
                    if let Err(error) = state.presenter.reset(&frame_handle) {
                        state.runtime.diagnostic = Some(error.to_string());
                    }
                }
                (slint::RenderingState::RenderingSetup, api) => {
                    state.runtime.fail(
                        preference,
                        format!("unsupported Slint graphics API: {api:?}"),
                        false,
                    );
                }
                (slint::RenderingState::BeforeRendering, _) => {
                    // Only GPU synchronization and presentation belong to this callback.
                    let BridgeState {
                        controller,
                        runtime,
                        presenter,
                        view,
                        ..
                    } = &mut *state;
                    if let Err(error) = runtime.apply_ready_updates(controller.ready.as_ref()) {
                        runtime.diagnostic = Some(error.to_string());
                        return;
                    }
                    let (logical, scale) =
                        frame_handle.cad_surface_size().unwrap_or(([0.0, 0.0], 1.0));
                    match runtime.render_if_dirty(view, logical, scale) {
                        Ok(Some(frame)) => {
                            match presenter.bind_frame_if_changed(&frame_handle, frame) {
                                Ok(()) => runtime.diagnostic = None,
                                Err(error) => runtime.diagnostic = Some(error.to_string()),
                            }
                        }
                        Ok(None) => {}
                        Err(error) if error.is_device_loss() => {
                            runtime.fail(preference, error.message().into(), true);
                            if let Err(error) = presenter.reset(&frame_handle) {
                                runtime.diagnostic = Some(error.to_string());
                            }
                        }
                        Err(error) => runtime.diagnostic = Some(error.to_string()),
                    }
                }
                (slint::RenderingState::RenderingTeardown, _) => {
                    state.runtime.detach();
                    if let Err(error) = state.presenter.reset(&frame_handle) {
                        state.runtime.diagnostic = Some(error.to_string());
                    }
                }
                _ => {}
            }
        })
        .map_err(|e| CadError::GpuFailure(format!("set_rendering_notifier failed: {e}")))?;
    view.request_redraw();
    Ok(view)
}

pub type UiWindow = YacrWindow;
#[cfg(test)]
mod tests;
