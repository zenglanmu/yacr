//! view module.

use super::*;

impl Default for AndroidHostConfiguration {
    fn default() -> Self {
        AndroidHostConfiguration {
            sample_paths: vec![
                "/sdcard/Download/yacr-sample.dwg".to_string(),
                "/storage/emulated/0/Download/yacr-sample.dwg".to_string(),
            ],
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
            // One snapshot refreshes undo/redo and every panel (audit U11).
            push_panel_state(&self.controller, handle, &self.view);
        }
        match outcome {
            Ok(_) => {}
            Err(CadError::NotImplemented(feature)) => self.status(format!("未实现：{feature}")),
            Err(e) => self.status(format!("命令失败：{e}")),
        }
    }

    /// Whether a capture tool (measure/annotate) owns canvas taps right now.
    ///
    /// With a capture tool active the Slint `canvas-pick` path (through the
    /// installed `AndroidCanvasPickMapper`) feeds the tool; the host must not
    /// also run a selection pick for the same tap.
    fn capture_tool_active(&self) -> bool {
        let controller = self.controller.borrow();
        matches!(controller.session.tool, cad_app::ToolState::Measuring(_))
    }

    /// A real tap with no capture tool: pick the closest entity and select it.
    ///
    /// A miss clears the selection and says so explicitly; it never keeps or
    /// invents a hit (F05). The world point comes from the authoritative
    /// viewport via `pick_at_screen`, using the same canvas size the renderer
    /// uses.
    fn pick_selection(&self, logical: [f64; 2]) {
        let (camera, logical_size, document) = {
            let controller = self.controller.borrow();
            let viewport = match controller
                .application
                .workspace
                .viewports
                .get(&self.viewport)
            {
                Some(viewport) => viewport,
                None => {
                    self.status("选择失败：视口不存在");
                    return;
                }
            };
            (
                viewport.camera,
                viewport.logical_size,
                controller.document_id,
            )
        };
        // Bind the borrow to its own statement: a `match` scrutinee temporary
        // would stay alive across `self.send` below and panic on the mutable
        // re-borrow inside it.
        let drawing = self.controller.borrow().drawing();
        let Some(drawing) = drawing else {
            self.status("选择失败：未打开文档");
            return;
        };
        let report = cad_app::picking::pick_at_screen(
            drawing.as_ref(),
            document,
            &camera,
            logical,
            logical_size,
            &TolerancePolicy::default(),
            cad_app::BackFacePolicy::Cull,
        );
        match report {
            Ok(report) => match report.hit {
                Some(hit) => {
                    let identity = cad_app::SelectionSet::identity_label(&hit.source);
                    self.send(
                        CommandId::Select,
                        CommandPayload::Selection(vec![hit.source]),
                    );
                    self.status(format!("已选择 {identity}"));
                }
                None => {
                    // Clear the selection explicitly on empty space; never a
                    // stale or fabricated hit.
                    self.send(CommandId::Select, CommandPayload::Selection(Vec::new()));
                    self.status("未命中任何对象（已清除选择）");
                }
            },
            Err(e) => self.status(format!("选择拾取失败：{e}")),
        }
    }
}

impl ViewInput for AndroidViewInput {
    fn pointer(&self, kind: i32, button: i32, x: f64, y: f64) {
        let logical = [x, y];
        match kind {
            // down
            0 => {
                if button == 1 || button == 0 {
                    self.dragging.set(true);
                    self.last.set(logical);
                    self.policy.borrow_mut().handle(PointerUpdate {
                        logical_position: logical,
                        contacts: 1,
                    });
                }
            }
            // move
            2 => {
                self.policy.borrow_mut().handle(PointerUpdate {
                    logical_position: logical,
                    contacts: 1,
                });
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
                self.last.set(logical);
            }
            // up / cancel
            1 | 3 => {
                self.dragging.set(false);
                if kind == 3 {
                    // OS stole the gesture: exit the policy to idle, cancel any
                    // in-progress tool, never a pick.
                    self.policy.borrow_mut().pointer_cancelled();
                } else {
                    // Read the phase before the policy consumes the up: a
                    // `Pending` contact released in place is a tap; a
                    // `Dragging` one is a pan and must not select.
                    let was_tap =
                        matches!(self.policy.borrow().phase(), PointerPhase::Pending { .. });
                    let outcome = self.policy.borrow_mut().handle(PointerUpdate {
                        logical_position: logical,
                        contacts: 0,
                    });
                    if was_tap && !self.capture_tool_active() {
                        if let InputOutcome::Commit { logical: tapped } = outcome {
                            self.pick_selection(tapped);
                        }
                    }
                }
                self.last.set(logical);
            }
            _ => {}
        }
    }

    fn scroll(&self, _dx: f64, dy: f64) {
        // Same formula as the web and Linux hosts: scrolling down (positive dy)
        // yields factor < 1 = zoom out; scrolling up zooms in (`Camera::zoom_at`
        // contract: factor > 1 zooms in).
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
