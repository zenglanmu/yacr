//! UI-independent interaction dispatch and coordinate conversion.
use super::*;
use cad_app::input::DRAG_THRESHOLD_LOGICAL_PX;
use cad_app::interaction::PointerPhase;
use cad_engine::events::{EventPoll, EventWatermark, SubscriptionId};
use std::time::Instant;

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

/// Whether the opt-in pick profile log is enabled.
///
/// The same environment gate the pick path itself uses (`YACR_PROFILE_PICK`,
/// see `cad_app::picking`), read here so the hover wrapper can label its own
/// measurement. Off by default: no always-on metric exists.
fn hover_profile_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("YACR_PROFILE_PICK").is_some())
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
                self.clear_hover();
                InteractionOutcome::ToolStarted
            }
            InteractionRequest::BeginMeasurement(kind) => {
                self.controller
                    .borrow_mut()
                    .session
                    .begin_measurement(kind)?;
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
            InteractionRequest::BeginPan => {
                self.controller.borrow_mut().session.begin_pan();
                self.clear_hover();
                InteractionOutcome::PanStarted
            }
            InteractionRequest::ClearCursor => {
                self.controller.borrow_mut().session.clear_cursor();
                // The UI reports "no cursor on the surface" through this request:
                // the hover it produced must not survive it.
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
                ..
            } => {
                self.pointer(id, phase, button, position)?;
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
        let report = cad_app::picking::pick_at_screen(
            &drawing,
            c.document_id,
            &viewport.camera,
            point,
            viewport.logical_size,
            &cad_domain::TolerancePolicy::default(),
            cad_app::BackFacePolicy::Cull,
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

    /// Whether a hover query must be suppressed right now.
    ///
    /// A positive tool (draw/measure capture or pan), a press/navigation gesture
    /// (pan/orbit/drag), or an active multi-contact navigation owns the pointer;
    /// hover must not compete with it. `Selecting` deliberately does not
    /// suppress: after a click the user must be able to hover the next object.
    /// This is the second line of defence — the session already emits no hover
    /// request for a pressed pointer — so a host that delivers one anyway never
    /// leaves a stale tint behind.
    fn hover_suppressed(&self) -> bool {
        let tool_owns_pointer = matches!(
            &self.controller.borrow().session.tool,
            ToolState::Drawing(_) | ToolState::Measuring(_) | ToolState::Panning
        );
        tool_owns_pointer
            || self.navigation.dragged.get()
            || self.navigation.navigating.get()
            || !self.navigation.active.borrow().is_empty()
    }

    /// Clear the hovered set and forget the last hover query position.
    ///
    /// Pure session/overlay state: no database write, no history entry, and no
    /// published shell state change, so nothing is marked dirty here. The
    /// `sync_render` that ends every interaction dispatch republishes the
    /// overlay inputs.
    pub(crate) fn clear_hover(&self) {
        self.hover.hovered.borrow_mut().clear();
        self.hover.last_query.set(None);
    }

    /// The number of hover pick queries this host has actually executed.
    ///
    /// Read-only observable behaviour for the hover throttle contract: an
    /// unchanged cursor position re-runs no pick, so the counter does not move.
    /// It is a counter, never a wall-clock measurement.
    pub fn hover_query_count(&self) -> u64 {
        self.hover.queries.get()
    }

    /// The canvas-local logical cursor position a hover request names.
    ///
    /// A canvas-local point is used as-is and its metrics revision validated
    /// exactly like `SelectAt`'s; a world point is projected through the active
    /// camera for a host that supplies one directly.
    fn hover_logical(&self, point: PointInput) -> CadResult<[f64; 2]> {
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

    /// Resolve the top-ranked pick under the cursor and publish it as hover.
    ///
    /// Hover reuses the **exact** click-selection path — `pick_at_screen` with
    /// the default `TolerancePolicy` and `BackFacePolicy::Cull` — so there is no
    /// second tolerance, no second ranking and no second identity type. Hover is
    /// entity-level and single: the one top-ranked ref the report returns.
    ///
    /// Throttling (ADR-0012 §10 S2): the UI already coalesces pointer moves to
    /// at most one per animation frame, so no time-window throttle exists here;
    /// an unchanged logical position re-runs no pick at all.
    fn resolve_hover(&self, point: PointInput) -> CadResult<()> {
        if self.hover_suppressed() {
            self.clear_hover();
            return Ok(());
        }
        let logical = self.hover_logical(point)?;
        if self.hover.last_query.get() == Some(logical) {
            return Ok(());
        }
        let profile = hover_profile_enabled();
        let started = profile.then(Instant::now);
        self.metrics()?;
        let picked = {
            let c = self.controller.borrow();
            let viewport = c
                .application
                .workspace
                .viewports
                .get(&c.viewport_id)
                .ok_or(CadError::Cancelled)?;
            let drawing = c.drawing().ok_or(CadError::Cancelled)?;
            let report = cad_app::picking::pick_at_screen(
                &drawing,
                c.document_id,
                &viewport.camera,
                logical,
                viewport.logical_size,
                &cad_domain::TolerancePolicy::default(),
                cad_app::BackFacePolicy::Cull,
            );
            report.map(|report| report.hit.map(|hit| hit.source))
        };
        let picked = match picked {
            Ok(picked) => picked,
            Err(error) => {
                // A hover that cannot be resolved is an explicit refusal, never
                // a silent "still hovering the previous entity".
                self.clear_hover();
                return Err(error);
            }
        };
        if let Some(started) = started {
            // Opt-in (`YACR_PROFILE_PICK`, the same gate the pick path itself
            // uses) and measured, never estimated. A local/synthetic number: it
            // is not a performance conclusion.
            eprintln!(
                "YACR_PROFILE_PICK hover elapsed_ms={:.3} picked={}",
                started.elapsed().as_secs_f64() * 1000.0,
                picked.is_some()
            );
        }
        self.hover.last_query.set(Some(logical));
        self.hover
            .queries
            .set(self.hover.queries.get().saturating_add(1));
        let mut hovered = self.hover.hovered.borrow_mut();
        let changed = match &picked {
            // Hover is single: an equal one-ref set is not a change.
            Some(reference) => !(hovered.len() == 1 && hovered.contains(reference)),
            None => !hovered.is_empty(),
        };
        if changed {
            match picked {
                Some(reference) => hovered.replace([reference]),
                None => hovered.clear(),
            }
        }
        Ok(())
    }

    fn pointer(
        &self,
        id: PointerId,
        phase: PointerPhase,
        button: Option<PointerButton>,
        position: PointInput,
    ) -> CadResult<()> {
        let point = self.pointer_logical(position)?;
        match phase {
            PointerPhase::Down => self.pointer_down(id, point),
            PointerPhase::Move => self.pointer_move(id, button, point),
            PointerPhase::Up => self.pointer_up(id, point),
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

    fn pointer_down(&self, id: PointerId, point: [f64; 2]) -> CadResult<()> {
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
        } else {
            // Hard rule: multi-touch navigation replaces any active tool and
            // never commits a capture.
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
        let contacts = self.navigation.active.borrow().len();
        if contacts >= 2 {
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
        // those as MoveCursor / CapturePoint, so the pointer path never pans.
        if self.capturing() {
            return Ok(());
        }
        let nav = &self.navigation;
        if !nav.dragged.get() {
            let anchor = nav.anchor.get().unwrap_or(point);
            if logical_distance(anchor, point) < DRAG_THRESHOLD_LOGICAL_PX {
                return Ok(());
            }
            nav.dragged.set(true);
            nav.navigating.set(true);
            nav.last.set(Some(anchor));
        } else if !nav.navigating.get() {
            return Ok(());
        }
        let from = nav.last.get().unwrap_or(point);
        if self.mode_is_3d() && button == Some(PointerButton::Primary) {
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

    fn pointer_up(&self, id: PointerId, point: [f64; 2]) -> CadResult<()> {
        let mut active = self.navigation.active.borrow_mut();
        let Some(index) = active.iter().position(|active| active == &id) else {
            return Ok(());
        };
        active.remove(index);
        let remaining = active.len();
        drop(active);
        if remaining > 0 {
            // Still navigating: keep the gesture alive for the contacts left.
            self.navigation.anchor.set(None);
            self.navigation.nav_last.set(Some(point));
            return Ok(());
        }
        let was_navigating = self.navigation.navigating.replace(false);
        self.navigation.dragged.set(false);
        self.navigation.anchor.set(None);
        self.navigation.last.set(None);
        self.navigation.nav_last.set(None);
        if was_navigating && matches!(self.controller.borrow().session.tool, ToolState::Panning) {
            self.controller.borrow_mut().cancel_tool()?;
            self.full_dirty.set(true);
        }
        Ok(())
    }

    fn pointer_cancel(&self, id: PointerId) -> CadResult<()> {
        // A cancel/blur ends the interaction: no hover from the interrupted
        // gesture may remain.
        self.clear_hover();
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
            // An OS/browser pointer cancel must not leave an open tool behind.
            self.controller.borrow_mut().cancel_tool()?;
            self.full_dirty.set(true);
        }
        Ok(())
    }

    fn capturing(&self) -> bool {
        matches!(
            &self.controller.borrow().session.tool,
            ToolState::Drawing(_) | ToolState::Measuring(_)
        )
    }

    /// Whether the active view is in 3D navigation mode.
    ///
    /// Keyed off the session's [`cad_app::ViewMode2d3d`], never the camera
    /// projection, so a legitimate 3D view with an orthographic camera still
    /// orbits (the legacy adapter's `is_3d`). This also matches the application
    /// layer, whose `Orbit` command requires `view_mode.kind() == ThreeD`.
    fn mode_is_3d(&self) -> bool {
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

impl HostCore {
    /// Whether a queued request is stale against the current host context.
    ///
    /// Every request checks document / document epoch / viewport, and a command
    /// also its own target; tool requests additionally check the tool
    /// generation; canvas-local requests check the metrics revision; gesture
    /// requests check the config revision.
    fn request_is_stale(&self, context: &RequestContext, request: &EngineRequest) -> bool {
        let current = self.request_context();
        if context.document != current.document
            || context.document_generation != current.document_generation
            || context.viewport != current.viewport
        {
            return true;
        }
        if let EngineRequest::Command(command) = request {
            if command.document != current.document || command.viewport != current.viewport {
                return true;
            }
        }
        if tool_generation_checked(request) && context.tool_generation != current.tool_generation {
            return true;
        }
        if let Some(metrics_revision) = request_metrics_revision(request) {
            if Some(metrics_revision) != current.metrics_revision {
                return true;
            }
        }
        if config_checked(request) && context.config_revision != current.config_revision {
            return true;
        }
        false
    }

    /// Acquire the synchronous dispatch guard, or `Busy` while queued/executing
    /// work or another pump/clock/sync window is active.
    pub(crate) fn begin_sync_request(&self) -> Result<ActiveGuard<'_>, RequestError> {
        if self.request_engine_busy() || self.queue.pending() || self.queue.executing_id().is_some()
        {
            return Err(RequestError::Busy);
        }
        Ok(ActiveGuard::new(&self.sync_active))
    }

    /// Clear the pointer life for a stale `Up`/`Cancel` without navigating.
    fn clear_pointer_life(&self, id: PointerId) {
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
        }
    }

    /// Invoke the registered waker with every queue borrow released.
    pub(crate) fn invoke_request_waker(&self) {
        let waker = self.request_waker.borrow().clone();
        if let Some(waker) = waker {
            waker();
        }
    }

    fn on_stale_request(&self, request: &EngineRequest) {
        if let EngineRequest::Interaction(InteractionRequest::Pointer {
            id,
            phase: PointerPhase::Up | PointerPhase::Cancel,
            ..
        }) = request
        {
            self.clear_pointer_life(*id);
        }
    }

    /// One bounded pump pass over the queue.
    ///
    /// The time budget is a single per-pass 4 ms window (not per request): the
    /// clock is sampled before each `begin` and the pass stops once the budget is
    /// exhausted. Without an installed clock, pending work is refused with an
    /// explicit `Unsupported` and no request is taken; an empty queue needs no
    /// clock.
    fn pump_inner(&self) -> Result<PumpReport, RequestError> {
        // No executable work: nothing to do, and no clock is needed (or sampled).
        if !self.queue.pending() {
            return Ok(self.queue.report(0));
        }
        // Clone the clock out and release the borrow before running the callback.
        let clock = self.request_clock.borrow().clone();
        let Some(clock) = clock else {
            return Err(RequestError::Execution(CadError::Unsupported(
                "request clock is not configured for this host".into(),
            )));
        };

        let start = clock();
        if !start.is_finite() || start < 0.0 {
            return Err(RequestError::Execution(CadError::InvalidInput(
                "request clock must return a finite non-negative millisecond timestamp".into(),
            )));
        }
        let mut previous = start;
        let mut budget = cad_engine::queue::PumpBudget::new();
        let mut processed = 0usize;
        loop {
            // Sample and validate before each request so the 4 ms window is a
            // per-pass budget, never a per-request preemption. A non-finite or
            // backwards sample is an explicit failure, never a fabricated time.
            let now = clock();
            if !now.is_finite() || now < previous || now < start {
                return Err(RequestError::Execution(CadError::InvalidInput(
                    "request clock must return a finite non-decreasing timestamp".into(),
                )));
            }
            previous = now;
            if budget.is_exhausted(now - start) {
                break;
            }
            let Some(mut executing) = self.queue.begin() else {
                break;
            };
            budget.record();
            let origin = executing.origin();
            let context = *executing.context();
            let Some(request) = executing.take_request() else {
                let error = RequestError::Execution(CadError::Invariant(
                    "queued request had no payload".into(),
                ));
                if let Some(entry) = self.permits.borrow_mut().remove(&executing.id()) {
                    let mut permit = entry.permit;
                    self.publish_finished_only(
                        &mut permit,
                        entry.origin,
                        &entry.context,
                        executing.id(),
                        &Err(RequestError::Execution(CadError::Invariant(
                            "queued request had no payload".into(),
                        ))),
                    );
                }
                executing.complete(Err(error));
                processed += 1;
                continue;
            };
            if self.request_is_stale(&context, &request) {
                // A stale queued request publishes only its terminal Finished
                // (no Started), with its old context; no business runs.
                self.on_stale_request(&request);
                self.finish_queued_stale(executing, origin, context);
                processed += 1;
                continue;
            }
            self.run_queued(executing, origin, request);
            processed += 1;
        }
        // Re-arm only while executable work remains, then notify once. All queue
        // borrows are released here; the waker only schedules.
        if self.queue.rearm_wake() && self.queue.take_wake() {
            self.invoke_request_waker();
        }
        Ok(self.queue.report(processed))
    }
}

/// Whether a queued request must also match the tool generation.
fn tool_generation_checked(request: &EngineRequest) -> bool {
    matches!(
        request,
        EngineRequest::Interaction(
            InteractionRequest::MoveCursor(_)
                | InteractionRequest::CapturePoint(_)
                | InteractionRequest::Confirm
                | InteractionRequest::ClearCursor
        )
    )
}

/// Whether a request carries a coordinate/gesture intent and so must also match
/// the current config revision (World or canvas-local alike).
///
/// `BeginDraw`/`BeginMeasurement`/`SetMeasurementKind`/`BeginPan`/`ClearCursor`/
/// `Confirm`/`Cancel` carry no coordinate and keep their original rules; metrics
/// are checked separately and only for canvas-local coordinates.
fn config_checked(request: &EngineRequest) -> bool {
    matches!(
        request,
        EngineRequest::Interaction(
            InteractionRequest::SelectAt { .. }
                | InteractionRequest::CapturePoint(_)
                | InteractionRequest::MoveCursor(_)
                | InteractionRequest::Pointer { .. }
                | InteractionRequest::Scroll { .. }
        )
    )
}

/// The metrics revision a request's canvas-local coordinate was tagged with.
fn request_metrics_revision(request: &EngineRequest) -> Option<u64> {
    let point = match request {
        EngineRequest::Interaction(InteractionRequest::CapturePoint(point))
        | EngineRequest::Interaction(InteractionRequest::MoveCursor(point)) => *point,
        EngineRequest::Interaction(InteractionRequest::SelectAt { point, .. }) => *point,
        EngineRequest::Interaction(InteractionRequest::Pointer { position, .. }) => *position,
        EngineRequest::Interaction(InteractionRequest::Scroll { position, .. }) => *position,
        _ => return None,
    };
    match point {
        PointInput::CanvasLocal {
            metrics_revision, ..
        } => Some(metrics_revision),
        PointInput::World(_) => None,
    }
}

/// Whether a request may be admitted through the asynchronous `submit` path.
///
/// Commands that need a user gesture (file pickers, backend switch) must run
/// synchronously through `dispatch`.
pub(crate) fn admissible_for_submit(request: &EngineRequest) -> bool {
    match request {
        EngineRequest::Command(command) => !matches!(
            command.id,
            CommandId::OpenDrawing
                | CommandId::PlotDrawing
                | CommandId::SaveDrawingAs
                | CommandId::SwitchBackend
        ),
        EngineRequest::Interaction(_) => true,
        // Controlled API requests are queue-admissible; only gesture commands
        // are refused (P3 rule unchanged).
        EngineRequest::Api { .. } => true,
    }
}

impl EngineClient for HostCore {
    fn dispatch(&self, request: EngineRequest) -> Result<RequestResult, RequestError> {
        // The legacy `dispatch` is the `Ui` shorthand for an origin-tagged,
        // traced synchronous request.
        self.dispatch_with_origin_impl(RequestOrigin::Ui, request)
            .and_then(|completion| completion.result)
    }

    fn request_context(&self) -> RequestContext {
        let c = self.controller.borrow();
        RequestContext {
            document: c.document_id,
            // The independent host document epoch, not the session generation:
            // edits/tools/camera never bump it.
            document_generation: self.document_epoch.get(),
            viewport: c.viewport_id,
            tool_generation: Some(c.session.tool_generation),
            metrics_revision: Some(self.metrics_revision.get()),
            config_revision: self.viewer_config.borrow().revision,
        }
    }

    fn take_updates(&self) -> CadResult<HostUpdates> {
        self.sync_render()?;
        let full_dirty = self.full_dirty.get();
        let navigation_dirty = self.navigation_dirty.get();
        let config_dirty = self.config_dirty.get();
        let status_dirty = self.status_dirty.get();
        let full = if full_dirty {
            Some(self.full_state()?)
        } else {
            None
        };
        let navigation = if navigation_dirty {
            Some(self.navigation_state()?)
        } else {
            None
        };
        let config = if config_dirty {
            Some(self.config_snapshot())
        } else {
            None
        };
        let status = if status_dirty {
            self.last_status.borrow().clone()
        } else {
            None
        };
        // The backend delta is derived unconditionally from the renderer's real
        // lifecycle/actual state, so a backend change is published even when
        // every other dirty flag is clear, and never forces a full query.
        let backend_candidate = self.backend_state();
        let backend = if self.last_backend.borrow().as_ref() != Some(&backend_candidate) {
            Some(backend_candidate.clone())
        } else {
            None
        };
        // Commit only after every fallible derivation above succeeded: on `Err`
        // the dirty markers (and the last published backend) are untouched.
        self.full_dirty.set(false);
        self.navigation_dirty.set(false);
        self.config_dirty.set(false);
        self.status_dirty.set(false);
        *self.last_backend.borrow_mut() = Some(backend_candidate);
        Ok(HostUpdates {
            full,
            navigation,
            config,
            status,
            backend,
        })
    }

    fn set_canvas_metrics(&self, metrics: CanvasMetrics) -> CadResult<()> {
        HostCore::set_canvas_metrics(self, metrics)
    }
    fn render_session(&self) -> RenderSessionHandle {
        self.render.clone()
    }
    fn config_snapshot(&self) -> ConfigSnapshot {
        HostCore::config_snapshot(self)
    }
    fn set_config_json(&self, json: &str) -> CadResult<()> {
        HostCore::set_config_json(self, json)
    }
    fn update_config_json(&self, json: &str) -> CadResult<()> {
        HostCore::update_config_json(self, json)
    }
    fn apply_user_preference_json(&self, json: &str) -> CadResult<()> {
        HostCore::apply_user_preference_json(self, json)
    }
    fn clear_user_preference(&self) -> CadResult<()> {
        // A real clear of the single store, never the trait's refusing default.
        HostCore::clear_user_preference(self)
    }

    fn submit(
        &self,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        // The legacy `submit` is the `Ui` shorthand for an origin-tagged submit.
        self.submit_with_origin_impl(RequestOrigin::Ui, context, request)
    }

    fn pump(&self) -> Result<PumpReport, RequestError> {
        if self.request_engine_busy() {
            return Err(RequestError::Busy);
        }
        // RAII so the re-entrancy flag is cleared even on panic/unwind.
        let _guard = ActiveGuard::new(&self.pump_active);
        self.pump_inner()
    }

    fn poll_completions(&self, limit: usize) -> Vec<RequestCompletion> {
        self.poll_completions_impl(limit)
    }

    fn set_request_waker(&self, waker: Option<Rc<dyn Fn()>>) {
        *self.request_waker.borrow_mut() = waker;
    }

    fn dispatch_with_origin(
        &self,
        origin: RequestOrigin,
        request: EngineRequest,
    ) -> Result<RequestCompletion, RequestError> {
        self.dispatch_with_origin_impl(origin, request)
    }

    fn submit_with_origin(
        &self,
        origin: RequestOrigin,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        self.submit_with_origin_impl(origin, context, request)
    }

    fn subscribe(&self) -> CadResult<SubscriptionId> {
        self.subscribe_impl()
    }

    fn poll_events(&self, id: SubscriptionId, limit: usize) -> CadResult<EventPoll> {
        self.poll_events_impl(id, limit)
    }

    fn unsubscribe(&self, id: SubscriptionId) -> CadResult<()> {
        self.unsubscribe_impl(id)
    }

    fn event_watermark(&self) -> EventWatermark {
        self.event_watermark_impl()
    }
}
