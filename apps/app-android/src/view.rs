//! Android canvas pointer input.
//!
//! Command routing, state push and the generic sinks live in
//! `cad_host::HostCore`; this module keeps only what is genuinely Android
//! specific: the shared [`InputPolicy`] tap/drag state machine (a one-finger
//! drag pans, a real tap selects) and the canvas-local selection pick with its
//! explicit hit/miss status.
//!
//! Every command still runs through the shared `HostCore::execute` funnel, so
//! panels, history and overlays are pushed exactly once.

use super::*;
use cad_app::input::{InputOutcome, InputPolicy, PointerPhase, PointerUpdate};
use cad_app::{Command, CommandId, CommandPayload};
use cad_host::HostCore;
use cad_ui_slint::ViewInput;
use std::cell::Cell;

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

/// Canvas navigation for Android: one-finger drag pans, wheel/pinch zooms.
///
/// The shell's `TouchArea` emits logical-pixel pointer events. This host turns
/// them into the same `Pan`/`Zoom` commands the web host uses, so the
/// authoritative viewport stays the single camera truth. Without this the
/// `pointer-input`/`scroll-input` callbacks were dropped and the Android canvas
/// could not be panned or zoomed at all (verified: a swipe changed 0 pixels).
pub(crate) struct AndroidViewInput {
    core: HostCore,
    viewport: ViewportId,
    last: Cell<[f64; 2]>,
    dragging: Cell<bool>,
    /// Shared pointer/gesture state machine (audit U05). Used to tell a tap from
    /// a drag so a one-finger drag pans and only a real tap turns into a
    /// selection pick.
    policy: RefCell<InputPolicy>,
    touch_guard: RefCell<cad_ui_slint::CanvasTouchGuard>,
    pinch_scale: Cell<Option<f64>>,
}

impl AndroidViewInput {
    pub(crate) fn new(core: HostCore, viewport: ViewportId) -> Self {
        Self {
            core,
            viewport,
            last: Cell::new([0.0, 0.0]),
            dragging: Cell::new(false),
            policy: RefCell::new(InputPolicy::new()),
            touch_guard: RefCell::new(cad_ui_slint::CanvasTouchGuard::default()),
            pinch_scale: Cell::new(None),
        }
    }

    fn status(&self, text: String) {
        if let Some(handle) = self.core.handle.borrow().as_ref() {
            let _ = handle.set_status(text);
        }
    }

    fn send(&self, id: CommandId, payload: CommandPayload) -> bool {
        let zoom_request = match (&id, &payload) {
            (CommandId::Zoom, CommandPayload::ZoomAt { factor, cursor }) => self
                .core
                .controller
                .borrow()
                .application
                .workspace
                .viewports
                .get(&self.viewport)
                .map(|viewport| (viewport.camera, viewport.logical_size, *factor, *cursor)),
            _ => None,
        };
        let document = self.core.controller.borrow().document_id;
        let command = Command {
            schema_version: 1,
            id,
            document,
            viewport: self.viewport,
            payload,
        };
        let result = state_push::run_command(&self.core, command);
        if result.is_ok() {
            return true;
        }
        // HostCore returns a post-command UI push error when the command itself
        // already succeeded. For zoom, camera mutation is authoritative evidence
        // that the command applied and the cumulative gesture baseline may move.
        zoom_request.is_some_and(|(before, size, factor, cursor)| {
            let changed = self
                .core
                .controller
                .borrow()
                .application
                .workspace
                .viewports
                .get(&self.viewport)
                .is_some_and(|viewport| viewport.camera != before);
            if changed {
                return true;
            }
            // A valid zoom can clamp at a limit and leave the camera unchanged.
            // Validate the same transformation on a copy to distinguish that
            // accepted no-op from a rejected command despite a later push error.
            let mut probe = before;
            probe.zoom_at(factor, cursor, size).is_ok() && probe == before
        })
    }

    /// Whether a capture tool (measure/annotate) owns canvas taps right now.
    ///
    /// With a capture tool active the Slint `canvas-pick` path (through the
    /// shared `CanvasPickMapper`) feeds the tool; the host must not also run a
    /// selection pick for the same tap.
    fn capture_tool_active(&self) -> bool {
        matches!(
            self.core.controller.borrow().session.tool,
            cad_app::ToolState::Measuring(_)
        )
    }

    /// A real tap with no capture tool: pick the closest entity and select it.
    ///
    /// A miss clears the selection and says so explicitly; it never keeps or
    /// invents a hit (F05). The world point comes from the authoritative
    /// viewport via `pick_at_screen`, using the same canvas size the renderer
    /// uses.
    fn pick_selection(&self, logical: [f64; 2]) {
        let messages = self.core.messages();
        let (camera, logical_size, document) = {
            let controller = self.core.controller.borrow();
            let viewport = match controller
                .application
                .workspace
                .viewports
                .get(&self.viewport)
            {
                Some(viewport) => viewport,
                None => {
                    self.status(messages.text("android.pick_viewport_missing", &[]));
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
        let drawing = self.core.controller.borrow().drawing();
        let Some(drawing) = drawing else {
            self.status(messages.text("android.pick_no_document", &[]));
            return;
        };
        let report = cad_app::picking::pick_at_screen(
            drawing.as_ref(),
            document,
            &camera,
            logical,
            logical_size,
            &cad_domain::TolerancePolicy::default(),
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
                    self.status(messages.text("android.selected", &[("identity", &identity)]));
                }
                None => {
                    // Clear the selection explicitly on empty space; never a
                    // stale or fabricated hit.
                    self.send(CommandId::Select, CommandPayload::Selection(Vec::new()));
                    self.status(messages.text("android.pick_miss", &[]));
                }
            },
            Err(error) => {
                self.status(messages.text("android.pick_failed", &[("error", &error.to_string())]))
            }
        }
    }
}

pub(super) fn pinch_factor(previous: f64, current: f64) -> Option<f64> {
    if !valid_scale(previous) || !valid_scale(current) {
        return None;
    }
    let factor = current / previous;
    (factor.is_finite() && factor > 0.0 && factor != 1.0).then_some(factor)
}

fn valid_scale(scale: f64) -> bool {
    scale.is_finite() && scale > 0.0
}

pub(super) fn update_pinch_scale(
    previous: Option<f64>,
    current: f64,
    mut submit: impl FnMut(f64) -> bool,
) -> Option<f64> {
    let previous = previous.filter(|scale| valid_scale(*scale))?;
    if !valid_scale(current) {
        return Some(previous);
    }
    let Some(factor) = pinch_factor(previous, current) else {
        return Some(previous);
    };
    Some(if submit(factor) { current } else { previous })
}

impl ViewInput for AndroidViewInput {
    fn pointer_exited(&self) {
        self.touch_guard.borrow_mut().exit();
        self.dragging.set(false);
        self.policy.borrow_mut().pointer_cancelled();
    }

    fn pointer(&self, kind: i32, button: i32, x: f64, y: f64) {
        self.pointer_with_id(kind, button, x, y, 0);
    }

    fn pointer_with_id(&self, kind: i32, button: i32, x: f64, y: f64, contact_id: i32) {
        self.touch_guard.borrow_mut().pointer(kind, contact_id);
        let logical = [x, y];
        match kind {
            // down
            0 => {
                if button == 1 || button == 0 {
                    if !self.touch_guard.borrow().allows_pick() {
                        self.dragging.set(false);
                        self.policy.borrow_mut().pointer_cancelled();
                        return;
                    }
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
                        .core
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
                    if was_tap
                        && self.touch_guard.borrow().allows_pick()
                        && !self.capture_tool_active()
                    {
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

    fn pinch(&self, phase: i32, scale: f64, x: f64, y: f64) {
        match phase {
            0 => {
                self.touch_guard.borrow_mut().gesture(phase);
                self.pinch_scale.set(valid_scale(scale).then_some(scale));
                self.dragging.set(false);
                self.policy.borrow_mut().pointer_cancelled();
            }
            1 => {
                self.touch_guard.borrow_mut().gesture(phase);
                let previous = self.pinch_scale.get();
                let next = update_pinch_scale(previous, scale, |factor| {
                    self.send(
                        CommandId::Zoom,
                        CommandPayload::ZoomAt {
                            factor,
                            cursor: [x, y],
                        },
                    )
                });
                self.pinch_scale.set(next);
            }
            2 | 3 => {
                self.touch_guard.borrow_mut().gesture(phase);
                self.pinch_scale.set(None);
            }
            _ => {}
        }
    }

    fn scroll(&self, _dx: f64, dy: f64, cursor: [f64; 2]) {
        // Same formula as the web and Linux hosts: scrolling down (positive dy)
        // yields factor < 1 = zoom out; scrolling up zooms in (`Camera::zoom_at`
        // contract: factor > 1 zooms in).
        let factor = (1.0 - dy * 0.0015).clamp(0.2, 5.0);
        self.last.set([0.0, 0.0]);
        self.send(CommandId::Zoom, CommandPayload::ZoomAt { factor, cursor });
    }
}
