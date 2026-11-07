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
        match option {
            ViewCommand::Save(name) => {
                let name = checked_view_name(name)?;
                let viewport = self
                    .workspace
                    .viewports
                    .get(&command.viewport)
                    .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
                let view = ViewSnapshot::capture(viewport);
                let replaced = session
                    .view_states
                    .save(&name, view, session.active_space.clone());
                Ok(view_outcome(
                    "view.saved",
                    format!(
                        "VIEW SAVE: {} named view `{name}` ({} saved view(s); session state only)",
                        if replaced { "replaced" } else { "created" },
                        session.view_states.len()
                    ),
                ))
            }
            ViewCommand::Restore(name) => self.restore_named_view(session, command, name),
            ViewCommand::List => {
                let names: Vec<&str> = session
                    .view_states
                    .views()
                    .map(|view| view.name.as_str())
                    .collect();
                let message = if names.is_empty() {
                    "VIEW LIST: no named views are saved in this session".to_string()
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
                if !session.view_states.remove(&name) {
                    return Err(unknown_view(&name));
                }
                Ok(view_outcome(
                    "view.deleted",
                    format!(
                        "VIEW DELETE: removed named view `{name}` ({} saved view(s); session state \
                         only)",
                        session.view_states.len()
                    ),
                ))
            }
        }
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
        let target = viewport.camera.target;
        let distance = if entering_from_plan {
            viewport
                .camera
                .enter_3d(bounds, camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?
                .distance()
        } else {
            let d = viewport.camera.distance();
            if d.is_finite() && d > 1e-6 {
                d
            } else {
                1000.0
            }
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
        let Some(saved) = session.view_states.resolve(&name).cloned() else {
            return Err(unknown_view(&name));
        };
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
            ViewSnapshot::capture(viewport) != saved.view || session.active_space != saved.space;
        if changed {
            self.push_view_snapshot(&command.viewport)?;
        }
        session.active_space = saved.space;
        saved.view.restore(self.viewport_mut(command)?);
        Ok(view_outcome(
            "view.restored",
            format!("VIEW RESTORE: applied named view `{name}` (session view state only)"),
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
