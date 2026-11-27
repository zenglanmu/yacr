//! The 2026-11-07 view/space batch: `REDRAW`, `REGEN`, `VIEW`, `VIEWGO`,
//! `VIEWBACK`, `VIEWFORWARD` and `VPOINT`.
//!
//! Every function here mutates **session view state only**: the viewport camera,
//! the 2D/3D mode, the plan work plane, the active drawing space, the bounded
//! `ZOOM PREVIOUS` history or the session's named-view store. None of them
//! writes the drawing, bumps its revision, creates a `cad-history` entry or opens
//! a transaction.
//!
//! `REDRAW`/`REGEN` are explicit refresh stamps rather than camera changes: the
//! former advances [`Viewport::refresh_generation`] and the session generation,
//! the latter advances [`Viewport::regen_generation`] and
//! [`SessionState::regen_generation`] so a host can invalidate its derived scene
//! cache. Neither pushes a `ZOOM PREVIOUS` snapshot, because the view does not
//! change.

use super::*;

use cad_domain::Point3;

use crate::camera::{add, dot3, normalize3, scale3};
use crate::{CommandPayload, VPointInput, ViewCommand, ViewSnapshot};

impl Application {
    /// Apply `REDRAW` (one viewport) or `REDRAWALL` (every viewport of the
    /// document).
    pub(crate) fn redraw(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        all: bool,
    ) -> CadResult<CommandOutcome> {
        payload_free(command, if all { "REDRAWALL" } else { "REDRAW" })?;
        let mut refreshed = 0usize;
        if all {
            for viewport in self
                .workspace
                .viewports
                .values_mut()
                .filter(|viewport| viewport.document == command.document)
            {
                viewport.refresh_generation = viewport.refresh_generation.saturating_add(1);
                refreshed += 1;
            }
        } else {
            let viewport = self.viewport_mut(command)?;
            viewport.refresh_generation = viewport.refresh_generation.saturating_add(1);
            refreshed = 1;
        }
        // The session refresh stamp is what a host watches to re-read derived
        // status; the camera, the database and the view history are untouched.
        session.generation = session.generation.saturating_add(1);
        let (code, message) = if all {
            (
                "redraw.all",
                format!(
                    "REDRAWALL: refreshed the display of {refreshed} viewport(s); the camera, the \
                     drawing and the view history are unchanged"
                ),
            )
        } else {
            (
                "redraw",
                "REDRAW: refreshed the current viewport's display; the camera, the drawing and the \
                 view history are unchanged"
                    .to_string(),
            )
        };
        Ok(view_outcome(code, message))
    }

    /// Apply `REGEN` (one viewport) or `REGENALL` (every viewport of the
    /// document).
    pub(crate) fn regen(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        all: bool,
    ) -> CadResult<CommandOutcome> {
        payload_free(command, if all { "REGENALL" } else { "REGEN" })?;
        let mut regenerated = 0usize;
        if all {
            for viewport in self
                .workspace
                .viewports
                .values_mut()
                .filter(|viewport| viewport.document == command.document)
            {
                viewport.regen_generation = viewport.regen_generation.saturating_add(1);
                regenerated += 1;
            }
        } else {
            let viewport = self.viewport_mut(command)?;
            viewport.regen_generation = viewport.regen_generation.saturating_add(1);
            regenerated = 1;
        }
        session.regen_generation = session.regen_generation.saturating_add(1);
        let (code, message) = if all {
            (
                "regen.all",
                format!(
                    "REGENALL: advanced the regeneration stamp of {regenerated} viewport(s); the \
                     drawing was not written"
                ),
            )
        } else {
            (
                "regen",
                "REGEN: advanced the current viewport's regeneration stamp; the drawing was not \
                 written"
                    .to_string(),
            )
        };
        Ok(view_outcome(code, message))
    }

    /// `VIEW SAVE|RESTORE|LIST|DELETE`; `-VIEW` reaches this same arm.
    pub(crate) fn view_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::View(option) = &command.payload else {
            return Err(CadError::InvalidInput(
                "VIEW needs SAVE <name>, RESTORE <name>, LIST or DELETE <name>".into(),
            ));
        };
        // VIEW itself is Viewer-legal for RESTORE/LIST (session view state), but
        // SAVE/DELETE write `tables/views.json`. The whole selector cannot be
        // Work-only without losing the read forms, so the write options refuse
        // explicitly in Viewer instead of performing a hidden document write.
        if session.app_mode() == AppMode::Viewer
            && matches!(
                option,
                ViewCommand::Save(_) | ViewCommand::Delete(_) | ViewCommand::Window { .. }
            )
        {
            return Err(CadError::PermissionDenied);
        }
        match option {
            ViewCommand::Save(name) => {
                let name = checked_view_name(name)?;
                let viewport = self
                    .workspace
                    .viewports
                    .get(&command.viewport)
                    .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
                let view = ViewSnapshot::capture(viewport);
                self.persist_named_view(
                    session,
                    command,
                    &name,
                    view,
                    "view.saved",
                    &format!("VIEW SAVE {name}"),
                )
            }
            ViewCommand::Window {
                name,
                first,
                second,
            } => {
                let name = checked_view_name(name)?;
                let view = self.window_view_snapshot(command, *first, *second)?;
                self.persist_named_view(
                    session,
                    command,
                    &name,
                    view,
                    "view.saved_window",
                    &format!("NEWVIEW WINDOW {name}"),
                )
            }
            ViewCommand::Restore(name) => self.restore_named_view(session, command, name),
            ViewCommand::List => {
                let drawing = self.drawing(command.document)?;
                let names: Vec<&str> = drawing
                    .named_views()
                    .views()
                    .map(|view| view.name.as_str())
                    .collect();
                let message = if names.is_empty() {
                    "VIEW LIST: no named views are saved in this drawing".to_string()
                } else {
                    format!(
                        "VIEW LIST: {} named view(s): {}",
                        names.len(),
                        names.join(", ")
                    )
                };
                Ok(view_outcome("view.list", message))
            }
            ViewCommand::Delete(name) => {
                let name = checked_view_name(name)?;
                let planned = self
                    .drawing(command.document)?
                    .plan_delete_named_view(&name)?;
                self.write_named_view(
                    command.document,
                    planned.patch,
                    &format!("VIEW DELETE {name}"),
                )?;
                let count = self.drawing(command.document)?.named_views().len();
                Ok(view_outcome(
                    "view.deleted",
                    format!(
                        "VIEW DELETE: removed named view `{name}` ({count} saved view(s) in the \
                         drawing)"
                    ),
                ))
            }
        }
    }

    /// Build the view a `NEWVIEW WINDOW` rectangle defines.
    ///
    /// The rectangle is two world-space corners on the current 2D plan. The
    /// existing `ZOOM WINDOW` mapping (`framing_rect` + `plan_camera`, margin
    /// [`WINDOW_MARGIN`]) is reused so there is no second framing rule; the
    /// current camera is **not** moved, because NEWVIEW defines a view.
    fn window_view_snapshot(
        &self,
        command: &Command,
        first: Point3,
        second: Point3,
    ) -> CadResult<ViewSnapshot> {
        for (label, point) in [("first", first), ("second", second)] {
            if !camera::is_finite_point(point) {
                return Err(CadError::InvalidInput(format!(
                    "newview.window.degenerate_rect: the {label} corner is not finite"
                )));
            }
        }
        let viewport = self
            .workspace
            .viewports
            .get(&command.viewport)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        if viewport.view_mode.kind() != ProjectionKind::TwoD
            || !viewport.camera.projection.is_orthographic()
        {
            return Err(CadError::InvalidInput(
                "newview.window.requires_2d: a rectangular view window is only defined in the 2D \
                 plan view; switch back to 2D first"
                    .into(),
            ));
        }
        let min = Point3 {
            x: first.x.min(second.x),
            y: first.y.min(second.y),
            z: first.z,
        };
        let max = Point3 {
            x: first.x.max(second.x),
            y: first.y.max(second.y),
            z: first.z,
        };
        if max.x <= min.x || max.y <= min.y {
            return Err(CadError::InvalidInput(
                "newview.window.degenerate_rect: the view window must have an extent on both axes"
                    .into(),
            ));
        }
        let (centre, scale) = Self::framing_rect(min, max, viewport.logical_size, WINDOW_MARGIN);
        let framed = Self::plan_camera(centre, scale, viewport.camera.eye.z);
        Ok(ViewSnapshot {
            camera: framed,
            view_mode: ViewMode2d3d::TwoD { saved: framed },
            work_plane: viewport.work_plane,
        })
    }

    /// Plan and commit one named-view row, then report the result.
    fn persist_named_view(
        &mut self,
        session: &SessionState,
        command: &Command,
        name: &str,
        view: ViewSnapshot,
        code: &str,
        label: &str,
    ) -> CadResult<CommandOutcome> {
        let named = crate::view_persist::named_view_from_snapshot(
            name.to_string(),
            &view,
            session.active_space.clone(),
        );
        let planning = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .plan_save_named_view(named)?;
        let Some(planned) = planning else {
            return Ok(view_outcome(
                "view.unchanged",
                format!("`{name}` already stores this exact view; nothing was written"),
            ));
        };
        let replaced = planned.replaced;
        self.write_named_view(command.document, planned.patch, label)?;
        let count = self.drawing(command.document)?.named_views().len();
        Ok(view_outcome(
            code,
            format!(
                "{} named view `{name}` ({count} saved view(s) in the drawing)",
                if replaced { "replaced" } else { "created" },
            ),
        ))
    }

    /// Commit one named-view table patch and record it as one undo step.
    ///
    /// The table write is one revision tied to the same transaction as the
    /// history record. If the record is refused (a pending undo step), the table
    /// write is reverted so a failed command never leaves a half-applied edit.
    fn write_named_view(
        &mut self,
        document: DocumentId,
        patch: cad_db::ViewPatch,
        label: &str,
    ) -> CadResult<()> {
        let transaction = crate::app_drawing::next_transaction_id();
        let changes = {
            let document = self
                .workspace
                .documents
                .get_mut(&document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            Arc::make_mut(&mut document.drawing).apply_named_view_patches(
                std::slice::from_ref(&patch),
                transaction,
                label,
            )?
        };
        debug_assert!(changes.scope.tables.contains(&cad_db::TableKind::Views));
        let record =
            cad_history::DrawingUndoRecord::entity_only(transaction, label.to_string(), Vec::new());
        if let Err(error) = self
            .history
            .entry(document)
            .or_default()
            .record_drawing_with_views(record, vec![patch.clone()])
        {
            // Put the row back: the write above already succeeded, and a refused
            // history record must not leave a table edit the user cannot undo.
            let inverse = cad_db::ViewPatch {
                name: patch.name,
                before: patch.after,
                after: patch.before,
            };
            let document = self
                .workspace
                .documents
                .get_mut(&document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            Arc::make_mut(&mut document.drawing).apply_named_view_patches(
                std::slice::from_ref(&inverse),
                crate::app_drawing::next_transaction_id(),
                "VIEW rollback",
            )?;
            return Err(error);
        }
        Ok(())
    }

    /// `VIEWGO <name>`: the same restore path as `VIEW RESTORE`.
    pub(crate) fn view_go(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ViewGo(name) = &command.payload else {
            return Err(CadError::InvalidInput(
                "VIEWGO needs one saved view name".into(),
            ));
        };
        let name = checked_view_name(name)?;
        self.restore_named_view(session, command, &name)
    }

    /// `VIEWBACK`: pop the per-viewport `ZOOM PREVIOUS` stack.
    pub(crate) fn view_back(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        payload_free(command, "VIEWBACK")?;
        let viewport = self.viewport_mut(command)?;
        let current = ViewSnapshot::capture(viewport);
        let Some(snapshot) = viewport.zoom_history.pop() else {
            return Ok(view_outcome(
                "view.back.empty",
                "VIEWBACK: there is no previous view in this viewport; nothing changed".to_string(),
            ));
        };
        viewport.zoom_history.push_forward(current);
        snapshot.restore(viewport);
        Ok(view_outcome(
            "view.back",
            "VIEWBACK: restored the previous view (session view state only)".to_string(),
        ))
    }

    /// `VIEWFORWARD`: pop the bounded forward stack.
    pub(crate) fn view_forward(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        payload_free(command, "VIEWFORWARD")?;
        let viewport = self.viewport_mut(command)?;
        let current = ViewSnapshot::capture(viewport);
        let Some(snapshot) = viewport.zoom_history.pop_forward() else {
            return Ok(view_outcome(
                "view.forward.empty",
                "VIEWFORWARD: there is no forward view in this viewport; nothing changed"
                    .to_string(),
            ));
        };
        viewport.zoom_history.push_back_preserving_forward(current);
        snapshot.restore(viewport);
        Ok(view_outcome(
            "view.forward",
            "VIEWFORWARD: restored the forward view (session view state only)".to_string(),
        ))
    }

    /// `VPOINT`: set the camera viewing direction without touching the model.
    pub(crate) fn vpoint(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::VPoint(input) = command.payload else {
            return Err(CadError::InvalidInput(
                "VPOINT needs <x> <y> <z> or TOP, BOTTOM, FRONT, BACK, LEFT, RIGHT".into(),
            ));
        };
        match input {
            VPointInput::Standard(view) => {
                let plan_noop = view.is_plan() && self.view_state_is_2d(&command.viewport);
                if !plan_noop {
                    self.push_view_snapshot(&command.viewport)?;
                }
                self.apply_standard_view(session, command, view)?;
                Ok(view_outcome(
                    "vpoint.standard",
                    format!(
                        "VPOINT: applied the {} standard view (the model is unchanged)",
                        view.key()
                    ),
                ))
            }
            VPointInput::Vector(vector) => self.vpoint_vector(command, vector),
        }
    }

    /// Apply a `VPOINT x y z` direction: a 3D observation from that eye vector.
    fn vpoint_vector(&mut self, command: &Command, vector: Point3) -> CadResult<CommandOutcome> {
        if !crate::camera::is_finite_point(vector) {
            return Err(CadError::InvalidInput(
                "vpoint.invalid_vector: VPOINT needs three finite numbers".into(),
            ));
        }
        let Some(direction) = normalize3(vector) else {
            return Err(CadError::InvalidInput(
                "vpoint.invalid_vector: VPOINT refuses the zero vector; give a non-zero viewing \
                 direction"
                    .into(),
            ));
        };
        let bounds = self.document_bounds(command.document);
        let viewport = self.viewport_mut(command)?;
        let entering_from_plan = viewport.view_mode.kind() == ProjectionKind::TwoD;
        // Entering 3D from the plan takes the fitted target as well as the
        // distance: the plan target's `z = 0` is not the model's centre, so a
        // VPOINT orbit would otherwise turn about a point below/above the solid.
        // An existing 3D observation keeps its own target and distance.
        let (target, distance) = if entering_from_plan {
            let fitted = viewport
                .camera
                .enter_3d(bounds, camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?;
            (fitted.target, fitted.distance())
        } else {
            let d = viewport.camera.distance();
            let distance = if d.is_finite() && d > 1e-6 { d } else { 1000.0 };
            (viewport.camera.target, distance)
        };
        // A top/bottom direction needs a non-Z up hint; every other direction
        // uses world +Z. `ViewBasis` would correct a parallel hint anyway, but
        // choosing here keeps the resulting frame stable and predictable.
        let up = if dot3(
            direction,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        )
        .abs()
            > 1.0 - 1e-9
        {
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            }
        } else {
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            }
        };
        let projection = if viewport.camera.projection.is_orthographic() {
            Projection::perspective(camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?
        } else {
            viewport.camera.projection
        };
        let camera = Camera {
            eye: add(target, scale3(direction, distance)),
            target,
            up,
            projection,
        };
        camera.validate()?;
        viewport.push_view_snapshot();
        viewport.camera = camera;
        let saved = viewport.view_mode.saved_2d_camera();
        viewport.view_mode = ViewMode2d3d::ThreeD { saved_2d: saved };
        Ok(view_outcome(
            "vpoint.vector",
            format!(
                "VPOINT: set the viewing direction to ({:.4}, {:.4}, {:.4}); the model is unchanged",
                direction.x, direction.y, direction.z
            ),
        ))
    }

    /// Restore one saved view by name (the shared `VIEW RESTORE`/`VIEWGO` path).
    fn restore_named_view(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        name: &str,
    ) -> CadResult<CommandOutcome> {
        let name = checked_view_name(name)?;
        let Some(saved) = self
            .drawing(command.document)?
            .named_views()
            .resolve(&name)
            .cloned()
        else {
            return Err(unknown_view(&name));
        };
        let snapshot = ViewSnapshot::from_named_view(&saved)?;
        // A saved paper space must still be drawable in the current drawing.
        let drawing = &self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing;
        crate::space::validate_space_id(drawing, &saved.space)?;
        let viewport = self
            .workspace
            .viewports
            .get(&command.viewport)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let changed =
            ViewSnapshot::capture(viewport) != snapshot || session.active_space != saved.space;
        if changed {
            self.push_view_snapshot(&command.viewport)?;
        }
        session.active_space = saved.space;
        snapshot.restore(self.viewport_mut(command)?);
        Ok(view_outcome(
            "view.restored",
            format!(
                "VIEW RESTORE: applied named view `{name}` from the drawing (camera, mode, work \
                 plane and space)"
            ),
        ))
    }
}

/// Reject a caller-supplied payload on a payload-free view command.
fn payload_free(command: &Command, name: &str) -> CadResult<()> {
    if matches!(command.payload, CommandPayload::None) {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!("{name} needs no payload")))
    }
}

/// A trimmed, non-empty name, or the stable named refusal.
fn checked_view_name(name: &str) -> CadResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CadError::InvalidInput(
            "view.name_required: a named view needs a non-empty name".into(),
        ));
    }
    Ok(name.to_string())
}

fn unknown_view(name: &str) -> CadError {
    CadError::InvalidInput(format!(
        "view.unknown: there is no saved view named `{name}` in this session"
    ))
}

fn view_outcome(code: &str, message: String) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: code.into(),
            message,
        }],
        measurement: None,
        point_report: None,
    }
}

impl Application {
    /// `CAMERA`: place a perspective viewpoint from the captured eye and target.
    ///
    /// This is SketchUp's Position Camera: the eye is the captured first point,
    /// the second point is what it looks at. Up is world `+Z` (world `+Y` for a
    /// vertical look, the same rule VPOINT uses), the field of view is the
    /// viewport's current perspective lens (or the documented default when the
    /// view is still orthographic), and the 2D camera is saved so switching back
    /// to plan restores it.
    ///
    /// View-only: no document write, no revision, no undo step (a
    /// `ZOOM PREVIOUS` snapshot is recorded, exactly like VPOINT/RESETVIEW).
    /// Refusals: `camera.invalid_point` (non-finite input),
    /// `camera.eye_target_coincident` (the two points coincide, so there is no
    /// viewing direction) and `camera.invalid_frame` (the constructed camera is
    /// degenerate). Two-point perspective and walk mode are refused at the typed
    /// line (`camera.two_point_not_implemented` / `camera.walk_not_implemented`)
    /// rather than silently placed here.
    pub(crate) fn position_camera(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::PositionCamera { eye, target } = command.payload else {
            return Err(CadError::InvalidInput(
                "camera.payload: CAMERA needs the captured eye and target points".into(),
            ));
        };
        if !crate::camera::is_finite_point(eye) || !crate::camera::is_finite_point(target) {
            return Err(CadError::InvalidInput(
                "camera.invalid_point: CAMERA needs two finite points".into(),
            ));
        }
        let direction = Point3 {
            x: target.x - eye.x,
            y: target.y - eye.y,
            z: target.z - eye.z,
        };
        let Some(direction) = normalize3(direction) else {
            return Err(CadError::InvalidInput(
                "camera.eye_target_coincident: the eye and the target must be different points"
                    .into(),
            ));
        };
        let up = if dot3(
            direction,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        )
        .abs()
            > 1.0 - 1e-9
        {
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            }
        } else {
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            }
        };
        let viewport = self.viewport_mut(command)?;
        let projection = if viewport.camera.projection.is_orthographic() {
            Projection::perspective(camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?
        } else {
            viewport.camera.projection
        };
        let camera = Camera {
            eye,
            target,
            up,
            projection,
        };
        camera.validate().map_err(|error| {
            CadError::InvalidInput(format!(
                "camera.invalid_frame: the captured points cannot define a camera frame ({error})"
            ))
        })?;
        viewport.push_view_snapshot();
        viewport.camera = camera;
        let saved = viewport.view_mode.saved_2d_camera();
        viewport.view_mode = ViewMode2d3d::ThreeD { saved_2d: saved };
        Ok(view_outcome(
            "camera.positioned",
            format!(
                "CAMERA: eye ({:.4}, {:.4}, {:.4}) looking at ({:.4}, {:.4}, {:.4}); the model is \
                 unchanged",
                eye.x, eye.y, eye.z, target.x, target.y, target.z
            ),
        ))
    }
}
