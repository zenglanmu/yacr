//! Drawing replacement through the shared asynchronous import path.
//!
//! The bytes arrive from the JS File API host and are queued as a
//! [`cad_platform::FileEvent`]; this module drains that queue and installs the
//! drawing. The browser has no worker thread, so the import runs synchronously
//! (see [`super::async_open`]).

use cad_app::host::OpenedDrawing;
use cad_host::HostCore;
use cad_platform::{FileEvent, PlatformFiles};
use cad_ui_slint::IncomingDocument;

use super::fonts::spawn_font_load;
use super::platform::WebFiles;
use super::{sync_view_camera, with_runtime};

/// Install a successfully opened drawing into the shared view/incoming slot.
///
/// Shared by the synchronous fallback and the asynchronous poll, so the
/// document swap, camera sync, panel push and font load happen in exactly one
/// place (no duplicated open path).
pub(super) fn install_opened(
    name: &str,
    opened: &OpenedDrawing,
    core: &HostCore,
    incoming: &IncomingDocument,
) {
    let viewport = core.controller.borrow().viewport_id;
    let drawing = {
        let mut c = core.controller.borrow_mut();
        let _ = c.fit();
        c.drawing()
    };
    if let Some(handle) = core.handle.borrow().as_ref() {
        let _ = handle.cancel_draw_capture();
    }
    *incoming.borrow_mut() = drawing;
    {
        let controller = core.controller.borrow();
        if let Some(view) = core.view.borrow().as_ref() {
            sync_view_camera(&controller, view, &viewport);
        }
    }
    if let Some(view) = core.view.borrow().as_ref() {
        view.request_redraw();
    }
    if let Some(handle) = core.handle.borrow().as_ref() {
        let _ = handle.set_status(format!("已打开 {name}: {}", opened.completeness_label));
    }
    // Panels (layout/properties/diagnostics) are derived from the new document;
    // push them after the content swap so none shows stale data.
    super::push_state(core);
    spawn_font_load();
}

/// Open a drawing from the File API / File System Access path.
pub fn open_document(name: &str, bytes: Vec<u8>) -> Result<(), String> {
    let (core, files, incoming) =
        with_runtime(|rt| (rt.core.clone(), rt.files.clone(), rt.incoming.clone()))
            .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
    // Queue the bytes as a platform file event and drain it through the shared
    // open funnel: the `PlatformFiles` contract is event-based, and the browser
    // read is synchronous so the event is available immediately.
    files.push_opened(name.to_string(), bytes);
    drain_open_events(&core, &files, &incoming)
}

/// Drain the web file queue, installing every opened drawing.
///
/// Only `Opened` events are handled here; any other event (a save target, for
/// example) is put back for the core's own save funnel, which drains the same
/// queue in its gesture.
pub(super) fn drain_open_events(
    core: &HostCore,
    files: &WebFiles,
    incoming: &IncomingDocument,
) -> Result<(), String> {
    let mut result = Ok(());
    for event in files.poll() {
        match event {
            FileEvent::Opened { target, bytes } => {
                match super::async_open::start_or_apply(
                    &core.controller,
                    bytes,
                    &target.display_name,
                ) {
                    Ok(super::async_open::OpenStart::Opened(opened)) => {
                        install_opened(&target.display_name, &opened, core, incoming);
                    }
                    Ok(super::async_open::OpenStart::Started) => {
                        if let Some(handle) = core.handle.borrow().as_ref() {
                            let _ =
                                handle.set_status(format!("正在后台打开 {}…", target.display_name));
                        }
                        super::state_push::push_import_state(core);
                    }
                    Err(error) => {
                        if let Some(handle) = core.handle.borrow().as_ref() {
                            let _ = handle
                                .set_status(format!("打开 {} 失败：{error}", target.display_name));
                        }
                        result = Err(error.to_string());
                    }
                }
            }
            other => files.requeue(other),
        }
    }
    result
}
