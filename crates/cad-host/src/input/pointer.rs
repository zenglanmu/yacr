//! Pointer gesture handling (press/drag/orbit/pan/box-select) and 3D navigation.
use super::*;
use cad_app::input::DRAG_THRESHOLD_LOGICAL_PX;
use cad_app::interaction::PointerPhase;

/// Dash length of a CROSSING box-selection edge, in logical pixels (ADR 0014 B.9).
const BOX_DASH_LOGICAL_PX: f64 = 5.0;
/// Gap between two dashes of a CROSSING edge, in logical pixels.
const BOX_GAP_LOGICAL_PX: f64 = 5.0;
/// Upper bound on the number of world-space segments one crossed rubber band
/// emits.
///
/// The CROSSING dash/gap step is derived from the rectangle's logical-pixel
/// perimeter so the count stays within this bound **without** truncating the
/// geometry (ADR 0014 B.9): a fixed dash pattern used to drop whole trailing
/// edges on a large window and draw an *open* rectangle. For a perimeter within
/// the bound the nominal 5 px dash / 5 px gap is preserved.
const BOX_MAX_SEGMENTS: usize = 512;

impl HostCore {
    pub(super) fn pointer(
        &self,
        id: PointerId,
        phase: PointerPhase,
        button: Option<PointerButton>,
        position: PointInput,
        modifiers: Modifiers,
    ) -> CadResult<()> {
        let point = self.pointer_logical(position)?;
        // The renderer's cursor crosshair follows the pointer: remember its
        // logical position on every real phase. A cancel carries no coordinates
        // (the typed request zero-fills them), so it never sets a position.
        if !matches!(phase, PointerPhase::Cancel) {
            self.cursor_logical.set(Some(point));
        }
        match phase {
            PointerPhase::Down => self.pointer_down(id, button, point, modifiers),
            PointerPhase::Move => self.pointer_move(id, button, point),
            PointerPhase::Up => self.pointer_up(id, point, modifiers),
            PointerPhase::Cancel => self.pointer_cancel(id),
        }
    }

    /// Resolve a pointer position to canvas-local logical coordinates.
    ///
    /// A UI delivers canvas-local logical positions tagged with the metrics
    /// revision it observed; a world point is accepted and projected only for a
    /// host that supplies one directly.
    fn pointer_logical(&self, position: PointInput) -> CadResult<[f64; 2]> {
        match position {
            PointInput::CanvasLocal {
                logical_position,
                metrics_revision,
            } => {
                if metrics_revision != self.metrics_revision.get() {
                    return Err(CadError::InvalidInput(
                        "stale canvas metrics revision".into(),
                    ));
                }
                if !logical_position.iter().copied().all(f64::is_finite) {
                    return Err(CadError::InvalidInput(
                        "pointer position must be finite".into(),
                    ));
                }
                Ok(logical_position)
            }
            PointInput::World(world) => {
                let c = self.controller.borrow();
                let viewport = c
                    .application
                    .workspace
                    .viewports
                    .get(&c.viewport_id)
                    .ok_or(CadError::Cancelled)?;
                viewport
                    .camera
                    .world_to_screen(world, viewport.logical_size)
                    .ok_or_else(|| CadError::InvalidInput("world point cannot be projected".into()))
            }
        }
    }

    fn pointer_down(
        &self,
        id: PointerId,
        button: Option<PointerButton>,
        point: [f64; 2],
        modifiers: Modifiers,
    ) -> CadResult<()> {
        // A press starts a gesture (pan/orbit/drag or a tap): hover must not
        // survive into it, and the tint is cleared before anything else.
        self.clear_hover();
        let mut active = self.navigation.active.borrow_mut();
        if active.contains(&id) {
            return Ok(());
        }
        active.push(id);
        let contacts = active.len();
        drop(active);
        if contacts == 1 {
            self.navigation.anchor.set(Some(point));
            self.navigation.last.set(Some(point));
            self.navigation.dragged.set(false);
            self.navigation.navigating.set(false);
            // Remember the button and modifiers for the single-contact gesture:
            // the arbitration (box / orbit / pan) reads them, and the UI reports
            // no button change on a move.
            self.navigation.button.set(button);
            self.navigation.press_modifiers.set(Some(modifiers));
            *self.navigation.box_gesture.borrow_mut() = None;
            // A pending click-suppression latch belongs to the previous gesture:
            // its click never arrived (a drag emits no `SelectAt`), so it must
            // not swallow this gesture's click.
            self.navigation.suppress_click.set(false);
            // The press is the gesture arbiter: landing on a handle of the
            // selection arms a grip drag here — before the box/pan decisions in
            // `pointer_move` can claim the pointer — so every later phase only has
            // to ask whether a drag is armed. Primary button only: the session
            // emits `SelectAt` for the primary button alone, and a middle-button
            // press on a grip must stay a pan.
            if button == Some(PointerButton::Primary) {
                self.arm_grip_at(point);
            }
        } else {
            // Hard rule: multi-touch navigation replaces any active tool and
            // never commits a capture. It also takes over any single-contact box
            // gesture and any armed grip drag: the winner is navigation, never a
            // rubber band or a geometry edit.
            self.clear_box_gesture();
            self.cancel_grip_drag();
            self.controller.borrow_mut().cancel_tool()?;
            self.navigation.dragged.set(true);
            self.navigation.navigating.set(true);
            self.navigation.nav_last.set(Some(point));
            self.full_dirty.set(true);
        }
        Ok(())
    }

    fn pointer_move(
        &self,
        id: PointerId,
        button: Option<PointerButton>,
        point: [f64; 2],
    ) -> CadResult<()> {
        if !self.navigation.active.borrow().contains(&id) {
            // A move for a pointer that never went down (hover) is not a gesture.
            return Ok(());
        }
        // An armed grip drag owns the single contact for its whole life: it is
        // serviced **before** the capture / modal-pan / box early returns and
        // before the drag threshold, otherwise the stream would die on the first
        // move and the release would commit a stale target. No pan is emitted and
        // no box gesture starts while it is armed.
        if self.track_grip_drag(point) {
            return Ok(());
        }
        let contacts = self.navigation.active.borrow().len();
        if contacts >= 2 {
            self.clear_box_gesture();
            self.navigation.navigating.set(true);
            if let Some(last) = self.navigation.nav_last.get() {
                let delta = [point[0] - last[0], point[1] - last[1]];
                if delta != [0.0, 0.0] {
                    self.pan_by_logical_delta(delta)?;
                }
            }
            self.navigation.nav_last.set(Some(point));
            return Ok(());
        }
        // A draw/measure capture owns single-contact gestures: the UI delivers
        // those as MoveCursor / CapturePoint, so the pointer path never pans and
        // never starts a box selection. A capture that becomes active *mid* box
        // drag must also end the live gesture: otherwise its frozen band keeps
        // drawing and the eventual release dispatches a `SelectInBox` the tool
        // layer refuses, with no visible result (ADR 0014 B.6).
        if self.capturing() {
            self.clear_box_gesture();
            return Ok(());
        }
        // A typed `ToolState::Panning` modal pan also owns the pointer (ADR 0014
        // B.2): a 2D primary drag must neither start a box nor keep a stale one.
        if self.pan_modal_active() {
            self.clear_box_gesture();
        }
        // An active box gesture owns the single contact: track the live
        // rectangle. The camera must not move and no pan is emitted.
        if self.navigation.box_gesture.borrow().is_some() {
            self.track_box_gesture(point);
            return Ok(());
        }
        let nav = &self.navigation;
        // The recorded press button is authoritative; the move's button is a
        // fallback for a host that supplies one directly.
        let pressed = nav.button.get().or(button);
        if !nav.dragged.get() {
            let anchor = nav.anchor.get().unwrap_or(point);
            if logical_distance(anchor, point) < DRAG_THRESHOLD_LOGICAL_PX {
                return Ok(());
            }
            nav.dragged.set(true);
            nav.last.set(Some(anchor));
            // The single winner for this gesture (ADR 0014 B.2): a 2D
            // primary-button drag is a box selection, not a pan. A capture
            // (handled above), a typed modal pan and 3D keep their own
            // behaviour.
            if pressed == Some(PointerButton::Primary)
                && !self.mode_is_3d()
                && !self.pan_modal_active()
            {
                self.begin_box_gesture(anchor, point);
                return Ok(());
            }
            nav.navigating.set(true);
        } else if !nav.navigating.get() {
            return Ok(());
        }
        let from = nav.last.get().unwrap_or(point);
        if self.mode_is_3d() && pressed == Some(PointerButton::Primary) {
            let (yaw, pitch) = orbit_delta(from, point);
            self.orbit(yaw, pitch)?;
        } else {
            if !matches!(self.controller.borrow().session.tool, ToolState::Panning) {
                self.controller.borrow_mut().session.begin_pan();
                self.full_dirty.set(true);
            }
            self.pan_by_logical_delta([point[0] - from[0], point[1] - from[1]])?;
        }
        nav.last.set(Some(point));
        Ok(())
    }

    fn pointer_up(&self, id: PointerId, point: [f64; 2], modifiers: Modifiers) -> CadResult<()> {
        let mut active = self.navigation.active.borrow_mut();
        let Some(index) = active.iter().position(|active| active == &id) else {
            return Ok(());
        };
        active.remove(index);
        let remaining = active.len();
        drop(active);
        if remaining > 0 {
            // Still navigating: keep the gesture alive for the contacts left. A
            // box gesture cannot continue under multi-touch, so it is dropped.
            self.clear_box_gesture();
            self.navigation.anchor.set(None);
            self.navigation.nav_last.set(Some(point));
            // The stored button/modifiers belong to the single-contact gesture
            // whose contact has now left; a surviving contact must not inherit
            // them (the UI reports no button change on a move), so a 2D
            // primary-drag arbitration or a Shift merge can never leak from a
            // released contact. A later single contact re-records them on its
            // own press.
            self.navigation.button.set(None);
            self.navigation.press_modifiers.set(None);
            return Ok(());
        }
        let was_navigating = self.navigation.navigating.replace(false);
        self.navigation.dragged.set(false);
        self.navigation.anchor.set(None);
        self.navigation.last.set(None);
        self.navigation.nav_last.set(None);
        let box_gesture = self.navigation.box_gesture.borrow_mut().take();
        self.navigation.button.set(None);
        self.navigation.press_modifiers.set(None);
        // A release that ends an armed grip drag commits exactly one
        // `GripStretch` (or nothing when the handle never moved) and claims the
        // click the session emits right after this release: a click that lands on
        // a grip must not also select. This runs before the box branch because a
        // grip drag and a rubber band are mutually exclusive winners of the same
        // press, and the grip won at press time.
        if self.grip_drag_active() {
            let committed = self.commit_grip_drag();
            self.navigation.suppress_click.set(true);
            committed?;
            return Ok(());
        }
        if let Some(gesture) = box_gesture {
            // A 2D primary drag selects a box: dispatch the real command through
            // the normal funnel and surface any refusal. The gesture and its
            // preview are already cleared, so a failure leaves no stale band.
            return self.finish_box_gesture(gesture, modifiers);
        }
        if was_navigating && matches!(self.controller.borrow().session.tool, ToolState::Panning) {
            self.controller.borrow_mut().cancel_tool()?;
            self.full_dirty.set(true);
        }
        Ok(())
    }

    fn pointer_cancel(&self, id: PointerId) -> CadResult<()> {
        // A cancel/blur ends the interaction: no hover from the interrupted
        // gesture and no rubber band may remain.
        self.clear_hover();
        self.clear_box_gesture();
        // An armed grip drag is dropped, not committed: nothing was written
        // before the release, so this is a pure state drop.
        self.cancel_grip_drag();
        // The pointer is gone (a cancel/blur carries no position): drop the
        // cursor overlay too rather than leaving the crosshair at a stale spot.
        self.cursor_logical.set(None);
        let mut active = self.navigation.active.borrow_mut();
        active.retain(|active| active != &id);
        let empty = active.is_empty();
        drop(active);
        if empty {
            self.navigation.anchor.set(None);
            self.navigation.last.set(None);
            self.navigation.nav_last.set(None);
            self.navigation.dragged.set(false);
            self.navigation.navigating.set(false);
            self.navigation.button.set(None);
            self.navigation.press_modifiers.set(None);
            // An OS/browser pointer cancel must not leave an open tool behind.
            self.controller.borrow_mut().cancel_tool()?;
            self.full_dirty.set(true);
        } else {
            // A surviving contact must not inherit the cancelled contact's
            // button/modifiers (see `pointer_up`).
            self.navigation.button.set(None);
            self.navigation.press_modifiers.set(None);
        }
        Ok(())
    }

    /// Start a box-selection drag (ADR 0014 B.2/B.9).
    ///
    /// The anchor is the press position; `current` is the move that crossed the
    /// threshold. The hover tint is cleared so a gesture never shows a stale
    /// tint. No pan is emitted and `ToolState::Panning` is never entered.
    fn begin_box_gesture(&self, anchor: [f64; 2], current: [f64; 2]) {
        let shift = self
            .navigation
            .press_modifiers
            .get()
            .is_some_and(|modifiers| modifiers.shift);
        *self.navigation.box_gesture.borrow_mut() = Some(BoxGesture {
            anchor,
            current,
            shift,
        });
        self.clear_hover();
    }

    /// Track the live rectangle of an active box gesture.
    fn track_box_gesture(&self, point: [f64; 2]) {
        if let Some(gesture) = self.navigation.box_gesture.borrow_mut().as_mut() {
            gesture.current = point;
        }
    }

    /// Drop an active box gesture (and therefore its preview input).
    pub(super) fn clear_box_gesture(&self) {
        *self.navigation.box_gesture.borrow_mut() = None;
    }

    /// Dispatch the release of a box gesture as the real `SelectInBox` command.
    ///
    /// The rectangle is in canvas logical pixels (the same convention as
    /// `ZoomAt`), `crossing` follows AutoCAD's right-to-left rule
    /// (`current.x < anchor.x`), and the mode is `Add` when Shift was held.
    /// A non-finite or degenerate rectangle is dropped rather than dispatched —
    /// the command would refuse it, and a rubber band of zero extent is not a
    /// selection.
    fn finish_box_gesture(&self, gesture: BoxGesture, modifiers: Modifiers) -> CadResult<()> {
        let rect_logical = [gesture.anchor, gesture.current];
        if !rect_logical.iter().flatten().all(|value| value.is_finite()) {
            // ADR 0014 B.6: the command API refuses a non-finite rectangle with
            // `InvalidInput`. The GUI keeps its behaviour (a non-finite drag is
            // not a selection, so nothing is dispatched) but the refusal is
            // surfaced rather than silently dropped.
            let _ = self.set_external_status("box selection refused: non-finite rectangle");
            return Ok(());
        }
        let width = (gesture.current[0] - gesture.anchor[0]).abs();
        let height = (gesture.current[1] - gesture.anchor[1]).abs();
        if width <= 0.0 || height <= 0.0 {
            // A perfectly axis-aligned drag has no area to select; never dispatch
            // a degenerate rectangle. ADR 0014 B.6: the same rectangle is
            // `InvalidInput` at the command layer, so the host surfaces that
            // refusal through its normal status channel instead of dropping it
            // silently.
            let _ = self.set_external_status("box selection refused: degenerate rectangle");
            return Ok(());
        }
        let crossing = gesture.current[0] < gesture.anchor[0];
        let mode = if gesture.shift || modifiers.shift {
            SelectionMode::Add
        } else {
            SelectionMode::Replace
        };
        self.immediate_command(
            CommandId::SelectInBox,
            CommandPayload::BoxSelect {
                rect_logical,
                crossing,
                mode,
            },
        )
    }

    /// The world-space rubber band for the live box gesture, or `None`.
    ///
    /// The rectangle's two logical corners are projected to the work plane with
    /// the viewport's own `screen_to_world` (the same mapping measurement picks
    /// use — never a hand-rolled second projection). A WINDOW rectangle emits its
    /// four solid edges; a CROSSING rectangle dashes them in **logical-pixel**
    /// screen space (nominal 5 px dash, 5 px gap) and converts each dash's
    /// endpoints, so the dash length stays uniform on screen regardless of zoom.
    /// For a perimeter large enough to exceed [`BOX_MAX_SEGMENTS`] the step grows
    /// uniformly, the final dash of each edge reaches its corner, and the count
    /// stays bounded — the rectangle is always closed, never truncated.
    pub(crate) fn box_selection_preview(&self) -> Option<BoxSelectionPreview> {
        let gesture = *self.navigation.box_gesture.borrow();
        let gesture = gesture?;
        let c = self.controller.borrow();
        let viewport = c.application.workspace.viewports.get(&c.viewport_id)?;
        let size = viewport.logical_size;
        let anchor = gesture.anchor;
        let current = gesture.current;
        let corners = [
            anchor,
            [current[0], anchor[1]],
            current,
            [anchor[0], current[1]],
        ];
        let crossing = current[0] < anchor[0];
        let mut segments: Vec<[Point3; 2]> = Vec::new();
        // The CROSSING dash/gap step is derived from the rectangle's
        // logical-pixel perimeter: the nominal 5 px dash / 5 px gap while the
        // perimeter fits the segment bound, growing uniformly beyond it so the
        // dashed perimeter is always fully covered and the count stays bounded
        // (ADR 0014 B.9). A fixed pattern truncated whole trailing edges on a
        // large window and drew an *open* rectangle.
        let step = if crossing {
            let perimeter: f64 = (0..4)
                .map(|index| {
                    let start = corners[index];
                    let end = corners[(index + 1) % 4];
                    let dx = end[0] - start[0];
                    let dy = end[1] - start[1];
                    (dx * dx + dy * dy).sqrt()
                })
                .filter(|length| length.is_finite())
                .sum();
            // Reserve room for the (at most) four edge-closing dashes so the
            // total never exceeds the cap. A non-finite perimeter yields an
            // infinite step, which degrades each edge to one solid segment
            // rather than looping.
            let cap = BOX_MAX_SEGMENTS.saturating_sub(8).max(1) as f64;
            (perimeter / cap).max(BOX_DASH_LOGICAL_PX + BOX_GAP_LOGICAL_PX)
        } else {
            0.0
        };
        for index in 0..4 {
            let start = corners[index];
            let end = corners[(index + 1) % 4];
            if crossing {
                let dx = end[0] - start[0];
                let dy = end[1] - start[1];
                let length = (dx * dx + dy * dy).sqrt();
                if !length.is_finite() || length <= 0.0 {
                    continue;
                }
                let (ux, uy) = (dx / length, dy / length);
                let dash = step / 2.0;
                let mut travelled = 0.0;
                while travelled < length {
                    // The final dash absorbs the trailing gap so the edge always
                    // reaches its corner: the rectangle is never drawn open and
                    // the last segment returns to the start point.
                    let dash_end = if length - travelled <= step {
                        length
                    } else {
                        travelled + dash
                    };
                    let from = [start[0] + ux * travelled, start[1] + uy * travelled];
                    let to = [start[0] + ux * dash_end, start[1] + uy * dash_end];
                    if let (Some(world_from), Some(world_to)) = (
                        viewport.screen_to_world(from, size),
                        viewport.screen_to_world(to, size),
                    ) {
                        segments.push([world_from, world_to]);
                    }
                    travelled += step;
                }
            } else if let (Some(world_from), Some(world_to)) = (
                viewport.screen_to_world(start, size),
                viewport.screen_to_world(end, size),
            ) {
                segments.push([world_from, world_to]);
            }
        }
        Some(BoxSelectionPreview { segments, crossing })
    }

    pub(super) fn capturing(&self) -> bool {
        matches!(
            &self.controller.borrow().session.tool,
            ToolState::Drawing(_) | ToolState::Measuring(_)
        )
    }

    /// Whether a typed `ToolState::Panning` modal pan currently owns the pointer.
    ///
    /// `BeginPan` replaces any capture with a modal pan (`cad_app`'s
    /// `begin_pan`); that pan owns the pointer, so a 2D primary drag must fall
    /// through to pan navigation rather than start a box selection (ADR 0014
    /// B.2).
    fn pan_modal_active(&self) -> bool {
        matches!(self.controller.borrow().session.tool, ToolState::Panning)
    }

    /// Whether the active view is in 3D navigation mode.
    ///
    /// Keyed off the session's [`cad_app::ViewMode2d3d`], never the camera
    /// projection, so a legitimate 3D view with an orthographic camera still
    /// orbits (the legacy adapter's `is_3d`). This also matches the application
    /// layer, whose `Orbit` command requires `view_mode.kind() == ThreeD`.
    ///
    /// Visible to the sibling input modules: grip arming refuses a 3D view for
    /// the same reason (no grip markers there and no 3D grip editing).
    pub(super) fn mode_is_3d(&self) -> bool {
        let c = self.controller.borrow();
        c.application
            .workspace
            .viewports
            .get(&c.viewport_id)
            .is_some_and(|viewport| viewport.view_mode.kind() == cad_app::ProjectionKind::ThreeD)
    }

    /// Pan the view by a logical-pixel delta, converted to world units.
    ///
    /// The same `world_per_px` the renderer uses converts the delta, so a pan
    /// keeps the picked world point under the pointer.
    fn pan_by_logical_delta(&self, delta: [f64; 2]) -> CadResult<()> {
        if delta == [0.0, 0.0] {
            return Ok(());
        }
        let world_per_px = {
            let c = self.controller.borrow();
            c.application
                .workspace
                .viewports
                .get(&c.viewport_id)
                .ok_or(CadError::Cancelled)?
                .world_per_px()
        };
        self.immediate_command(
            CommandId::Pan,
            CommandPayload::Points(vec![Point3 {
                x: delta[0] * world_per_px,
                y: -delta[1] * world_per_px,
                z: 0.0,
            }]),
        )
    }

    /// Orbit the 3D view by a `(yaw, pitch)` radian delta.
    fn orbit(&self, yaw: f64, pitch: f64) -> CadResult<()> {
        if yaw == 0.0 && pitch == 0.0 {
            return Ok(());
        }
        self.immediate_command(CommandId::Orbit, CommandPayload::Orbit { yaw, pitch })
    }
}

/// Radians of orbit per logical pixel of drag.
///
/// Mirrors the shared navigation constant; the UI-free host owns the orbit
/// decision under the engine boundary.
const ORBIT_RADIANS_PER_PIXEL: f64 = 0.008;

/// Convert a 3D orbit drag delta (logical pixels) into `(yaw, pitch)` radians.
///
/// Horizontal drag yaws about world `+Z`; vertical drag pitches about the view
/// right axis. Non-finite input yields `(0, 0)` rather than poisoning a camera.
fn orbit_delta(from: [f64; 2], to: [f64; 2]) -> (f64, f64) {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    if !dx.is_finite() || !dy.is_finite() {
        return (0.0, 0.0);
    }
    (-dx * ORBIT_RADIANS_PER_PIXEL, dy * ORBIT_RADIANS_PER_PIXEL)
}

/// Euclidean distance between two logical positions.
fn logical_distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}
