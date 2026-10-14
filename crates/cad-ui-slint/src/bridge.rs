//! Shared-device CAD composition. Slint owns only scheduling and presentation.
use crate::{UiHandle, YacrWindow};
use cad_app::BackendChoice;
use cad_domain::{CadError, CadResult};
use cad_engine::render;
use cad_render_wgpu::BackendPreference;
use std::cell::RefCell;
use std::rc::Rc;

mod presenter;
mod view;
pub use cad_app::render_scene::{
    build_scene, build_scene_with_fonts, build_scene_with_overrides, build_scene_with_space,
    layout_descriptors, overlay_fingerprint, preview_overlay, selection_highlight,
    snap_hint_overlay, OverlayInputs, PreviewOptions, SnapHint, SnapHintKind, VisualOverlay,
};
pub use cad_app::viewer_config::OverlayVisibility;
pub use render::IncomingDocument;
pub use render::{
    camera2d_from_params, camera3d_from_params, fit_camera, BridgeCamera, FrameBinding,
    RenderLifecycle, ViewSnapshot,
};
pub use view::CadView;

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
    let session = render::RenderSessionHandle::with_incoming(incoming, preference);
    install_existing(handle, window, session)
}

/// Install Slint's renderer hooks around a render session owned by the engine/client.
pub fn install_existing(
    handle: UiHandle,
    window: &slint::Window,
    session: render::RenderSessionHandle,
) -> CadResult<CadView> {
    let view = CadView::from_session(handle, session);
    let session = view.session.clone();
    let presenter = Rc::new(RefCell::new(presenter::SlintPresenter::default()));
    let frame_handle = view.handle.clone();
    let frame_presenter = presenter.clone();
    window
        .set_rendering_notifier(move |phase, api| match (phase, api) {
            (
                slint::RenderingState::RenderingSetup,
                slint::GraphicsAPI::WGPU30 { device, queue, .. },
            ) => {
                if let Err(reason) = session.attach_device(device.clone(), queue.clone()) {
                    log::warn!("CAD renderer attach failed: {reason}");
                }
                frame_presenter.borrow_mut().invalidate();
            }
            (slint::RenderingState::RenderingSetup, api) => {
                session.fail_device(format!("unsupported Slint graphics API: {api:?}"), false);
            }
            (slint::RenderingState::BeforeRendering, _) => {
                if let Err(error) = session.apply_ready_updates() {
                    session.set_runtime_diagnostic(Some(error.to_string()));
                    return;
                }
                let (logical, scale) = frame_handle.cad_surface_size().unwrap_or(([0.0, 0.0], 1.0));
                match session.render(logical, scale) {
                    Ok(Some(frame)) => match frame_presenter
                        .borrow_mut()
                        .bind_frame_if_changed(&frame_handle, frame)
                    {
                        Ok(()) => session.set_runtime_diagnostic(None),
                        Err(error) => session.set_runtime_diagnostic(Some(error.to_string())),
                    },
                    Ok(None) => {}
                    Err(error) if error.is_device_loss() => {
                        session.fail_device(error.message(), true);
                        frame_presenter.borrow_mut().invalidate();
                    }
                    Err(error) => session.set_runtime_diagnostic(Some(error.to_string())),
                }
                if session.uploading() || session.drawing_pages() {
                    let _ = frame_handle.request_redraw();
                }
            }
            (slint::RenderingState::RenderingTeardown, _) => {
                session.detach_device();
                frame_presenter.borrow_mut().invalidate();
            }
            _ => {}
        })
        .map_err(|e| CadError::GpuFailure(format!("set_rendering_notifier failed: {e}")))?;
    view.request_redraw();
    Ok(view)
}

pub type UiWindow = YacrWindow;
