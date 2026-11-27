//! Interaction request dispatch and coordinate conversion.
use super::*;
use cad_app::interaction::PointerPhase;
use cad_app::plugin_prompt::{ApiPromptInput, ApiPromptKind};

/// Which derived shell state one interaction invalidates.
///
/// `Overlay` is the hover case: a hovered entity only tints the renderer's
/// flags input, which `sync_render` refreshes on every dispatch anyway, so no
/// published state (`HostState`/`NavigationState`) changes and a mere cursor
/// move schedules no host update.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DirtyScope {
    /// Only the transient render overlay inputs change.
    Overlay,
    /// Only the navigation/camera view state changes.
    Navigation,
    /// Every derived shell state changes.
    Full,
}

impl HostCore {
    fn world_point(&self, input: PointInput) -> CadResult<Point3> {
        match input.validate()? {
            PointInput::World(point) => Ok(point),
            PointInput::CanvasLocal {
                logical_position,
                metrics_revision,
            } => {
                self.metrics()?;
                if metrics_revision != self.metrics_revision.get() {
                    return Err(CadError::InvalidInput(
                        "stale canvas metrics revision".into(),
                    ));
                }
                let c = self.controller.borrow();
                let viewport = c
                    .application
                    .workspace
                    .viewports
                    .get(&c.viewport_id)
                    .ok_or(CadError::Cancelled)?;
                cad_app::input::ViewMetrics::new(self.canvas_metrics.get(), viewport)
                    .surface_to_world(logical_position)
                    .ok_or_else(|| {
                        CadError::InvalidInput("canvas point is outside the viewport".into())
                    })
            }
        }
    }

    /// Resolve a cursor-bearing request to canvas-local logical coordinates, for
    /// the pointer-cursor overlay input.
    ///
    /// Mirrors `world_point`/`hover_logical`: a canvas-local point is used as-is
    /// after its metrics revision is validated, and a world point is projected
    /// through the active camera. Only used to remember where the pointer is for
    /// the crosshair; the interaction itself still runs its own resolution.
    fn cursor_logical_of(&self, input: PointInput) -> CadResult<[f64; 2]> {
        match input.validate()? {
            PointInput::CanvasLocal {
                logical_position,
                metrics_revision,
            } => {
                self.metrics()?;
                if metrics_revision != self.metrics_revision.get() {
                    return Err(CadError::InvalidInput(
                        "stale canvas metrics revision".into(),
                    ));
                }
                Ok(logical_position)
            }
            PointInput::World(point) => {
                let c = self.controller.borrow();
                let viewport = c
                    .application
                    .workspace
                    .viewports
                    .get(&c.viewport_id)
                    .ok_or(CadError::Cancelled)?;
                viewport
                    .camera
                    .world_to_screen(point, viewport.logical_size)
                    .ok_or_else(|| CadError::InvalidInput("world point cannot be projected".into()))
            }
        }
    }

    pub(crate) fn dispatch_interaction(
        &self,
        request: InteractionRequest,
    ) -> CadResult<InteractionOutcome> {
        request.validate()?;
        // Pointer and scroll are navigation-only: they must not force a full
        // state re-query. A hover-only `MoveCursor` and a `ClearCursor` with no
        // active capture are overlay-only for the same reason (both demote to
        // `DirtyScope::Overlay` inside their arms); every other interaction
        // changes derived shell state.
        let mut scope = match &request {
            InteractionRequest::Pointer { .. }
            | InteractionRequest::Scroll { .. }
            | InteractionRequest::Pinch { .. } => DirtyScope::Navigation,
            _ => DirtyScope::Full,
        };
        // A pending plugin prompt owns the prompt-bearing inputs: it is answered
        // here, **before** the tool state machine, and the shared publication tail
        // below still runs (a selection prompt publishes a selection).
        // A pending plugin prompt owns the prompt-bearing inputs: it is
        // answered here, **before** the tool state machine, and the derived
        // state is published exactly like the normal path's full-dirty tail
        // (a selection prompt publishes a selection).
        if let Some(outcome) = self.resolve_api_prompt(&request)? {
            self.full_dirty.set(true);
            self.navigation_dirty.set(true);
            if let Err(error) = self.sync_render() {
                self.note_post_commit_error(&error);
            }
            return Ok(outcome);
        }
        let outcome = match request {
            InteractionRequest::BeginDraw(kind) => {
                // CHAMFEREDGE refuses **at tool start**, before any click: a
                // carrier without a modeler and a selection that is not exactly
                // one addressable exact solid are both named here, so no pick is
                // silently consumed by a capture that can never commit.
                if matches!(
                    kind,
                    cad_app::DrawToolKind::ChamferEdge | cad_app::DrawToolKind::PressPull
                ) {
                    self.begin_solid_face_tool(kind)?;
                } else {
                    self.controller.borrow_mut().begin_draw_tool(kind)?;
                }
                // A capture owns the pointer: any live box gesture ends with it,
                // so no frozen band survives and no refused `SelectInBox` is
                // dispatched on release (ADR 0014 B.6).
                self.clear_box_gesture();
                self.clear_hover();
                InteractionOutcome::ToolStarted
            }
            InteractionRequest::BeginMeasurement(kind) => {
                self.controller
                    .borrow_mut()
                    .session
                    .begin_measurement(kind)?;
                self.clear_box_gesture();
                self.clear_hover();
                InteractionOutcome::ToolStarted
            }
            InteractionRequest::SetMeasurementKind(kind) => {
                self.controller
                    .borrow_mut()
                    .session
                    .set_measurement_kind(kind)?;
                InteractionOutcome::SelectionKindChanged
            }
            // A retained session tool parameter changed (ADR 0016 decision A). The
            // session validates the parameter's own range/kind and refuses out of
            // range; the outcome reuses `SelectionKindChanged` — the existing "a
            // retained tool/parameter choice changed" result — rather than widening
            // the frozen `InteractionOutcome` enum for one parameter.
            InteractionRequest::SetToolParameter { parameter, value } => {
                self.controller
                    .borrow_mut()
                    .session
                    .set_tool_parameter(parameter, value)?;
                InteractionOutcome::SelectionKindChanged
            }
            // The terminal text of a text tool (ADR 0019 decision A.1). It is
            // stored first (the session validates state/emptiness/length and
            // refuses by name), then the command is completed **immediately**:
            // text is the last step, so a second `Confirm` would be ambiguous and
            // is refused while the prompt is open.
            InteractionRequest::SetToolText { text } => {
                self.controller.borrow_mut().session.set_tool_text(&text)?;
                return self.confirm_tool();
            }
            // The panel edit path (ADR 0020 amendment B): the host resolves the
            // **current single selection** itself (no id travels) and validates
            // it at apply time. Refusals are named and open no transaction; a
            // successful edit is one transaction / one undo step, and the tail
            // below republishes the derived state (including `selection.text`).
            InteractionRequest::SetSelectedText { text } => {
                self.controller.borrow_mut().set_selected_text(&text)?;
                InteractionOutcome::Confirmed
            }
            // The properties-panel write path: like the text field, the host
            // resolves its own current selection at apply time (no id travels).
            // Refusals are named and open no transaction; a successful edit is
            // one transaction / one undo step, and the tail below republishes the
            // derived state (properties rows, selection, undo availability).
            InteractionRequest::SetSelectedProperty { key, value } => {
                self.controller
                    .borrow_mut()
                    .set_selection_property(&key, &value)?;
                InteractionOutcome::Confirmed
            }
            // The first `Esc`: a focus-only change that never cancels the tool
            // (ADR 0019 decision A.1). `Cancel` remains the second `Esc` and is
            // the only request that resets the tool.
            InteractionRequest::DismissTextFocus => {
                self.controller.borrow().session.dismiss_text_focus()?;
                InteractionOutcome::TextFocusDismissed
            }
            InteractionRequest::BeginPan => {
                self.controller.borrow_mut().session.begin_pan();
                // A modal pan owns the pointer: end any live box gesture.
                self.clear_box_gesture();
                self.clear_hover();
                InteractionOutcome::PanStarted
            }
            InteractionRequest::ClearCursor => {
                self.controller.borrow_mut().session.clear_cursor();
                // The pointer is off the surface: forget its position so the
                // crosshair overlay is not left at a stale spot.
                self.cursor_logical.set(None);
                // The UI reports "no cursor on the surface" through this request:
                // the hover it produced must not survive it, and neither may the
                // capture candidate resolved at that departed cursor position
                // (otherwise a TRIM/EXTEND tint stays painted with no pointer).
                self.controller
                    .borrow_mut()
                    .session
                    .set_draw_candidate(None);
                self.clear_hover();
                // Clearing a *capture's* cursor changes the published
                // `draw_preview`/`measurement_preview` inside `HostState`, so a
                // capture keeps the full rebuild. Without a capture this only
                // clears the transient hover overlay, which `sync_render`
                // republishes anyway, so it is overlay-only exactly like the
                // hover `MoveCursor` (ADR-0012 §10 S2): leaving the surface
                // schedules no O(layers+entities) host-state rebuild.
                if !self.capturing() {
                    scope = DirtyScope::Overlay;
                }
                InteractionOutcome::CursorCleared
            }
            InteractionRequest::CapturePoint(point) => {
                // CHAMFEREDGE's one step is a **pixel** pick: the seed is the
                // hit point the pick funnel resolved, never a work-plane
                // projection (which would hard-stamp a fabricated `z` in a 3D
                // view). This branch precedes `world_point` on purpose.
                if matches!(
                    self.controller.borrow().session.draw_kind(),
                    Some(cad_app::DrawToolKind::ChamferEdge | cad_app::DrawToolKind::PressPull)
                ) {
                    // A second click would run a whole pixel pick and target
                    // resolve only to be refused by the capture ("that step
                    // takes no entity pick"). Name the real state instead — the
                    // exact sentence `DrawTool::push_point_checked` uses for a
                    // completed fixed-arity pick capture.
                    if !self.controller.borrow().session.draw_accepts_entity_pick() {
                        let kind = self
                            .controller
                            .borrow()
                            .session
                            .draw_kind()
                            .expect("the face-pick capture kind");
                        return Err(CadError::InvalidInput(format!(
                            "draw kind '{}' is ready; press Enter to confirm",
                            kind.key()
                        )));
                    }
                    let kind = self
                        .controller
                        .borrow()
                        .session
                        .draw_kind()
                        .expect("the face-pick capture kind");
                    self.capture_solid_face_pick(point, kind)?;
                    InteractionOutcome::PointCaptured
                } else {
                    let point = self.world_point(point)?;
                    // AutoCAD's "click the first vertex to close the ring" rule for a
                    // crossing polygon: the click that lands on the first vertex is the
                    // close, not another vertex. The aperture is the same pick box every
                    // other pick uses, and the ring itself is validated by the capture
                    // (a refused close leaves it open and changes nothing).
                    if self.close_stretch_ring_at(point)? {
                        // The ring closed on this click: that is capture state (no
                        // command ran), so only the derived tool state is republished.
                        self.full_dirty.set(true);
                        InteractionOutcome::PointCaptured
                    } else {
                        let auto_ready = self.controller.borrow_mut().capture_tool_point(point)?;
                        if auto_ready {
                            return self.confirm_tool();
                        }
                        InteractionOutcome::PointCaptured
                    }
                }
            }
            InteractionRequest::MoveCursor(point) => {
                // Remember where the pointer is for the renderer's cursor
                // crosshair overlay (both the capture and the hover branch below
                // keep the same logical position). Best-effort: the interaction
                // still runs its own resolution and reports its own error.
                let logical = self.cursor_logical_of(point).ok();
                if self.capturing() {
                    // CHAMFEREDGE's seed is a pixel pick hit point; there is no
                    // work-plane point to move. This request is demoted to the
                    // suppressed-hover path (the drawing tool owns the pointer,
                    // so no pick and no tint run) instead of feeding a fabricated
                    // `z` into the tool cursor. No highlight exists to promise.
                    if matches!(
                        self.controller.borrow().session.draw_kind(),
                        Some(cad_app::DrawToolKind::ChamferEdge | cad_app::DrawToolKind::PressPull)
                    ) {
                        scope = DirtyScope::Overlay;
                        self.resolve_hover(point)?;
                    } else {
                        // A draw/measurement capture owns the cursor: this is the
                        // tool preview cursor, unchanged.
                        let point = self.world_point(point)?;
                        self.controller.borrow_mut().move_tool_cursor(point)?;
                        // The capture, not the pointer, decides what is highlighted.
                        self.clear_hover();
                    }
                } else {
                    // Without a capture the same request is the hover probe
                    // (ADR-0012 §10 S2): it only refreshes the flags overlay.
                    scope = DirtyScope::Overlay;
                    self.resolve_hover(point)?;
                }
                if let Some(logical) = logical {
                    self.cursor_logical.set(Some(logical));
                }
                InteractionOutcome::CursorMoved
            }
            InteractionRequest::Confirm => {
                // ADR 0019 decision A.1/F.3: while a text tool's prompt is open,
                // `Confirm` must never complete the command without text — the
                // terminal step is `SetToolText`. A named refusal, no transaction.
                if self.controller.borrow().session.draw_awaiting_text() {
                    return Err(CadError::InvalidInput(
                        "tool.awaiting_text: the active tool needs its text; type it and press \
                     Enter to submit (Esc dismisses the prompt, Esc again cancels)"
                            .into(),
                    ));
                }
                // A STRETCH polygon capture closes its crossing-window ring on
                // `Confirm` instead of committing: the window is not defined until
                // the ring is closed, so the same key advances the capture. A ring
                // that is still too small is refused by name (no transaction).
                if matches!(
                    self.controller.borrow().session.draw_window_step(),
                    Some(cad_app::StretchWindowStep::PolygonVertex)
                ) {
                    self.controller
                        .borrow_mut()
                        .session
                        .close_draw_window_ring()?;
                    self.full_dirty.set(true);
                    self.navigation_dirty.set(true);
                    if let Err(error) = self.sync_render() {
                        self.note_post_commit_error(&error);
                    }
                    return Ok(InteractionOutcome::PointCaptured);
                }
                return self.confirm_tool();
            }
            InteractionRequest::Cancel => {
                self.controller.borrow_mut().cancel_tool()?;
                self.render.set_draw_preview(None);
                self.render.set_measurement_preview(None);
                self.clear_hover();
                // An armed grip drag is dropped, never committed: nothing was
                // written before the release, so a cancel is a pure state drop.
                // A pending click-suppression latch belongs to the interrupted
                // gesture and dies with it.
                self.reset_grip_gesture();
                InteractionOutcome::Cancelled
            }
            InteractionRequest::SelectAt { point, mode } => {
                self.metrics()?;
                let point = match point {
                    PointInput::CanvasLocal {
                        logical_position,
                        metrics_revision,
                    } => {
                        if metrics_revision != self.metrics_revision.get() {
                            return Err(CadError::InvalidInput(
                                "stale canvas metrics revision".into(),
                            ));
                        }
                        logical_position
                    }
                    PointInput::World(point) => {
                        let c = self.controller.borrow();
                        let vp = c
                            .application
                            .workspace
                            .viewports
                            .get(&c.viewport_id)
                            .ok_or(CadError::Cancelled)?;
                        vp.camera
                            .world_to_screen(point, vp.logical_size)
                            .ok_or_else(|| {
                                CadError::InvalidInput("world point cannot be projected".into())
                            })?
                    }
                };
                self.select_at_mode(point, mode)?;
                InteractionOutcome::SelectionUpdated
            }
            InteractionRequest::Pointer {
                id,
                kind,
                phase,
                button,
                position,
                modifiers,
            } => {
                // A grip drag owns the frame as an overlay-only change: the moved
                // marker **is** the preview, so a press/move/release of a handle
                // publishes no navigation delta (the camera did not move). The
                // release commit runs through the ordinary command funnel and
                // marks the derived state dirty there, exactly like any other
                // write.
                let grip_before = self.grip_drag_active();
                let lasso_before = self.navigation.lasso_gesture.borrow().is_some();
                if let Err(error) = self.pointer(id, kind, phase, button, position, modifiers) {
                    // A stale/invalid coordinate refuses the request before its
                    // phase runs; a refusal of an `Up`/`Cancel` must still clear
                    // the pointer life, exactly as the queue path's stale
                    // handling does, so no `active`/`button`/box state dangles.
                    if matches!(phase, PointerPhase::Up | PointerPhase::Cancel) {
                        self.clear_pointer_life(id);
                    }
                    return Err(error);
                }
                // A lasso gesture feeds the active STRETCH capture, so the published
                // tool state really changes (the ring a release hands over, or the
                // waiting state a cancel restores) and the dispatch keeps its
                // full rebuild. A grip drag stays overlay-only: the moved marker is
                // itself the preview and no camera or tool state changed.
                if lasso_before || self.navigation.lasso_gesture.borrow().is_some() {
                    scope = DirtyScope::Full;
                } else if grip_before || self.grip_drag_active() {
                    scope = DirtyScope::Overlay;
                }
                InteractionOutcome::PointerObserved
            }
            InteractionRequest::Scroll {
                delta_logical,
                position,
                ..
            } => {
                self.metrics()?;
                let point = match position {
                    PointInput::CanvasLocal {
                        logical_position,
                        metrics_revision,
                    } if metrics_revision == self.metrics_revision.get() => logical_position,
                    PointInput::CanvasLocal { .. } => {
                        return Err(CadError::InvalidInput(
                            "stale canvas metrics revision".into(),
                        ))
                    }
                    PointInput::World(point) => {
                        let c = self.controller.borrow();
                        let vp = c
                            .application
                            .workspace
                            .viewports
                            .get(&c.viewport_id)
                            .ok_or(CadError::Cancelled)?;
                        vp.camera
                            .world_to_screen(point, vp.logical_size)
                            .ok_or_else(|| {
                                CadError::InvalidInput("world point cannot be projected".into())
                            })?
                    }
                };
                // A wheel notch is **continuous** navigation: it runs the same
                // `ZoomAt` command but must not record a `ZOOM PREVIOUS` step.
                self.zoom_continuously((1.0 + delta_logical[1] * 0.0015).clamp(0.2, 5.0), point)?;
                InteractionOutcome::ScrollObserved
            }
            // Two-finger pinch: a navigation takeover, never a capture and never a
            // box selection. The order is fixed by the contract: cancel the active
            // tool (a pinch must not leave a half-captured command armed), then pan
            // by the midpoint delta and zoom by the incremental multiplier about
            // the midpoint. Both reuse the existing navigation helpers
            // (`pan_by_logical_delta` / `zoom_continuously`), so a pinch never
            // records a `ZOOM PREVIOUS` step either.
            InteractionRequest::Pinch {
                position,
                delta_logical,
                scale,
            } => {
                // `validate()` already refused these, but the check is repeated at
                // the point of use so no future caller can bypass it and drive the
                // camera with a non-finite/zero/negative factor.
                if !scale.is_finite() || scale <= 0.0 {
                    return Err(CadError::InvalidInput(
                        "pinch scale must be finite and greater than zero".into(),
                    ));
                }
                if !delta_logical.iter().all(|value| value.is_finite()) {
                    return Err(CadError::InvalidInput("pinch delta must be finite".into()));
                }
                let point = self.cursor_logical_of(position)?;
                // Whatever the gesture would otherwise win (box, grip drag, armed
                // `ZOOM WINDOW`), navigation wins: the pinch is the recorded
                // multi-contact takeover, made explicit instead of a second touch
                // contact. A tool that was active is cancelled, never committed.
                let tool_was_active =
                    !matches!(self.controller.borrow().session.tool, ToolState::Idle);
                self.clear_box_gesture();
                self.reset_grip_gesture();
                self.disarm_zoom_window();
                self.controller.borrow_mut().cancel_tool()?;
                self.render.set_draw_preview(None);
                self.render.set_measurement_preview(None);
                self.clear_hover();
                if tool_was_active {
                    // The tool generation/previews changed: republish the derived
                    // state, not just the camera.
                    scope = DirtyScope::Full;
                }
                self.cursor_logical.set(Some(point));
                let history_before = self.view_zoom_history_len();
                if delta_logical != [0.0, 0.0] {
                    self.pan_by_logical_delta(delta_logical)?;
                }
                // A pinch is continuous multi-touch navigation with no
                // begin/end signal: it must not fill the view history with one
                // step per frame. Drop the step the pan half just recorded (the
                // zoom half drops its own inside `zoom_continuously`).
                if self.view_zoom_history_len() > history_before {
                    let mut c = self.controller.borrow_mut();
                    let id = c.viewport_id;
                    if let Some(viewport) = c.application.workspace.viewports.get_mut(&id) {
                        viewport.discard_last_view_snapshot();
                    }
                }
                if scale != 1.0 {
                    self.zoom_continuously(scale, point)?;
                }
                InteractionOutcome::PinchObserved
            }
        };
        match scope {
            DirtyScope::Overlay => {}
            DirtyScope::Navigation => self.navigation_dirty.set(true),
            DirtyScope::Full => {
                self.full_dirty.set(true);
                self.navigation_dirty.set(true);
            }
        }
        // Post-business render sync: an error is an explicit diagnostic, never a
        // reclassification of a successful interaction.
        if let Err(error) = self.sync_render() {
            self.note_post_commit_error(&error);
        }
        Ok(outcome)
    }

    /// Close an open STRETCH polygon ring when the click lands on its first vertex.
    ///
    /// Returns whether the click **was** the close. AutoCAD's rule is to click the
    /// first point again; the aperture is [`TolerancePolicy::pickbox_logical_pixels`]
    /// — the same pick box every other pick, hover and grip use — so "on the
    /// vertex" means the same thing everywhere. Nothing is closed while the ring
    /// has fewer than [`cad_app::STRETCH_MIN_WINDOW_VERTICES`] vertices (a click
    /// near the first point of a one-vertex ring is just another vertex), and a
    /// refused close (a zero-area ring) propagates its named error with the ring
    /// untouched.
    fn close_stretch_ring_at(&self, point: Point3) -> CadResult<bool> {
        let (first, vertices) = {
            let c = self.controller.borrow();
            (
                c.session.draw_window_first_vertex(),
                c.session.draw_window_ring_len(),
            )
        };
        let (Some(first), true) = (first, vertices >= cad_app::STRETCH_MIN_WINDOW_VERTICES) else {
            return Ok(false);
        };
        let (first_logical, click_logical) = {
            let c = self.controller.borrow();
            let Some(viewport) = c.application.workspace.viewports.get(&c.viewport_id) else {
                // No active viewport: nothing to project through, so this click is
                // not a close (the ordinary capture path reports the real refusal).
                return Ok(false);
            };
            let size = viewport.logical_size;
            let first = viewport.camera.world_to_screen(first, size);
            let click = viewport.camera.world_to_screen(point, size);
            match (first, click) {
                // A view without a plan mapping cannot answer "the same vertex" and
                // must not close a ring on a guess.
                (Some(first), Some(click)) => (first, click),
                _ => return Ok(false),
            }
        };
        let aperture = self.tolerance_policy().pickbox_logical_pixels;
        let dx = click_logical[0] - first_logical[0];
        let dy = click_logical[1] - first_logical[1];
        if !(dx * dx + dy * dy).sqrt().is_finite() || (dx * dx + dy * dy).sqrt() > aperture {
            return Ok(false);
        }
        self.controller
            .borrow_mut()
            .session
            .close_draw_window_ring()?;
        Ok(true)
    }

    fn confirm_tool(&self) -> CadResult<InteractionOutcome> {
        // HostController remains the sole transaction/history funnel. Draw
        // capture is cleared only after successful confirmation.
        let measuring = matches!(
            &self.controller.borrow().session.tool,
            ToolState::Measuring(_)
        );
        // A confirm that does not produce an admitted edit must not leave a
        // receipt from an earlier confirmation behind.
        self.pending_edit.borrow_mut().take();
        #[cfg(test)]
        self.last_edit_receipt_for_test.set(false);
        let mut edit = None;
        if measuring {
            self.immediate_command(CommandId::ConfirmMeasurement, CommandPayload::None)?;
        } else {
            // The draw confirmation is a real App command; collect its single
            // trace and surface an inner command failure explicitly (the
            // capture is preserved by `confirm_draw_traced`). The trusted funnel
            // also binds an optional incremental-display receipt to its own
            // before/after database snapshots.
            let (traced, receipt) = self
                .controller
                .borrow_mut()
                .confirm_draw_traced_with_edit()?;
            edit = receipt;
            self.pending_trace.borrow_mut().extend(traced.commands);
            traced.result?;
        }
        *self.pending_edit.borrow_mut() = edit.clone();
        #[cfg(test)]
        self.last_edit_receipt_for_test.set(edit.is_some());
        self.full_dirty.set(true);
        // A draw tool may have created the drawing's first text (a dimension's
        // measurement): install its fallback engine before the rebuild, so the
        // new text is shaped rather than silently left unshaped.
        if let Err(error) = self.ensure_fonts_for_text() {
            self.note_post_commit_error(&error);
        }
        let edit = self.pending_edit.borrow_mut().take();
        if let Err(error) = self.sync_render_with_edit(edit.as_ref()) {
            self.note_post_commit_error(&error);
        }
        Ok(InteractionOutcome::Confirmed)
    }

    /// Answer a pending plugin prompt with one interaction, when one is pending.
    ///
    /// `Some(outcome)` means the prompt consumed the input (so the tool state
    /// machine must not also see it); `None` leaves the ordinary path untouched.
    /// Only the five prompt-bearing requests are intercepted — every pointer,
    /// scroll, pinch and tool request keeps its existing handling.
    fn resolve_api_prompt(
        &self,
        request: &InteractionRequest,
    ) -> CadResult<Option<InteractionOutcome>> {
        let kind = self
            .controller
            .borrow()
            .api_prompt()
            .map(|prompt| prompt.kind);
        let Some(kind) = kind else {
            return Ok(None);
        };
        match request {
            InteractionRequest::CapturePoint(point) => {
                let point = self.world_point(*point)?;
                self.controller
                    .borrow_mut()
                    .resolve_api_prompt(ApiPromptInput::Point(point));
                Ok(Some(InteractionOutcome::PointCaptured))
            }
            InteractionRequest::SelectAt { point, .. } => {
                match kind {
                    ApiPromptKind::Point => {
                        // An idle click reaches the host as `SelectAt`, so a
                        // getPoint prompt takes it as the point (the same
                        // `world_point` conversion the capture path uses).
                        let world = self.world_point(*point)?;
                        self.controller
                            .borrow_mut()
                            .resolve_api_prompt(ApiPromptInput::Point(world));
                    }
                    ApiPromptKind::Entity => {
                        let logical = self.select_logical(*point)?;
                        let hit = self.pick_ref_at(logical)?;
                        self.controller.borrow_mut().resolve_api_prompt(match hit {
                            Some(reference) => ApiPromptInput::Entity(reference.entity),
                            None => ApiPromptInput::EntityMiss,
                        });
                    }
                    ApiPromptKind::Selection => {
                        // The ordinary click-toggle selection, published exactly
                        // like a non-prompt click.
                        let logical = self.select_logical(*point)?;
                        self.select_at_mode(logical, SelectionMode::Toggle)?;
                        self.controller
                            .borrow_mut()
                            .resolve_api_prompt(ApiPromptInput::SelectionChanged);
                    }
                    // A click during a text/point prompt is consumed harmlessly.
                    _ => {
                        self.controller
                            .borrow_mut()
                            .resolve_api_prompt(ApiPromptInput::SelectionChanged);
                    }
                }
                Ok(Some(InteractionOutcome::SelectionUpdated))
            }
            InteractionRequest::SetToolText { text } => {
                self.controller
                    .borrow_mut()
                    .resolve_api_prompt(ApiPromptInput::Text(text.clone()));
                Ok(Some(InteractionOutcome::Confirmed))
            }
            InteractionRequest::Confirm => {
                self.controller
                    .borrow_mut()
                    .resolve_api_prompt(ApiPromptInput::Confirm);
                Ok(Some(InteractionOutcome::Confirmed))
            }
            InteractionRequest::Cancel => {
                // Consumed: the prompt's Esc belongs to the prompt, never to the
                // tool machine.
                self.controller
                    .borrow_mut()
                    .resolve_api_prompt(ApiPromptInput::Cancel);
                Ok(Some(InteractionOutcome::Cancelled))
            }
            _ => Ok(None),
        }
    }

    /// The precise hit under one logical canvas point, through the one pick path.
    ///
    /// Shared by the ordinary click selection, a pending plugin prompt's
    /// entity/selection reads, the hover probe and CHAMFEREDGE's seed — so a
    /// click, a hover and a chamfer seed can never disagree about what is under
    /// the cursor or use a second tolerance. The hit carries both the identity
    /// (`source`) and the model-space point the ray resolved (`point`).
    fn pick_hit_at(&self, point: [f64; 2]) -> CadResult<Option<cad_app::picking::PickHit>> {
        self.metrics()?;
        let c = self.controller.borrow();
        let viewport = c
            .application
            .workspace
            .viewports
            .get(&c.viewport_id)
            .ok_or(CadError::Cancelled)?;
        let drawing = c.drawing().ok_or(CadError::Cancelled)?;
        // The same configured pickbox the hover path and the cursor overlay use,
        // so a click and the box drawn around it can never disagree.
        let policy = self.tolerance_policy();
        let report = cad_app::picking::pick_at_screen_with_visibility_and_vendor(
            &drawing,
            c.document_id,
            &viewport.camera,
            point,
            viewport.logical_size,
            &policy,
            cad_app::BackFacePolicy::Cull,
            &c.session.layer_overrides,
            // HIDEOBJECTS/ISOLATEOBJECTS refs are not clickable.
            &c.session.hidden_entities,
            // Vendor-backed opaque entities are part of the clickable catalogue
            // (ADR-0028 D2); `None` is the exact pre-plugin behaviour.
            c.application.vendor_representations.as_deref(),
        )?;
        Ok(report.hit)
    }

    /// The source ref under one logical canvas point, through the one pick path.
    fn pick_ref_at(&self, point: [f64; 2]) -> CadResult<Option<cad_domain::SelectionRef>> {
        Ok(self.pick_hit_at(point)?.map(|hit| hit.source))
    }

    // `begin_solid_face_tool` lives in `input/solid_face.rs`; the BeginDraw
    // call site above reaches it through the same `impl HostCore`.

    /// Capture a solid face edit's one step: the pixel pick hit point on the
    /// selected body.
    ///
    /// The click is resolved through [`Self::pick_hit_at`] (the one pick funnel,
    /// its pickbox aperture and its visibility rules) and the captured point is
    /// `PickHit.point`, never a work-plane projection. The hit must name the
    /// **currently selected** body — document, entity and instance path compared,
    /// sub-element ignored, because a body click legitimately resolves a face —
    /// and the normalised body ref is stored with the point. A miss or a hit on
    /// another object is a named refusal that leaves the capture armed.
    fn capture_solid_face_pick(
        &self,
        point: PointInput,
        kind: cad_app::DrawToolKind,
    ) -> CadResult<()> {
        let pick = solid_face_tool(kind)?;
        let logical = self.select_logical(point)?;
        let hit = self.pick_hit_at(logical)?.ok_or_else(|| {
            CadError::InvalidInput(format!(
                "{}.pick_missed: no object is under the cursor; pick a face on the selected body",
                pick.code
            ))
        })?;
        // A mesh face capture is kernel-free and has no exact-body target; its
        // identity is the selected row + face sub-element. The exact path still
        // resolves the body through the command's own target resolver.
        let mesh_target = {
            let c = self.controller.borrow();
            let refs = c.session.selection.refs();
            (kind == cad_app::DrawToolKind::PressPull
                && refs.len() == 1
                && refs[0].sub_element.is_some()
                && refs[0].instance.0.is_empty()
                && c.application
                    .workspace
                    .documents
                    .get(&c.document_id)
                    .and_then(|document| document.drawing.entity(refs[0].entity))
                    .is_some_and(mesh_face_edit_target))
            .then(|| refs[0].clone())
        };
        let (target_document, target_entity, target_instance, selected) = if let Some(selected) =
            mesh_target
        {
            (
                selected.document,
                selected.entity,
                selected.instance.clone(),
                selected,
            )
        } else {
            let c = self.controller.borrow();
            let target = (pick.target)(&c.application, c.document_id, c.session.selection.refs())?;
            (
                target.body.document,
                target.body.entity,
                target.body.instance.clone(),
                target.body,
            )
        };
        if hit.source.document != target_document
            || hit.source.entity != target_entity
            || hit.source.instance != target_instance
        {
            return Err(CadError::InvalidInput(format!(
                "{}.pick_not_selected_body: the pick hit entity {} but the selected body is \
                 entity {}; select the body you want to edit",
                pick.code, hit.source.entity.0, target_entity.0
            )));
        }
        self.controller
            .borrow_mut()
            .session
            .capture_draw_pick(hit.point, selected)
    }

    /// One `SelectAt`/`CapturePoint` input as logical canvas coordinates.
    fn select_logical(&self, point: PointInput) -> CadResult<[f64; 2]> {
        self.metrics()?;
        match point {
            PointInput::CanvasLocal {
                logical_position,
                metrics_revision,
            } => {
                if metrics_revision != self.metrics_revision.get() {
                    return Err(CadError::InvalidInput(
                        "stale canvas metrics revision".into(),
                    ));
                }
                Ok(logical_position)
            }
            PointInput::World(point) => {
                let c = self.controller.borrow();
                let vp = c
                    .application
                    .workspace
                    .viewports
                    .get(&c.viewport_id)
                    .ok_or(CadError::Cancelled)?;
                vp.camera
                    .world_to_screen(point, vp.logical_size)
                    .ok_or_else(|| CadError::InvalidInput("world point cannot be projected".into()))
            }
        }
    }

    fn select_at_mode(&self, point: [f64; 2], mode: SelectionMode) -> CadResult<()> {
        // A click that lands on a grip was already answered by the grip drag: the
        // session emits `[Pointer Up, SelectAt]` for a click, so a release that
        // committed a grip drag swallows the fabricated selection here. One shot:
        // the latch is consumed, and a press resets it (see `pointer_down`), so a
        // drag that produced no click never swallows a later one.
        if self.navigation.suppress_click.replace(false) {
            return Ok(());
        }
        // A `ZOOM WINDOW` capture is waiting for two corners: a bare click is
        // not one, so it must not select either. The arm is deliberately kept —
        // the next drag supplies the rectangle.
        if self.zoom_window_armed() {
            return Ok(());
        }
        self.metrics()?;
        if !matches!(
            &self.controller.borrow().session.tool,
            ToolState::Idle | ToolState::Selecting
        ) {
            return Ok(());
        }
        let hit = self.pick_ref_at(point)?;
        let c = self.controller.borrow();
        let mut refs = c.session.selection.refs().to_vec();
        match mode {
            SelectionMode::Replace => refs = hit.into_iter().collect(),
            SelectionMode::Add => {
                if let Some(hit) = hit {
                    if !refs.contains(&hit) {
                        refs.push(hit);
                    }
                }
            }
            SelectionMode::Toggle => {
                if let Some(hit) = hit {
                    if let Some(index) = refs.iter().position(|existing| existing == &hit) {
                        refs.remove(index);
                    } else {
                        refs.push(hit);
                    }
                }
            }
            SelectionMode::Remove => {
                if let Some(hit) = hit {
                    refs.retain(|existing| existing != &hit);
                }
            }
        }
        drop(c);
        self.immediate_command(CommandId::Select, CommandPayload::Selection(refs))
    }
}
