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
            // An explicit cancelled terminal keeps the progress panel truthful
            // even though the synchronous fallback never started an import.
            super::async_open::record_cancelled();
            super::async_open::push_state(&controller, &handle);
            return Err("cancelled".into());
        }
        Err(e) => {
            let _ = handle.set_status(format!("打开 {name} 失败：{e}"));
            return Err(e.to_string());
        }
    };

    let started = super::async_open::start_or_apply(
        &controller,
        bytes,
        name,
        decision,
        resolution.saved,
        resolution.recovery_persisted,
    );
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
            Ok(format!("已打开 {name}: {}", opened.completeness_label))
        }
        Ok(super::async_open::OpenStart::Started) => {
            // A cancellable background job is running; the poll heartbeat will
            // install the document. Push the running panel now so it is visible
            // before the next heartbeat, not one tick later.
            super::async_open::push_state(&controller, &handle);
            Ok(format!("正在后台打开 {name}…"))
        }
        Err(CadError::Cancelled) => {
            let _ = handle.set_status(format!("已取消打开 {name}：当前文档与未保存批注保留"));
            Err("cancelled".into())
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
    let started = super::async_open::start_or_apply(
        &controller,
        bytes,
        name,
        cad_app::UnsavedDecision::Cancel,
        false,
        false,
    );
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
