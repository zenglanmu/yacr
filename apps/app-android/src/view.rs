//! view module.

use super::*;

impl Default for AndroidHostConfiguration {
    fn default() -> Self {
        AndroidHostConfiguration {
            recovery_enabled: true,
            sample_paths: vec![
                "/sdcard/Download/yacr-sample.dwg".to_string(),
                "/storage/emulated/0/Download/yacr-sample.dwg".to_string(),
            ],
            recovery_directory: None,
            export_directory: None,
            unsaved_decision: None,
        }
    }
}

/// Mirror the authoritative viewport camera into the composited CAD frame.
///
/// The camera lives in the application `Viewport`; the bridge's `CadView` only
/// reflects it for rendering (spec §4.2, single camera truth).
pub(crate) fn sync_view_camera(
    view: &SharedView,
    controller: &Rc<RefCell<HostController>>,
    viewport: ViewportId,
) {
    let controller = controller.borrow();
    if let (Some(view), Some(viewport)) = (
        view.borrow().as_ref(),
        controller.application.workspace.viewports.get(&viewport),
    ) {
        // One call keeps the active space, the observation mode and the full
        // camera in sync (F04/F13); a bare `set_camera` would leave the bridge
        // stuck in model-space 2D (workstream C's host glue).
        view.sync_session(&controller.session.active_space, viewport);
    }
}

impl AndroidViewInput {
    fn status(&self, text: impl Into<String>) {
        if let Some(handle) = self.handle.borrow().as_ref() {
            let _ = handle.set_status(text.into());
        }
    }

    fn send(&self, id: CommandId, payload: CommandPayload) {
        let document = self.controller.borrow().document_id;
        let command = Command {
            schema_version: 1,
            id,
            document,
            viewport: self.viewport,
            payload,
        };
        let outcome = self.controller.borrow_mut().execute(command);
        sync_view_camera(&self.view, &self.controller, self.viewport);
        if let Some(handle) = self.handle.borrow().as_ref() {
            let can_undo = {
                let controller = self.controller.borrow();
                controller.application.can_undo(&controller.document_id)
            };
            let _ = handle.set_can_undo(can_undo);
        }
        match outcome {
            Ok(_) => {}
            Err(CadError::NotImplemented(feature)) => self.status(format!("未实现：{feature}")),
            Err(e) => self.status(format!("命令失败：{e}")),
        }
    }
}

impl ViewInput for AndroidViewInput {
    fn pointer(&self, kind: i32, button: i32, x: f64, y: f64) {
        match kind {
            // down
            0 => {
                if button == 1 || button == 0 {
                    self.dragging.set(true);
                    self.last.set([x, y]);
                }
            }
            // move
            2 => {
                if self.dragging.get() {
                    let last = self.last.get();
                    let world_per_px = self
                        .controller
                        .borrow()
                        .application
                        .workspace
                        .viewports
                        .get(&self.viewport)
                        .map(|v| v.world_per_px())
                        .unwrap_or(1.0);
                    // Command delta is world units; screen y points down and
                    // world y points up, so its sign flips.
                    let delta = Point3 {
                        x: (x - last[0]) * world_per_px,
                        y: -(y - last[1]) * world_per_px,
                        z: 0.0,
                    };
                    self.send(CommandId::Pan, CommandPayload::Points(vec![delta]));
                }
                self.last.set([x, y]);
            }
            // up / cancel
            1 | 3 => {
                self.dragging.set(false);
                self.last.set([x, y]);
            }
            _ => {}
        }
    }

    fn scroll(&self, _dx: f64, dy: f64) {
        // Scroll down (positive dy) zooms in, matching the web host.
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
