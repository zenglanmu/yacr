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
/// Minimum drag extent, in logical pixels, for a `ZOOM WINDOW` capture.
///
/// A drag thinner than this on either axis is a slip, not a window: it is
/// refused by name in `finish_zoom_window_gesture` and dispatches nothing. The
/// command layer refuses a degenerate rectangle too — this is the host's own
/// threshold, so a one-pixel twitch never even reaches it.
const ZOOM_WINDOW_MIN_DRAG_LOGICAL_PX: f64 = 2.0;

impl HostCore {
    pub(super) fn pointer(
        &self,
        id: PointerId,
        kind: PointerKind,
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
            PointerPhase::Down => self.pointer_down(id, kind, button, point, modifiers),
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
        kind: PointerKind,
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
            // Remember the button, device and modifiers for the single-contact
            // gesture: the arbitration (box / orbit / pan) reads them, and the UI
            // reports no button change on a move. The device is recorded here
            // (absent on the wire ⇒ mouse) so a later move cannot change it
            // mid-gesture.
            self.navigation.button.set(button);
            self.navigation.contact_kind.set(Some(kind));
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
                // A STRETCH crossing-window capture in lasso mode is waiting for
                // its ring: the press starts the freehand trace (the release hands
                // it over), so no box selection and no pan competes for it. This is
                // the same single arbitration point the grip/box decision uses.
                if self.stretch_lasso_waiting() {
                    *self.navigation.lasso_gesture.borrow_mut() = Some(LassoGesture::new(point));
                }
            }
        } else {
            // Hard rule: multi-touch navigation replaces any active tool and
            // never commits a capture. It also takes over any single-contact box
            // gesture and any armed grip drag: the winner is navigation, never a
            // rubber band or a geometry edit.
            self.clear_box_gesture();
            self.clear_lasso_gesture();
            self.cancel_grip_drag();
            self.disarm_zoom_window();
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
            self.clear_lasso_gesture();
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
        // A live lasso owns the single contact for its whole life: it belongs to
        // the active STRETCH capture, so it is serviced **before** the capture
        // early return (that return is exactly the state it runs in). Only the
        // path grows; the camera never moves and no box gesture starts.
        if self.navigation.lasso_gesture.borrow().is_some() {
            self.track_lasso(point);
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
            // A `ZOOM WINDOW` arm cannot fire while a capture owns the pointer,
            // so it is dropped instead of surviving the tool and hijacking a
            // later drag.
            self.disarm_zoom_window();
            return Ok(());
        }
        // A typed `ToolState::Panning` modal pan also owns the pointer (ADR 0014
        // B.2): a 2D primary drag must neither start a box nor keep a stale one.
        if self.pan_modal_active() {
            self.clear_box_gesture();
            self.disarm_zoom_window();
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
            // The single winner for this gesture (ADR 0014 B.2, input-modality
            // contract): a 2D primary-button **mouse/pen** drag is a box
            // selection, while a **touch** drag pans (one-finger pan) — and the
            // optional `interaction.leftDragPans` preference turns the
            // mouse/pen drag into a pan too. A capture (handled above), a typed
            // modal pan, an armed `ZOOM WINDOW` capture and 3D keep their own
            // behaviour (their priority is unchanged).
            let touch = nav.contact_kind.get() == Some(PointerKind::Touch);
            let pans = (touch || self.left_drag_pans()) && !self.zoom_window_armed();
            if pressed == Some(PointerButton::Primary)
                && !self.mode_is_3d()
                && !self.pan_modal_active()
                && !pans
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
            // A 3D orbit is navigation, never a window: an arm that predates the
            // 3D switch is dropped rather than kept for some later 2D drag.
            self.disarm_zoom_window();
            // The user's sensitivity preference is read per event (like
            // `leftDragPans`), so a settings write is live on the next drag.
            let radians_per_pixel = orbit_radians_per_pixel(self.orbit_sensitivity_percent());
            let (yaw, pitch) = orbit_delta(from, point, radians_per_pixel);
            self.orbit(yaw, pitch)?;
        } else {
            self.disarm_zoom_window();
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
            // The stored button/modifiers/device belong to the single-contact
            // gesture whose contact has now left; a surviving contact must not
            // inherit them (the UI reports no button change on a move), so a 2D
            // primary-drag arbitration, a touch-pan rule or a Shift merge can
            // never leak from a released contact. A later single contact
            // re-records them on its own press.
            self.navigation.button.set(None);
            self.navigation.contact_kind.set(None);
            self.navigation.press_modifiers.set(None);
            return Ok(());
        }
        let was_navigating = self.navigation.navigating.replace(false);
        self.navigation.dragged.set(false);
        self.navigation.anchor.set(None);
        self.navigation.last.set(None);
        self.navigation.nav_last.set(None);
        let box_gesture = self.navigation.box_gesture.borrow_mut().take();
        let lasso_gesture = self.navigation.lasso_gesture.borrow_mut().take();
        self.navigation.button.set(None);
        self.navigation.contact_kind.set(None);
        self.navigation.press_modifiers.set(None);
        // A release that ends a live lasso hands its traced ring to the active
        // STRETCH capture (which then asks for the base and displacement points).
        // It is not a click, so nothing selects and no box is finished.
        if let Some(lasso) = lasso_gesture {
            return self.finish_lasso_gesture(lasso);
        }
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
        // A live lasso is dropped, never handed over: nothing was captured, so the
        // STRETCH capture keeps waiting for its window (a pure state drop).
        self.clear_lasso_gesture();
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
            self.navigation.contact_kind.set(None);
            self.navigation.press_modifiers.set(None);
            // An OS/browser pointer cancel must not leave an open tool behind.
            self.controller.borrow_mut().cancel_tool()?;
            self.full_dirty.set(true);
        } else {
            // A surviving contact must not inherit the cancelled contact's
            // button/device/modifiers (see `pointer_up`).
            self.navigation.button.set(None);
            self.navigation.contact_kind.set(None);
            self.navigation.press_modifiers.set(None);
        }
        Ok(())
    }

    /// Start a box-selection drag (ADR 0014 B.2/B.9).
    ///
    /// The anchor is the press position; `current` is the move that crossed the
    /// threshold. The hover tint is cleared so a gesture never shows a stale
    /// tint. No pan is emitted and `ToolState::Panning` is never entered.
    ///
    /// An armed `ZOOM WINDOW` capture ([`HostCore::begin_zoom_window`]) is
    /// recorded on the gesture here — the single arbitration point — so the
    /// release dispatches a zoom rectangle instead of a selection box.
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
            window_zoom: self.navigation.zoom_window_armed.get(),
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
    ///
    /// A gesture that started while a `ZOOM WINDOW` capture was armed is the
    /// zoom rectangle instead (`finish_zoom_window_gesture`): one drag
    /// means one gesture, never both.
    fn finish_box_gesture(&self, gesture: BoxGesture, modifiers: Modifiers) -> CadResult<()> {
        if gesture.window_zoom {
            return self.finish_zoom_window_gesture(gesture);
        }
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

    /// Arm the **one-shot** `ZOOM WINDOW` two-corner capture.
    ///
    /// The typed `ZOOM WINDOW` line reaches this through the session
    /// (`cad-ui-core`), never through a command payload: arming is transient
    /// gesture state with no `CommandId` and no engine request of its own. The
    /// next 2D primary drag then becomes the zoom rectangle and its release
    /// dispatches exactly one [`CommandPayload::ZoomWindow`]
    /// (`finish_zoom_window_gesture`); the drag path is the box-select
    /// path, so the corners are **logical pixels** and no drafting constraint
    /// (object snap, grid snap, `ORTHO`) can distort them.
    ///
    /// Refusals are explicit, never a silent arm that can never fire:
    ///
    /// * a 3D observation — `zoom.window.requires_2d` (a window is a plan
    ///   rectangle; faking a perspective fit is not an option);
    /// * no active viewport — `CadError::Cancelled`, the host's "no surface"
    ///   shape;
    /// * a draw/measurement capture or a modal pan that owns the pointer —
    ///   `zoom.window.capture_busy` (nothing would ever start the rectangle).
    pub fn begin_zoom_window(&self) -> CadResult<()> {
        {
            let c = self.controller.borrow();
            let viewport = c
                .application
                .workspace
                .viewports
                .get(&c.viewport_id)
                .ok_or(CadError::Cancelled)?;
            if viewport.view_mode.kind() != cad_app::ProjectionKind::TwoD
                || !viewport.camera.projection.is_orthographic()
            {
                return Err(CadError::InvalidInput(
                    "zoom.window.requires_2d: ZOOM WINDOW is only defined in the 2D plan view; a \
                     perspective view has no window to fit (switch back to 2D first)"
                        .into(),
                ));
            }
        }
        if self.capturing() || self.pan_modal_active() {
            return Err(CadError::InvalidInput(
                "zoom.window.capture_busy: another capture owns the pointer; finish or cancel it \
                 before picking a ZOOM WINDOW rectangle"
                    .into(),
            ));
        }
        if !self.navigation.zoom_window_armed.replace(true) {
            // A fresh arm starts a fresh capture: a live band or hover tint from
            // an earlier gesture would describe a rectangle that is gone.
            self.clear_box_gesture();
            self.clear_hover();
        }
        Ok(())
    }

    /// Whether the `ZOOM WINDOW` two-corner capture is currently armed.
    pub fn zoom_window_armed(&self) -> bool {
        self.navigation.zoom_window_armed.get()
    }

    /// Drop an armed `ZOOM WINDOW` capture without dispatching anything.
    ///
    /// One entry for every non-zoom terminal: a cancel, a gesture clear, a
    /// multi-touch takeover, a capture/modal pan owning the pointer and a
    /// pan/orbit drag. Pure state — nothing was written while armed.
    pub(super) fn disarm_zoom_window(&self) {
        self.navigation.zoom_window_armed.set(false);
    }

    /// Dispatch the release of a `ZOOM WINDOW` drag as one `ZoomWindow` payload.
    ///
    /// The rectangle is the drag's two corners in canvas **logical** pixels,
    /// canonicalised to `[min, max]` by the shared
    /// [`cad_app::canonical_logical_rect`] rule (the same rule the command
    /// validates with, so the host cannot disagree with it). A non-finite corner,
    /// or a drag thinner than [`ZOOM_WINDOW_MIN_DRAG_LOGICAL_PX`] on either axis,
    /// is **refused by name and dispatched not at all**: a one-pixel slip is not
    /// a window, and the command would refuse a degenerate rectangle anyway. The
    /// arm is consumed either way (one-shot capture).
    fn finish_zoom_window_gesture(&self, gesture: BoxGesture) -> CadResult<()> {
        self.disarm_zoom_window();
        let rect_logical = [gesture.anchor, gesture.current];
        if !rect_logical.iter().flatten().all(|value| value.is_finite()) {
            let _ = self.set_external_status("zoom.window.invalid_rect: non-finite rectangle");
            return Ok(());
        }
        let width = (gesture.current[0] - gesture.anchor[0]).abs();
        let height = (gesture.current[1] - gesture.anchor[1]).abs();
        if width < ZOOM_WINDOW_MIN_DRAG_LOGICAL_PX || height < ZOOM_WINDOW_MIN_DRAG_LOGICAL_PX {
            let _ = self.set_external_status(format!(
                "zoom.window.degenerate_rect: the drag is thinner than \
                 {ZOOM_WINDOW_MIN_DRAG_LOGICAL_PX} logical pixels on an axis"
            ));
            return Ok(());
        }
        match cad_app::canonical_logical_rect(rect_logical) {
            Ok(rect_logical) => {
                self.immediate_command(CommandId::Zoom, CommandPayload::ZoomWindow { rect_logical })
            }
            Err(error) => {
                let _ = self.set_external_status(crate::error_text(&error));
                Ok(())
            }
        }
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
        // A live STRETCH lasso is the same transient gesture input: its traced path
        // is the CROSSING boundary the user is drawing, so it goes through the one
        // rubber-band overlay rather than a second preview channel.
        if let Some(preview) = self.lasso_preview() {
            return Some(preview);
        }
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
        // A `ZOOM WINDOW` capture is always a solid WINDOW rectangle: the
        // left-to-right CROSSING convention describes a *selection* semantic and
        // has no meaning for a zoom, so it never dashes the zoom band.
        let crossing = !gesture.window_zoom && current[0] < anchor[0];
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

    /// Wheel zoom: the same `ZoomAt` payload as the discrete `ZOOMIN`/`ZOOMOUT`
    /// steps, declared **continuous navigation**.
    ///
    /// The application cannot distinguish a wheel notch from a typed/discrete
    /// step — both are one `ZoomAt` — so the host states the difference here, at
    /// the single place that maps a scroll to a zoom: it remembers the history
    /// length, runs the ordinary command, and drops the one `ZOOM PREVIOUS` step
    /// that command recorded. A wheel notch is therefore never restorable by
    /// `ZOOM PREVIOUS` (the declared divergence documented in
    /// `cad_app::zoom_history`); pan and orbit need no such rule because they
    /// never record a step at all.
    pub(super) fn zoom_continuously(&self, factor: f64, cursor: [f64; 2]) -> CadResult<()> {
        let before = self.view_zoom_history_len();
        self.immediate_command(CommandId::Zoom, CommandPayload::ZoomAt { factor, cursor })?;
        if self.view_zoom_history_len() > before {
            let mut c = self.controller.borrow_mut();
            let id = c.viewport_id;
            if let Some(viewport) = c.application.workspace.viewports.get_mut(&id) {
                viewport.discard_last_view_snapshot();
            }
        }
        Ok(())
    }

    /// The active viewport's recorded `ZOOM PREVIOUS` depth (`0` without one).
    fn view_zoom_history_len(&self) -> usize {
        let c = self.controller.borrow();
        c.application
            .workspace
            .viewports
            .get(&c.viewport_id)
            .map(|viewport| viewport.zoom_history_len())
            .unwrap_or(0)
    }

    /// Pan the view by a logical-pixel delta, converted to world units.
    ///
    /// The same `world_per_px` the renderer uses converts the delta, so a pan
    /// keeps the picked world point under the pointer.
    pub(super) fn pan_by_logical_delta(&self, delta: [f64; 2]) -> CadResult<()> {
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

/// Base rate of 3D orbit: radians per **logical** pixel at `100` % sensitivity.
///
/// This is only the base rate; the user's multiplier is
/// `interaction.orbitSensitivityPercent` (`100` = this rate), applied by
/// [`orbit_radians_per_pixel`]. The unit is deliberately **logical pixels**, not
/// physical pixels and not "per DPI": a surface already applies its scale factor
/// to the pointer positions, so a per-physical-pixel rate would make the same
/// hand movement rotate twice as fast on a HiDPI display.
const ORBIT_RADIANS_PER_PIXEL: f64 = 0.0035;

/// Defensive bounds on the user multiplier: `25` % and `200` %, the schema's own
/// accepted range. A preference that was hand-edited past validation can never
/// produce a runaway orbit angle.
const ORBIT_SENSITIVITY_MIN_MULTIPLIER: f64 = 0.25;
const ORBIT_SENSITIVITY_MAX_MULTIPLIER: f64 = 2.0;

/// Effective radians of orbit per logical pixel for a sensitivity `percent`.
///
/// `100` is [`ORBIT_RADIANS_PER_PIXEL`], `50` halves it and `200` doubles it. A
/// non-finite or non-positive percentage falls back to the base rate; a finite
/// percentage is clamped into the schema range (`25..=200`, as a multiplier
/// `0.25..=2.0`) rather than trusted, because the orbit path must not depend on
/// the durable document having been validated.
pub(crate) fn orbit_radians_per_pixel(percent: f64) -> f64 {
    let percent = if percent.is_finite() && percent > 0.0 {
        percent
    } else {
        crate::DEFAULT_ORBIT_SENSITIVITY_PERCENT
    };
    let multiplier = (percent / crate::DEFAULT_ORBIT_SENSITIVITY_PERCENT).clamp(
        ORBIT_SENSITIVITY_MIN_MULTIPLIER,
        ORBIT_SENSITIVITY_MAX_MULTIPLIER,
    );
    ORBIT_RADIANS_PER_PIXEL * multiplier
}

/// Convert a 3D orbit drag delta (logical pixels) into `(yaw, pitch)` radians.
///
/// Horizontal drag yaws about world `+Z`; vertical drag pitches about the view
/// right axis. `radians_per_pixel` is the effective rate
/// ([`orbit_radians_per_pixel`]). A non-finite input or rate yields `(0, 0)`
/// rather than poisoning a camera with a NaN angle — the same finite-only
/// contract the shared `OrbitDelta` wire type applies to `cad_orbit`.
pub(crate) fn orbit_delta(from: [f64; 2], to: [f64; 2], radians_per_pixel: f64) -> (f64, f64) {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    if !dx.is_finite() || !dy.is_finite() || !radians_per_pixel.is_finite() {
        return (0.0, 0.0);
    }
    (-dx * radians_per_pixel, dy * radians_per_pixel)
}

/// Euclidean distance between two logical positions.
fn logical_distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod orbit_rate_tests {
    //! The rate arithmetic itself: the base rate, the user multiplier and the
    //! defensive clamp. The end-to-end camera effect is pinned by
    //! `crate::tests::settings_orbit`.

    use super::*;

    /// `100` % is the documented `0.0035` rad per logical pixel.
    #[test]
    fn the_base_rate_is_the_hundred_percent_rate() {
        assert!((orbit_radians_per_pixel(100.0) - 0.0035).abs() < 1e-15);
    }

    /// The multiplier is linear across the accepted range (`25..=200`).
    #[test]
    fn the_multiplier_is_linear_between_25_and_200() {
        assert!((orbit_radians_per_pixel(50.0) - 0.00175).abs() < 1e-15);
        assert!((orbit_radians_per_pixel(25.0) - 0.000875).abs() < 1e-15);
        assert!((orbit_radians_per_pixel(200.0) - 0.007).abs() < 1e-15);
    }

    /// A hand-edited preference outside the schema range cannot run away: the
    /// multiplier is clamped to `[0.25, 2.0]`, and a non-finite percentage falls
    /// back to the base rate.
    #[test]
    fn an_out_of_range_or_non_finite_percentage_cannot_run_away() {
        assert_eq!(
            orbit_radians_per_pixel(500.0),
            orbit_radians_per_pixel(200.0)
        );
        assert_eq!(orbit_radians_per_pixel(1.0), orbit_radians_per_pixel(25.0));
        assert_eq!(
            orbit_radians_per_pixel(f64::NAN),
            orbit_radians_per_pixel(100.0)
        );
        assert_eq!(
            orbit_radians_per_pixel(f64::INFINITY),
            orbit_radians_per_pixel(100.0)
        );
        assert_eq!(
            orbit_radians_per_pixel(-50.0),
            orbit_radians_per_pixel(100.0)
        );
    }

    /// A 100 px drag at 100 % is `0.35` rad yaw; vertical drag pitches with the
    /// opposite sign convention and the same magnitude.
    #[test]
    fn a_hundred_pixel_drag_yaws_the_base_rate() {
        let rate = orbit_radians_per_pixel(100.0);
        let (yaw, pitch) = orbit_delta([100.0, 100.0], [200.0, 100.0], rate);
        assert!((yaw + 0.35).abs() < 1e-12, "{yaw}");
        assert_eq!(pitch, 0.0);

        let (yaw, pitch) = orbit_delta([0.0, 0.0], [0.0, 100.0], rate);
        assert_eq!(yaw, 0.0);
        assert!((pitch - 0.35).abs() < 1e-12, "{pitch}");
    }

    /// A non-finite delta or rate produces no rotation rather than a NaN angle.
    #[test]
    fn a_non_finite_delta_or_rate_yields_no_rotation() {
        let rate = orbit_radians_per_pixel(100.0);
        assert_eq!(orbit_delta([0.0, 0.0], [f64::NAN, 0.0], rate), (0.0, 0.0));
        assert_eq!(orbit_delta([0.0, 0.0], [10.0, f64::NAN], rate), (0.0, 0.0));
        assert_eq!(
            orbit_delta([0.0, 0.0], [10.0, 10.0], f64::INFINITY),
            (0.0, 0.0)
        );
    }
}
