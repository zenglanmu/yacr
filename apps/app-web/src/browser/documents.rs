//! Drawing replacement through the shared asynchronous import path.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use cad_app::host::HostController;
use cad_domain::ViewportId;
use cad_ui_slint::{CadView, IncomingDocument, UiHandle};

use super::fonts::spawn_font_load;
use super::state_push;
use super::{sync_view_camera, with_runtime};

/// Install a successfully opened drawing into the shared view/incoming slot.
///
/// Shared by the synchronous fallback and the asynchronous poll, so the
/// document swap, camera sync, panel push and font load happen in exactly one
/// place (no duplicated open path).
pub(super) fn install_opened(
    name: &str,
    opened: &cad_app::host::OpenedDrawing,
    controller: &Rc<RefCell<HostController>>,
    handle: &UiHandle,
    view: &CadView,
    incoming: &IncomingDocument,
    viewport: &ViewportId,
) {
    let drawing = {
        let mut c = controller.borrow_mut();
        let _ = c.fit();
        c.drawing()
    };
    let _ = handle.cancel_draw_capture();
    *incoming.borrow_mut() = drawing;
    {
        let c = controller.borrow();
        sync_view_camera(&c, view, viewport);
    }
    view.request_redraw();
    let _ = handle.set_status(format!("已打开 {name}: {}", opened.completeness_label));
    // Panels (layout/properties/diagnostics) are derived from the new document;
    // push them after the content swap so none shows stale data.
    state_push::push_panel_state_for_view(controller, handle, view);
    spawn_font_load();
}

/// Open a drawing from the File API / File System Access path.
pub fn open_document(name: &str, bytes: Vec<u8>) -> Result<(), String> {
    let (controller, handle, view, incoming, viewport) = with_runtime(|rt| {
        (
            rt.controller.clone(),
            rt.handle.clone(),
            rt.view.clone(),
            rt.incoming.clone(),
            rt.viewport,
        )
    })
    .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
    let bytes: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
    let started = super::async_open::start_or_apply(&controller, bytes, name);
    match started {
        Ok(super::async_open::OpenStart::Opened(opened)) => {
            install_opened(
                name,
                &opened,
                &controller,
                &handle,
                &view,
                &incoming,
                &viewport,
            );
            Ok(())
        }
        Ok(super::async_open::OpenStart::Started) => {
            let _ = handle.set_status(format!("正在后台打开 {name}…"));
            super::async_open::push_state(&controller, &handle);
            Ok(())
        }
        Err(e) => {
            let _ = handle.set_status(format!("打开 {name} 失败：{e}"));
            Err(e.to_string())
        }
    }
}
