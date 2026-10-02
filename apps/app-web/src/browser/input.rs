//! UI command dispatch and pointer navigation through the application layer.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cad_app::host::HostController;
use cad_app::host_files::{export_annotations_atomically, persist_recovery};
use cad_app::{Command, CommandId, CommandPayload};
use cad_domain::{CadError, CadResult, Point3, ViewportId};
use cad_ui_slint::{CadView, UiCommandSink, ViewInput};

use super::persistence::{download_text, open_dialog, WebPersistence};
use super::{preference_for, viewport_camera, SharedHandle};

fn dispatch(
    controller: &Rc<RefCell<HostController>>,
    handle: &SharedHandle,
    view: &Rc<RefCell<Option<CadView>>>,
    viewport: &ViewportId,
    command: Command,
) {
    let can_undo;
    let status;
    {
        let mut c = controller.borrow_mut();
        match c.execute(command) {
            Ok(outcome) => {
                can_undo = c.application.can_undo(&c.document_id);
                status = outcome
                    .diagnostics
                    .first()
                    .map(|d| d.message.clone())
                    .unwrap_or_else(|| c.status().to_string());
            }
            Err(CadError::NotImplemented(feature)) => {
                can_undo = c.application.can_undo(&c.document_id);
                status = format!("未实现：{feature}");
            }
            Err(CadError::Unsupported(reason)) => {
                can_undo = c.application.can_undo(&c.document_id);
                status = format!("仅宿主执行：{reason}");
            }
            Err(e) => {
                can_undo = c.application.can_undo(&c.document_id);
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
        let _ = handle.set_status(status);
        let _ = handle.set_can_undo(can_undo);
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
}

impl ViewInput for WebViewInput {
    fn pointer(&self, kind: i32, button: i32, x: f64, y: f64) {
        match kind {
            0 => {
                if button == 1 || button == 0 {
                    self.dragging.set(true);
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
            1 | 3 => {
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
