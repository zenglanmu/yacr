//! Annotation sidecar exports, imports and explicit recovery decisions.

use cad_app::host_files::{load_recovery, parse_recovery};
use cad_domain::Revision;

use super::persistence::WebPersistence;
use super::state_push;
use super::with_runtime;

/// Pure getter: confirm the exact revision only after a successful host write.
pub fn export_annotations_json() -> Result<(String, u64), String> {
    let controller =
        with_runtime(|rt| rt.controller.clone()).ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
    let (json, revision) = controller
        .borrow()
        .prepare_annotation_export()
        .map_err(|e| e.to_string())?;
    Ok((json, revision.0))
}

/// Confirm that the export at `revision` was durably written.
///
/// Confirming changes the document's dirty state, so the annotation panel is
/// re-pushed (the hidden/count labels must not lag).
pub fn confirm_annotation_export(revision: u64) -> Result<(), String> {
    let (controller, handle, view) =
        with_runtime(|rt| (rt.controller.clone(), rt.handle.clone(), rt.view.clone()))
            .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
    controller
        .borrow_mut()
        .confirm_annotation_export(Revision(revision))
        .map_err(|e| e.to_string())?;
    state_push::push_panel_state_for_view(&controller, &handle, &view);
    let _ = handle.set_status("批注已确认保存".to_string());
    Ok(())
}

/// Import annotations from JSON text chosen by the user.
pub fn import_annotations_json(text: &str) -> Result<usize, String> {
    let (controller, handle, view) =
        with_runtime(|rt| (rt.controller.clone(), rt.handle.clone(), rt.view.clone()))
            .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
    let count = {
        let mut c = controller.borrow_mut();
        c.import_annotations_json(text, cad_annotations::FingerprintPolicy::RejectMismatch)
            .map_err(|e| e.to_string())?
    };
    // One import is one transaction: the new rows and the enabled undo must
    // appear together.
    state_push::push_panel_state_for_view(&controller, &handle, &view);
    let _ = handle.set_status(format!("已导入 {count} 条批注"));
    Ok(count)
}

/// Restore through the strict transaction path; failed restores keep the copy.
pub fn restore_pending_recovery_snapshot() -> Result<usize, String> {
    let (controller, handle, view) =
        with_runtime(|rt| (rt.controller.clone(), rt.handle.clone(), rt.view.clone()))
            .ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
    let persistence = WebPersistence;
    let document = controller.borrow().document_id;
    let snapshot = cad_platform::block_on(load_recovery(&persistence, document))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "无恢复快照".to_string())?;
    let count = controller
        .borrow_mut()
        .restore_recovery_snapshot(&snapshot)
        .map_err(|e| e.to_string())?;
    cad_ui_slint::web::clear_recovery_snapshot();
    state_push::push_panel_state_for_view(&controller, &handle, &view);
    let _ = handle.set_status(format!("已从恢复快照恢复 {count} 条批注"));
    Ok(count)
}

/// Explicitly discard the pending recovery snapshot.
pub fn drop_pending_recovery_snapshot() {
    cad_ui_slint::web::clear_recovery_snapshot();
}

/// Inspect without clearing, including corrupt snapshots.
pub fn pending_recovery_snapshot() -> Option<String> {
    cad_ui_slint::web::peek_recovery_snapshot()
}

pub fn pending_recovery_is_valid() -> bool {
    cad_ui_slint::web::peek_recovery_snapshot()
        .map(|text| parse_recovery(text.as_bytes()).is_ok())
        .unwrap_or(false)
}
