//! Browser command sink and pointer navigation.
//!
//! Command routing, state push and the generic navigation/pick/draw sinks live
//! in `cad_host::HostCore`; this module keeps only what is genuinely
//! browser-specific:
//!
//! * the `OpenDrawing` / `SwitchBackend` interception (a hidden file input click
//!   and a backend preference store + reload), and
//! * the browser pointer semantics — the drag threshold, the button set and the
//!   wheel sign differ from the desktop winit backend, and a move feeds the
//!   active capture tool's live preview cursor.
//!
//! Every command still runs through the shared `HostCore::execute` funnel, so
//! panels, history and overlays are pushed exactly once.

use std::cell::Cell;

use cad_app::{Command, CommandId, CommandPayload, ToolState};
use cad_domain::{CadResult, Point3, ViewportId};
use cad_host::HostCore;
use cad_ui_slint::{LayoutSwitchSink, UiCommandSink, ViewInput};

use super::persistence::open_dialog;
use super::pick;

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
    !matches!(tool, ToolState::Measuring(_))
}

fn wheel_zoom_payload(dy: f64, cursor: [f64; 2]) -> CommandPayload {
    CommandPayload::ZoomAt {
        factor: (1.0 - dy * 0.0015).clamp(0.2, 5.0),
        cursor,
    }
}

/// Which capture tool a pointer move should feed its live preview cursor.
///
/// Pure classification so the pointer glue stays trivial: an inactive/idle tool
/// reports [`PreviewCursor::None`], meaning the move must not touch the session
/// or re-push the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PreviewCursor {
    Measurement,
    None,
}

/// The capture tool whose preview cursor a move belongs to, if any.
pub(super) fn preview_cursor_for(tool: &ToolState) -> PreviewCursor {
    match tool {
        ToolState::Measuring(_) => PreviewCursor::Measurement,
        _ => PreviewCursor::None,
    }
}

/// UI command sink; also the adapter's layout-switch sink is the shared core.
///
/// The browser intercepts the two commands whose implementation is genuinely
/// browser-specific and delegates every other command to `HostCore::execute`.
#[derive(Clone)]
pub(super) struct WebSink {
    pub(super) core: HostCore,
}

impl WebSink {
    fn set_status(&self, text: String) {
        if let Some(handle) = self.core.handle.borrow().as_ref() {
            let _ = handle.set_status(text);
        }
    }
}

impl UiCommandSink for WebSink {
    fn send(&mut self, command: Command) -> CadResult<()> {
        match command.id {
            CommandId::OpenDrawing => {
                open_dialog("file-input");
                self.set_status(super::messages::current_messages().text("file.choose", &[]));
                Ok(())
            }
            CommandId::SwitchBackend => {
                if let CommandPayload::Backend(choice) = command.payload {
                    let preference = super::preference_for(choice);
                    self.set_status(format!("切换后端为 {preference:?}，重建渲染会话…"));
                    if let Err(e) = cad_ui_slint::web::store_preference_and_reload(preference) {
                        self.set_status(format!("切换失败：{e}"));
                    }
                } else {
                    self.set_status("SwitchBackend 需要后端参数".into());
                }
                Ok(())
            }
            _ => super::run_command(&self.core, command),
        }
    }
}

/// Layout panel switch (F04): dispatch the validated `SwitchSpace` command
/// through the **web** funnel (`run_command`), so the browser overlay is
/// re-applied after the shared `HostCore::push` — a layout click must not
/// clobber the browser font summary / import panel or claim images are modeled.
impl LayoutSwitchSink for WebSink {
    fn select(&mut self, space: cad_representation::SpaceSelection) {
        let (document, viewport) = {
            let c = self.core.controller.borrow();
            (c.document_id, c.viewport_id)
        };
        let command = Command {
            schema_version: 1,
            id: CommandId::SwitchSpace,
            document,
            viewport,
            payload: cad_app::input::space_switch_payload(space),
        };
        let _ = super::run_command(&self.core, command);
    }
}

pub(super) struct WebViewInput {
    pub(super) core: HostCore,
    pub(super) viewport: ViewportId,
    pub(super) last: Cell<[f64; 2]>,
    pub(super) dragging: Cell<bool>,
    /// Where the current left/primary pointer went down, for tap-vs-drag.
    pub(super) down: Cell<Option<[f64; 2]>>,
}

impl WebViewInput {
    fn send(&self, id: CommandId, payload: CommandPayload) {
        let document = self.core.controller.borrow().document_id;
        let command = Command {
            schema_version: 1,
            id,
            document,
            viewport: self.viewport,
            payload,
        };
        let _ = super::run_command(&self.core, command);
    }

    /// Hit-test a tap and dispatch `Select` with the hit (or an empty payload on
    /// a miss). Runs through the shared funnel so the property panel is refreshed.
    fn select_at(&self, logical: [f64; 2]) {
        let outcome = {
            let controller = self.core.controller.borrow();
            match controller
                .application
                .workspace
                .viewports
                .get(&self.viewport)
            {
                None => return,
                Some(viewport) => match pick::canvas_size(&self.core.handle, viewport) {
                    None => return,
                    Some(size) => pick::pick_selection(&controller, size, logical),
                },
            }
        };
        match outcome {
            Ok(refs) => self.send(CommandId::Select, CommandPayload::Selection(refs)),
            Err(error) => {
                if let Some(handle) = self.core.handle.borrow().as_ref() {
                    let _ = handle.set_status(format!("选择失败：{error}"));
                }
            }
        }
    }

    /// Forward a pointer move as the active capture tool's live cursor.
    ///
    /// Cheap by construction: it maps the *same* logical point the renderer
    /// would ([`pick::map_canvas_point`]) to the session's preview cursor and
    /// re-runs the shared state-push funnel. It never captures a point, never
    /// writes the database and never rebuilds the base scene — only the overlay
    /// revision advances. A move with no capture tool is a no-op, so navigation
    /// stays free of preview work.
    fn preview_cursor_at(&self, logical: [f64; 2]) {
        let mapped = {
            let controller = self.core.controller.borrow();
            match preview_cursor_for(&controller.session.tool) {
                PreviewCursor::None => return,
                PreviewCursor::Measurement => {
                    match controller
                        .application
                        .workspace
                        .viewports
                        .get(&self.viewport)
                    {
                        None => return,
                        Some(viewport) => match pick::canvas_size(&self.core.handle, viewport) {
                            None => return,
                            Some(size) => pick::map_canvas_point(viewport, size, logical),
                        },
                    }
                }
            }
        };
        {
            let mut controller = self.core.controller.borrow_mut();
            let result = match preview_cursor_for(&controller.session.tool) {
                PreviewCursor::Measurement => controller.session.set_measurement_cursor(mapped),
                // The tool stopped between the two borrows: nothing to preview.
                PreviewCursor::None => return,
            };
            if result.is_err() {
                return;
            }
        }
        super::push_state(&self.core);
    }
}

impl ViewInput for WebViewInput {
    fn pointer(&self, kind: i32, button: i32, x: f64, y: f64) {
        match kind {
            0 => {
                if button == 1 || button == 0 || button == 3 {
                    self.dragging.set(true);
                    self.down.set(Some([x, y]));
                    self.last.set([x, y]);
                }
            }
            2 => {
                let last = self.last.get();
                if self.dragging.get() {
                    let wpp = self
                        .core
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
                } else {
                    // Not a navigation drag: feed the active capture tool's
                    // live preview cursor. Idle tools ignore this entirely.
                    self.preview_cursor_at([x, y]);
                }
                self.last.set([x, y]);
            }
            1 => {
                let down = self.down.take();
                self.dragging.set(false);
                self.last.set([x, y]);
                // A tap that did not become a pan hit-tests the model. The tap
                // test compares the press anchor to the release point, so a real
                // drag (past the shared threshold) is navigation and never
                // selects; merely holding the button is not a drag.
                let tap = is_selection_tap(down, [x, y]);
                let allowed = {
                    let controller = self.core.controller.borrow();
                    selection_allowed(&controller.session.tool)
                };
                if tap && allowed && (button == 1 || button == 0) {
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

    fn scroll(&self, _dx: f64, dy: f64, cursor: [f64; 2]) {
        self.last.set([0.0, 0.0]);
        self.send(CommandId::Zoom, wheel_zoom_payload(dy, cursor));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_app::{MeasurementTool, MeasurementToolKind};

    #[test]
    fn wheel_zoom_payload_keeps_cursor_and_browser_delta_sign() {
        let CommandPayload::ZoomAt { factor, cursor } = wheel_zoom_payload(200.0, [123.0, 177.0])
        else {
            panic!("wheel must emit a cursor-anchored zoom payload");
        };
        assert!((factor - 0.7).abs() < 1e-12);
        assert_eq!(cursor, [123.0, 177.0]);
    }

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
    fn a_press_released_in_place_is_a_tap_even_though_holding_sets_dragging() {
        // Regression (headless verification): the pointer-down sets the
        // "button held" flag, which is NOT a drag. Selection must key off the
        // press/release distance alone, otherwise every click is rejected as a
        // drag and nothing is ever selectable.
        let press = [640.0, 242.0];
        let release = [640.0, 242.0];
        assert!(is_selection_tap(Some(press), release));
        assert!(selection_allowed(&ToolState::Idle));
        // A held button that moved past the threshold is a pan, not a tap.
        assert!(!is_selection_tap(
            Some(press),
            [press[0] + cad_app::DRAG_THRESHOLD_LOGICAL_PX, press[1]]
        ));
    }

    #[test]
    fn selection_is_blocked_while_a_capture_tool_is_active() {
        assert!(selection_allowed(&ToolState::Idle));
        assert!(selection_allowed(&ToolState::Selecting));
        assert!(selection_allowed(&ToolState::Panning));
        assert!(!selection_allowed(&ToolState::Measuring(
            MeasurementTool::new(MeasurementToolKind::Distance)
        )));
    }

    #[test]
    fn only_capture_tools_receive_a_preview_cursor() {
        // Navigation/idle states must never trigger preview work on a move.
        assert_eq!(preview_cursor_for(&ToolState::Idle), PreviewCursor::None);
        assert_eq!(
            preview_cursor_for(&ToolState::Selecting),
            PreviewCursor::None
        );
        assert_eq!(preview_cursor_for(&ToolState::Panning), PreviewCursor::None);
        // Active capture tools are classified exactly.
        assert_eq!(
            preview_cursor_for(&ToolState::Measuring(MeasurementTool::new(
                MeasurementToolKind::Distance
            ))),
            PreviewCursor::Measurement
        );
    }
}
