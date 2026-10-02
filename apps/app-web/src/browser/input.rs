//! UI command dispatch and pointer navigation through the application layer.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cad_app::host::HostController;
use cad_app::host_files::{export_annotations_atomically, persist_recovery};
use cad_app::{Command, CommandId, CommandPayload, ToolState};
use cad_domain::{CadError, CadResult, Point3, ViewportId};
use cad_ui_slint::{CadView, UiCommandSink, ViewInput};

use super::persistence::{download_text, open_dialog, WebPersistence};
use super::{pick, state_push};
use super::{preference_for, viewport_camera, SharedHandle};

/// Whether a down→up sequence is a tap (a selection candidate) rather than a
/// navigation drag.
///
/// Uses the shared [`cad_app::DRAG_THRESHOLD_LOGICAL_PX`] so a host tap and a
/// Slint draw gesture agree; a missing down or a non-finite delta is never a
/// tap.
pub(super) fn is_selection_tap(down: Option<[f64; 2]>, up: [f64; 2]) -> bool {
    let Some(down) = down else {
        return false;
    };
    let dx = up[0] - down[0];
    let dy = up[1] - down[1];
    if !dx.is_finite() || !dy.is_finite() {
        return false;
    }
    (dx * dx + dy * dy).sqrt() < cad_app::DRAG_THRESHOLD_LOGICAL_PX
}

/// Selection is only a tap action while no capture tool is running.
pub(super) fn selection_allowed(tool: &ToolState) -> bool {
    !matches!(tool, ToolState::Measuring(_) | ToolState::Annotating(_))
}

pub(super) fn dispatch(
    controller: &Rc<RefCell<HostController>>,
    handle: &SharedHandle,
    view: &Rc<RefCell<Option<CadView>>>,
    viewport: &ViewportId,
    command: Command,
) {
    let status;
    {
        let mut c = controller.borrow_mut();
        match c.execute(command) {
            Ok(outcome) => {
                status = outcome
                    .diagnostics
                    .first()
                    .map(|d| d.message.clone())
                    .unwrap_or_else(|| c.status().to_string());
            }
            Err(CadError::NotImplemented(feature)) => {
                status = format!("未实现：{feature}");
            }
            Err(CadError::Unsupported(reason)) => {
                status = format!("仅宿主执行：{reason}");
            }
            Err(e) => {
                status = format!("命令失败：{e}");
            }
        }
        if let (Some(view), Some(vp)) = (
            view.borrow().as_ref(),
            c.application.workspace.viewports.get(viewport),
        ) {
            view.sync_session(&c.session.active_space, vp);
        }
    }
    if let Some(handle) = handle.borrow().as_ref() {
        // One funnel for every command: history, measurement, layers, layout,
        // properties, annotations and diagnostics are pushed together so no
        // panel can lag the command that changed it (audit U03/U04/U11).
        state_push::push_panel_state(controller, handle, view);
        let _ = handle.set_status(status);
    }
}

pub(super) struct WebSink {
    pub(super) controller: Rc<RefCell<HostController>>,
    pub(super) handle: SharedHandle,
    pub(super) view: Rc<RefCell<Option<CadView>>>,
    pub(super) viewport: ViewportId,
}

impl UiCommandSink for WebSink {
    fn send(&mut self, command: Command) -> CadResult<()> {
        let set_status = |text: String| {
            if let Some(handle) = self.handle.borrow().as_ref() {
                let _ = handle.set_status(text);
            }
        };
        match command.id {
            CommandId::OpenDrawing => {
                open_dialog("file-input");
                set_status("请选择 DWG 文件…".into());
                return Ok(());
            }
            CommandId::ExportAnnotations => {
                // Prepare → host write → confirm the exact revision (B07).
                let result =
                    export_annotations_atomically(&mut self.controller.borrow_mut(), |json| {
                        download_text("annotations.cadnotes.json", json)
                    });
                // Confirming the export clears dirty: re-push so the annotation
                // panel and history availability follow.
                if let Some(handle) = self.handle.borrow().as_ref() {
                    state_push::push_panel_state(&self.controller, handle, &self.view);
                }
                match result {
                    Ok(()) => set_status("批注已确认保存到下载文件".into()),
                    Err(e) => set_status(format!("导出失败：{e}（批注仍为未保存）")),
                }
                return Ok(());
            }
            CommandId::ImportAnnotations => {
                open_dialog("annotation-input");
                set_status("请选择批注 JSON 文件…".into());
                return Ok(());
            }
            CommandId::SwitchBackend => {
                if let CommandPayload::Backend(choice) = command.payload {
                    let preference = preference_for(choice);
                    // Persist unsaved annotations before reloading (B06).
                    {
                        let c = self.controller.borrow();
                        let signal = c.unsaved_signal();
                        if signal.dirty {
                            let (center, wpp) = viewport_camera(&c);
                            let snapshot = match c.capture_recovery_snapshot(center, wpp) {
                                Ok(snapshot) => snapshot,
                                Err(e) => {
                                    set_status(format!(
                                        "切换失败：无法生成恢复快照（{e}），未重载"
                                    ));
                                    return Ok(());
                                }
                            };
                            if let Err(e) = cad_platform::block_on(persist_recovery(
                                &WebPersistence,
                                c.document_id,
                                &snapshot,
                            )) {
                                set_status(format!("切换失败：恢复快照未能持久化（{e}），未重载"));
                                return Ok(());
                            }
                        }
                    }
                    set_status(format!("切换后端为 {preference:?}，重建渲染会话…"));
                    if let Err(e) = cad_ui_slint::web::store_preference_and_reload(preference) {
                        set_status(format!("切换失败：{e}（未保存批注未丢失）"));
                    }
                } else {
                    set_status("SwitchBackend 需要后端参数".into());
                }
                return Ok(());
            }
            _ => {}
        }
        dispatch(
            &self.controller,
            &self.handle,
            &self.view,
            &self.viewport,
            command,
        );
        Ok(())
    }
}

pub(super) struct WebViewInput {
    pub(super) controller: Rc<RefCell<HostController>>,
    pub(super) handle: SharedHandle,
    pub(super) view: Rc<RefCell<Option<CadView>>>,
    pub(super) viewport: ViewportId,
    pub(super) last: Cell<[f64; 2]>,
    pub(super) dragging: Cell<bool>,
    /// Where the current left/primary pointer went down, for tap-vs-drag.
    pub(super) down: Cell<Option<[f64; 2]>>,
}

impl WebViewInput {
    fn send(&self, id: CommandId, payload: CommandPayload) {
        let document = self.controller.borrow().document_id;
        let command = Command {
            schema_version: 1,
            id,
            document,
            viewport: self.viewport,
            payload,
        };
        dispatch(
            &self.controller,
            &self.handle,
            &self.view,
            &self.viewport,
            command,
        );
    }

    /// Hit-test a tap and dispatch `Select` with the hit (or an empty payload on
    /// a miss). Runs through [`dispatch`] so the property panel is refreshed.
    fn select_at(&self, logical: [f64; 2]) {
        let outcome = {
            let controller = self.controller.borrow();
            match controller
                .application
                .workspace
                .viewports
                .get(&self.viewport)
            {
                None => return,
                Some(viewport) => match pick::canvas_size(&self.handle, viewport) {
                    None => return,
                    Some(size) => pick::pick_selection(&controller, size, logical),
                },
            }
        };
        match outcome {
            Ok(refs) => self.send(CommandId::Select, CommandPayload::Selection(refs)),
            Err(error) => {
                if let Some(handle) = self.handle.borrow().as_ref() {
                    let _ = handle.set_status(format!("选择失败：{error}"));
                }
            }
        }
    }
}

impl ViewInput for WebViewInput {
    fn pointer(&self, kind: i32, button: i32, x: f64, y: f64) {
        match kind {
            0 => {
                if button == 1 || button == 0 {
                    self.dragging.set(true);
                    self.down.set(Some([x, y]));
                    self.last.set([x, y]);
                }
            }
            2 => {
                let last = self.last.get();
                if self.dragging.get() {
                    let wpp = self
                        .controller
                        .borrow()
                        .application
                        .workspace
                        .viewports
                        .get(&self.viewport)
                        .map(|v| v.world_per_px())
                        .unwrap_or(1.0);
                    // Command delta is in world units; screen y is inverted.
                    let delta = Point3 {
                        x: (x - last[0]) * wpp,
                        y: -(y - last[1]) * wpp,
                        z: 0.0,
                    };
                    self.send(CommandId::Pan, CommandPayload::Points(vec![delta]));
                }
                self.last.set([x, y]);
            }
            1 => {
                let down = self.down.take();
                let was_dragging = self.dragging.replace(false);
                self.last.set([x, y]);
                // A tap that did not become a pan or a capture-tool pick hit-
                // tests the model. A drag (past the shared threshold) is
                // navigation and must never select.
                let tap = is_selection_tap(down, [x, y]);
                let allowed = {
                    let controller = self.controller.borrow();
                    selection_allowed(&controller.session.tool)
                };
                if tap && allowed && (button == 1 || button == 0) && !was_dragging {
                    self.select_at([x, y]);
                }
            }
            3 => {
                // Pointer cancelled by the OS/browser: never a tap.
                self.down.set(None);
                self.dragging.set(false);
                self.last.set([x, y]);
            }
            _ => {}
        }
    }

    fn scroll(&self, _dx: f64, dy: f64) {
        let factor = (1.0 - dy * 0.0015).clamp(0.2, 5.0);
        self.last.set([0.0, 0.0]);
        self.send(
            CommandId::Zoom,
            CommandPayload::Points(vec![Point3 {
                x: factor,
                y: 0.0,
                z: 0.0,
            }]),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_app::{MeasurementTool, MeasurementToolKind};

    #[test]
    fn a_zero_length_press_and_release_is_a_selection_tap() {
        assert!(is_selection_tap(Some([10.0, 20.0]), [10.0, 20.0]));
    }

    #[test]
    fn a_move_at_the_drag_threshold_is_not_a_tap() {
        let threshold = cad_app::DRAG_THRESHOLD_LOGICAL_PX;
        assert!(!is_selection_tap(Some([0.0, 0.0]), [threshold, 0.0]));
        assert!(!is_selection_tap(Some([0.0, 0.0]), [0.0, threshold + 5.0]));
        // Just under the threshold is still a tap.
        assert!(is_selection_tap(Some([0.0, 0.0]), [threshold - 0.5, 0.0]));
    }

    #[test]
    fn a_missing_or_non_finite_press_is_not_a_tap() {
        assert!(!is_selection_tap(None, [0.0, 0.0]));
        assert!(!is_selection_tap(Some([0.0, 0.0]), [f64::NAN, 0.0]));
    }

    #[test]
    fn selection_is_blocked_while_a_capture_tool_is_active() {
        assert!(selection_allowed(&ToolState::Idle));
        assert!(selection_allowed(&ToolState::Selecting));
        assert!(selection_allowed(&ToolState::Panning));
        assert!(!selection_allowed(&ToolState::Measuring(
            MeasurementTool::new(MeasurementToolKind::Distance)
        )));
        assert!(!selection_allowed(&ToolState::Annotating(
            cad_app::AnnotationTool::new(cad_app::AnnotationToolKind::Text)
        )));
    }
}
