//! Shared-device CAD composition. Slint coordinates presentation, not scene building.
use crate::{UiHandle, YacrWindow};
use cad_app::layers::LayerOverrideSet;
use cad_app::recovery::{ActiveBackendKind, BackendFailure, BackendOutcome};
use cad_app::{BackendChoice, Camera, Projection, ProjectionKind, Viewport};
use cad_db::DrawingDatabase;
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
#[cfg(not(target_arch = "wasm32"))]
mod preparation;
mod presenter;
mod runtime;
mod view;
pub use cad_app::render_scene::{
    build_scene, build_scene_with_fonts, build_scene_with_overrides, build_scene_with_space,
    layout_descriptors, overlay_fingerprint, preview_overlay, selection_highlight,
    snap_hint_overlay, OverlayInputs, PreviewOptions, SnapHint, SnapHintKind, VisualOverlay,
};
pub use cad_app::viewer_config::OverlayVisibility;
pub use camera::{camera2d_from_params, camera3d_from_params, fit_camera, BridgeCamera};
pub use runtime::{FrameBinding, RenderLifecycle};
pub use view::{CadView, ViewSnapshot};

/// Current document, not a one-shot mailbox. `None` means no open document.
pub type IncomingDocument = Rc<RefCell<Option<Arc<DrawingDatabase>>>>;

#[derive(Default)]
struct BridgeState {
    controller: controller::CadSceneController,
    runtime: runtime::CadRenderRuntime,
    presenter: presenter::SlintPresenter,
    view: ViewSnapshot,
    preparation_scheduled: bool,
    preparation_stopped: bool,
    #[cfg(not(target_arch = "wasm32"))]
    preparation: preparation::NativePreparation,
    view_diagnostic: Option<String>,
    /// Separate dirty marker for the transient selection/preview overlay. It
    /// advances on a highlight/preview change and never on a drawing revision
    /// change.
    overlay_revision: u64,
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
    #[cfg(not(target_arch = "wasm32"))]
    let frame_incoming = incoming.clone();
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
                    state.presenter.invalidate();
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
                    #[cfg(not(target_arch = "wasm32"))]
                    let current = state.preparation.current(&frame_incoming.borrow());
                    #[cfg(target_arch = "wasm32")]
                    let current = true;
                    let BridgeState {
                        controller,
                        runtime,
                        presenter,
                        view,
                        ..
                    } = &mut *state;
                    if let Err(error) = runtime.apply_ready_updates(if current {
                        controller.ready.as_ref()
                    } else {
                        None
                    }) {
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
                            presenter.invalidate();
                        }
                        Err(error) => runtime.diagnostic = Some(error.to_string()),
                    }
                    if runtime.uploading() || runtime.drawing_pages() {
                        let _ = frame_handle.request_redraw();
                    }
                }
                (slint::RenderingState::RenderingTeardown, _) => {
                    state.runtime.detach();
                    state.presenter.invalidate();
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
