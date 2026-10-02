//! Drawing replacement and explicit unsaved-work decisions.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use cad_app::host::HostController;
use cad_app::host_files::{export_annotations_atomically, parse_decision, resolve_leave};
use cad_domain::{CadError, ViewportId};
use cad_platform::Persistence;
use cad_ui_slint::{CadView, IncomingDocument, UiHandle};

use super::fonts::spawn_font_load;
use super::persistence::{download_text, WebPersistence};
use super::state_push;
use super::{sync_view_camera, viewport_camera, with_runtime};

fn install_opened(
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
    *incoming.borrow_mut() = drawing;
    {
        let c = controller.borrow();
        sync_view_camera(&c, view, viewport);
    }
    view.request_redraw();
    let _ = handle.set_status(format!("已打开 {name}: {}", opened.completeness_label));
    // Panels (layout/annotations/properties/diagnostics) are derived from the
    // new document; push them after the content swap so none shows stale data.
    state_push::push_panel_state_for_view(controller, handle, view);
    spawn_font_load();
}

/// JS must prompt before replacing a drawing with unsaved annotations.
pub fn open_needs_decision() -> bool {
    with_runtime(|rt| rt.controller.borrow().unsaved_signal().dirty).unwrap_or(false)
}

/// Save/recovery/discard/cancel are explicit; failed writes never replace data.
pub fn open_document_decided(name: &str, bytes: Vec<u8>, decision: &str) -> Result<String, String> {
    let decision = parse_decision(decision)
        .ok_or_else(|| format!("未知未保存决策：{decision}（save/recovery/discard/cancel）"))?;
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
    let (center, wpp) = {
        let c = controller.borrow();
        viewport_camera(&c)
    };
    let persistence = WebPersistence;
    let resolution = cad_platform::block_on(resolve_leave(
        &mut controller.borrow_mut(),
        decision,
        center,
        wpp,
        Some(&persistence as &dyn Persistence),
        |ctrl| {
            export_annotations_atomically(ctrl, |json| {
                download_text("annotations.cadnotes.json", json)
            })
        },
    ));
    let resolution = match resolution {
        Ok(resolution) => resolution,
        Err(CadError::Cancelled) => {
            let _ = handle.set_status(format!("已取消打开 {name}：当前文档与未保存批注保留"));
            return Err("cancelled".into());
        }
        Err(e) => {
            let _ = handle.set_status(format!("打开 {name} 失败：{e}"));
            return Err(e.to_string());
        }
    };
    let result = controller.borrow_mut().open_bytes_leaving(
        bytes,
        name,
        decision,
        resolution.saved,
        resolution.recovery_persisted,
    );
    match result {
        Ok(opened) => {
            install_opened(
                name,
                &opened,
                &controller,
                &handle,
                &view,
                &incoming,
                &viewport,
            );
            Ok(format!("已打开 {name}: {}", opened.completeness_label))
        }
        Err(e) => {
            let _ = handle.set_status(format!("打开 {name} 失败：{e}"));
            Err(e.to_string())
        }
    }
}

/// Conservative File API entry: refuses unsaved work without a decision.
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
    if controller.borrow().unsaved_signal().dirty {
        let _ = handle.set_status(format!(
            "打开 {name} 需要未保存决策：请选择保存/保留恢复/丢弃/取消"
        ));
        return Err("需要未保存决策".into());
    }
    let bytes: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
    let result = controller.borrow_mut().open_bytes(bytes, name);
    match result {
        Ok(opened) => {
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
        Err(e) => {
            let _ = handle.set_status(format!("打开 {name} 失败：{e}"));
            Err(e.to_string())
        }
    }
}
