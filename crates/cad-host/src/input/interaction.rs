//! Interaction request dispatch and coordinate conversion.
use super::*;
use cad_app::interaction::PointerPhase;

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
            InteractionRequest::Pointer { .. } | InteractionRequest::Scroll { .. } => {
                DirtyScope::Navigation
            }
            _ => DirtyScope::Full,
        };
        let outcome = match request {
            InteractionRequest::BeginDraw(kind) => {
                self.controller.borrow_mut().begin_draw_tool(kind)?;
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
                let point = self.world_point(point)?;
                let auto_ready = self.controller.borrow_mut().capture_tool_point(point)?;
                if auto_ready {
                    return self.confirm_tool();
                }
                InteractionOutcome::PointCaptured
            }
            InteractionRequest::MoveCursor(point) => {
                // Remember where the pointer is for the renderer's cursor
                // crosshair overlay (both the capture and the hover branch below
                // keep the same logical position). Best-effort: the interaction
                // still runs its own resolution and reports its own error.
                let logical = self.cursor_logical_of(point).ok();
                if self.capturing() {
                    // A draw/measurement capture owns the cursor: this is the
                    // tool preview cursor, unchanged.
                    let point = self.world_point(point)?;
                    self.controller.borrow_mut().move_tool_cursor(point)?;
                    // The capture, not the pointer, decides what is highlighted.
                    self.clear_hover();
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
            InteractionRequest::Confirm => return self.confirm_tool(),
            InteractionRequest::Cancel => {
                self.controller.borrow_mut().cancel_tool()?;
                self.render.set_draw_preview(None);
                self.render.set_measurement_preview(None);
                self.clear_hover();
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
                phase,
                button,
                position,
                modifiers,
                ..
            } => {
                if let Err(error) = self.pointer(id, phase, button, position, modifiers) {
                    // A stale/invalid coordinate refuses the request before its
                    // phase runs; a refusal of an `Up`/`Cancel` must still clear
                    // the pointer life, exactly as the queue path's stale
                    // handling does, so no `active`/`button`/box state dangles.
                    if matches!(phase, PointerPhase::Up | PointerPhase::Cancel) {
                        self.clear_pointer_life(id);
                    }
                    return Err(error);
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
                self.immediate_command(
                    CommandId::Zoom,
                    CommandPayload::ZoomAt {
                        factor: (1.0 + delta_logical[1] * 0.0015).clamp(0.2, 5.0),
                        cursor: point,
                    },
                )?;
                InteractionOutcome::ScrollObserved
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

    fn confirm_tool(&self) -> CadResult<InteractionOutcome> {
        // HostController remains the sole transaction/history funnel. Draw
        // capture is cleared only after successful confirmation.
        let measuring = matches!(
            &self.controller.borrow().session.tool,
            ToolState::Measuring(_)
        );
        if measuring {
            self.immediate_command(CommandId::ConfirmMeasurement, CommandPayload::None)?;
        } else {
            // The draw confirmation is a real App command; collect its single
            // trace and surface an inner command failure explicitly (the
            // capture is preserved by `confirm_draw_traced`).
            let traced = self.controller.borrow_mut().confirm_draw_traced()?;
            self.pending_trace.borrow_mut().extend(traced.commands);
            traced.result?;
        }
        self.full_dirty.set(true);
        // A draw tool may have created the drawing's first text (a dimension's
        // measurement): install its fallback engine before the rebuild, so the
        // new text is shaped rather than silently left unshaped.
        if let Err(error) = self.ensure_fonts_for_text() {
            self.note_post_commit_error(&error);
        }
        if let Err(error) = self.sync_render() {
            self.note_post_commit_error(&error);
        }
        Ok(InteractionOutcome::Confirmed)
    }

    fn select_at_mode(&self, point: [f64; 2], mode: SelectionMode) -> CadResult<()> {
        self.metrics()?;
        let c = self.controller.borrow();
        if !matches!(&c.session.tool, ToolState::Idle | ToolState::Selecting) {
            return Ok(());
        }
        let viewport = c
            .application
            .workspace
            .viewports
            .get(&c.viewport_id)
            .ok_or(CadError::Cancelled)?;
        let drawing = c.drawing().ok_or(CadError::Cancelled)?;
        let report = cad_app::picking::pick_at_screen_with_visibility(
            &drawing,
            c.document_id,
            &viewport.camera,
            point,
            viewport.logical_size,
            &cad_domain::TolerancePolicy::default(),
            cad_app::BackFacePolicy::Cull,
            &c.session.layer_overrides,
        )?;
        let hit = report.hit.map(|hit| hit.source);
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
