//! CHAMFER and FILLET of two picked objects (AutoCAD 2026 contract).
//!
//! ## CHAMFER (`Select first line or [Undo/Polyline/Distance/Angle/Trim/mEthod/Multiple]`)
//!
//! Two picked straight objects. The distances are the session typed channel
//! ([`ToolParameter::ChamferDistanceA`]/`B`); the **method**, **angle** and
//! **trim** switch are the stored [`crate::session_state::CornerModes`]
//! (`CHAMMODE`/`TRIMMODE` plus the angle token), read here so the session stays
//! the single authority:
//!
//! * **Distance** — `A` applies to the first-picked object and `B` to the second,
//!   in selection order. Both distances `0` extend/trim the two objects to their
//!   intersection and create **no** chamfer line.
//! * **Angle** — the distance from the first object plus the angle measured from
//!   that object. The second distance is `d₂ = d·sin α / sin(φ + α)` (`φ` the
//!   angle between the two picked arms); a zero distance **and** a zero angle are
//!   again the intersect-only form. A zero angle with a positive distance
//!   collapses the joining segment and is refused by name (not documented by
//!   AutoCAD).
//! * **Trim / No Trim** — with trim on (default) both objects are trimmed or
//!   extended to the computed points; with trim off they are left untouched and
//!   only the joining LINE is created. Trim-off intersect-only has nothing to
//!   create and is refused rather than reported as an empty success.
//! * **Method / Multiple / Undo** — the method is the stored
//!   [`crate::session_state::CornerModes::chamfer_method`]; `Multiple` and the
//!   in-command `Undo` sub-option need the interactive option loop this build
//!   does not have (each accepted invocation is exactly one transaction / one
//!   undo step, and the global `UNDO` reverses it).
//!
//! Accepted objects are **2D polylines and lines** (a finite RAY/XLINE reaches us
//! as a LINE). Arcs, circles and splines are refused by name — AutoCAD's CHAMFER
//! does not accept them either. A negative distance is not documented and is
//! refused (`chamfer.distance_negative`), never guessed; a negative angle is
//! refused the same way.
//!
//! ## FILLET (`Select first object or [Undo/Polyline/Radius/Trim/Multiple]`)
//!
//! * `R = 0` is the sharp corner: both objects are trimmed/extended to their
//!   intersection and no ARC is created.
//! * `R > 0` creates one tangent ARC and trims both objects to the tangent points
//!   (with trim off the objects are untouched).
//! * **Parallel lines** — the stored radius is ignored and the result is a tangent
//!   arc whose effective radius is **half the perpendicular gap**. Derivation: a
//!   circle tangent to two distinct parallel lines has its centre on the midline,
//!   so its radius is the distance from the midline to either line, i.e.
//!   `gap / 2`. The tangent points are the perpendicular feet of that centre. The
//!   centre's position **along** the lines is our declared rule: the average of
//!   the two pick projections, so the arc lands between the picks.
//! * Accepted objects: **2D polylines, lines, circles, arcs**. A selected circle
//!   is **never trimmed**: the fillet meets it tangentially and the circle stays
//!   whole. An arc is trimmed to the tangent point on its pick side. Ellipses,
//!   elliptical arcs, splines and every pair beyond line↔line / line↔circle /
//!   line↔arc are explicit named `Unsupported` refusals in this build.
//! * A negative radius is not documented and is refused
//!   (`fillet.radius_negative`).
//!
//! Every accepted invocation commits in **one transaction / one undo step**. All
//! refusals are resolved before the first write, so a refusal changes nothing.
//! The pure planners live in the private [`corner_geometry`] submodule (declared
//! with an explicit `#[path]` so the shared `app_drawing/mod.rs` needs no edit).

#[path = "corner_geometry.rs"]
mod corner_geometry;

use super::helpers::corner_tolerance;
use super::*;
use crate::draw_tool::ToolParameter;
use crate::session_state::ChamferMethod;
use corner_geometry::{
    fillet_curve, plan_chamfer_two_line, plan_fillet_two_line, straight_line, ChamferSpec,
    GeometryUpdate, TwoLineCommandKind, TwoLineInput, TwoLinePlan,
};

impl Application {
    /// CHAMFER two picked straight objects.
    ///
    /// The two picks are entity picks resolved at capture time; the distances
    /// are the session typed channel, the method/angle/trim are the stored
    /// [`crate::session_state::CornerModes`]. Read here so the session stays the
    /// single authority.
    pub(crate) fn chamfer_lines(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Chamfer(payload) = &command.payload else {
            return Err(CadError::InvalidInput(
                "ChamferLines needs a Chamfer(TwoLineCommand) payload".into(),
            ));
        };
        let modes = session.drafting_extras.corner_modes();
        let distance_a = tool_number(session, ToolParameter::ChamferDistanceA);
        let spec = match modes.chamfer_method() {
            ChamferMethod::Distance => ChamferSpec::Distances {
                a: distance_a,
                b: tool_number(session, ToolParameter::ChamferDistanceB),
            },
            ChamferMethod::Angle => ChamferSpec::Angle {
                distance: distance_a,
                angle_radians: modes.chamfer_angle_radians(),
            },
        };
        let kind = TwoLineCommandKind::Chamfer {
            spec,
            trim: modes.trim(),
        };
        let plan = self.plan_two_line(command, payload, kind)?;
        self.commit_two_line(command, session, plan, kind)
    }

    /// FILLET two picked objects.
    ///
    /// The radius is the session typed channel ([`ToolParameter::FilletRadius`]);
    /// trim is the stored [`crate::session_state::CornerModes::trim`]. `R = 0` is
    /// the sharp corner.
    pub(crate) fn fillet_lines(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Fillet(payload) = &command.payload else {
            return Err(CadError::InvalidInput(
                "FilletLines needs a Fillet(TwoLineCommand) payload".into(),
            ));
        };
        let modes = session.drafting_extras.corner_modes();
        let kind = TwoLineCommandKind::Fillet {
            radius: tool_number(session, ToolParameter::FilletRadius),
            trim: modes.trim(),
        };
        let plan = self.plan_two_line(command, payload, kind)?;
        self.commit_two_line(command, session, plan, kind)
    }

    /// Resolve both picks and plan the edit **before any write**.
    fn plan_two_line(
        &self,
        command: &Command,
        payload: &TwoLineCommand,
        kind: TwoLineCommandKind,
    ) -> CadResult<TwoLinePlan> {
        let (command_name, _) = kind.labels();
        if payload.first.document != command.document || payload.second.document != command.document
        {
            return Err(CadError::InvalidInput(format!(
                "{command_name} references belong to another document"
            )));
        }
        if payload.first.entity == payload.second.entity {
            return Err(CadError::InvalidInput(format!(
                "{command_name} needs two distinct objects; the same entity was picked twice"
            )));
        }
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let first_entity = document
            .drawing
            .entity(payload.first.entity)
            .ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "{command_name} first object {} does not exist",
                    payload.first.entity.0
                ))
            })?;
        let second_entity = document
            .drawing
            .entity(payload.second.entity)
            .ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "{command_name} second object {} does not exist",
                    payload.second.entity.0
                ))
            })?;
        let tol = corner_tolerance();
        match kind {
            TwoLineCommandKind::Chamfer { spec, trim } => {
                let input = TwoLineInput {
                    first: straight_line(&first_entity.geometry, command_name)?,
                    pick_first: payload.pick_first,
                    second: straight_line(&second_entity.geometry, command_name)?,
                    pick_second: payload.pick_second,
                };
                plan_chamfer_two_line(&input, spec, trim, tol)
            }
            TwoLineCommandKind::Fillet { radius, trim } => plan_fillet_two_line(
                fillet_curve(&first_entity.geometry)?,
                payload.pick_first,
                fillet_curve(&second_entity.geometry)?,
                payload.pick_second,
                radius,
                trim,
                tol,
            ),
        }
        .map_err(CadError::InvalidInput)
    }

    /// Commit a planned two-line edit: one transaction, one undo record.
    fn commit_two_line(
        &mut self,
        command: &Command,
        session: &SessionState,
        plan: TwoLinePlan,
        kind: TwoLineCommandKind,
    ) -> CadResult<CommandOutcome> {
        let (command_name, label) = kind.labels();
        let TwoLinePlan {
            first,
            second,
            inserted,
        } = plan;
        if first.is_none() && second.is_none() && inserted.is_none() {
            return Err(CadError::InvalidInput(format!(
                "{command_name}: the current modes produce no geometry"
            )));
        }
        let document_id = command.document;
        let (first_id, second_id) = match &command.payload {
            CommandPayload::Chamfer(payload) | CommandPayload::Fillet(payload) => {
                (payload.first.entity, payload.second.entity)
            }
            _ => {
                return Err(CadError::InvalidInput(format!(
                    "{command_name} needs its two-object payload"
                )));
            }
        };
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        // The active layer must exist. The corner-only shapes write no new entity,
        // but the ADR lists a missing layer as a refusal for both commands, so the
        // check is unconditional (a nonsense layer is refused, never a silent
        // fallback).
        if document.drawing.layer(session.active_layer).is_none() {
            return Err(CadError::InvalidInput(format!(
                "layer {} does not exist",
                session.active_layer.0
            )));
        }
        let first_before =
            document.drawing.entity(first_id).cloned().ok_or_else(|| {
                CadError::Invariant(format!("{command_name} first object vanished"))
            })?;
        let second_before =
            document.drawing.entity(second_id).cloned().ok_or_else(|| {
                CadError::Invariant(format!("{command_name} second object vanished"))
            })?;
        let mut updates = Vec::new();
        if let Some(update) = first {
            updates.push((first_before.clone(), update));
        }
        if let Some(update) = second {
            updates.push((second_before.clone(), update));
        }
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        let inserted_entity = inserted.map(|geometry| {
            let draw_order = drawing
                .model_space()
                .iter()
                .map(|entity| entity.draw_order)
                .max()
                .map(|max| max.saturating_add(1))
                .unwrap_or(0);
            let entity_id = drawing.allocate_entity_id();
            let entity = DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(entity_id.0),
                    type_key: class_name(&geometry).to_string(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: entity_id,
                layer: session.active_layer,
                space: SpaceId::Model,
                geometry,
                draw_order,
            };
            (entity_id, entity)
        });
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
            for (before, update) in &updates {
                staged.update_entity(apply_update(before, *update, command_name)?)?;
            }
            if let Some((_, entity)) = &inserted_entity {
                staged.insert_entity(entity.clone())?;
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(updates.len() + 1);
        let mut objects = Vec::with_capacity(updates.len() + 1);
        for (before, _) in &updates {
            let stored = drawing.entity(before.id).cloned().ok_or_else(|| {
                CadError::Invariant(format!("{command_name} object vanished after commit"))
            })?;
            patches.push(drawing_patch(before.id, Some(before.clone()), Some(stored)));
            objects.push(ObjectId(before.id.0));
        }
        if let Some((entity_id, entity)) = &inserted_entity {
            patches.push(drawing_patch(*entity_id, None, Some(entity.clone())));
            objects.push(ObjectId(entity_id.0));
        }
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.to_string(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: match kind {
                    TwoLineCommandKind::Chamfer { .. } => "drawing.chamfer".into(),
                    TwoLineCommandKind::Fillet { .. } => "drawing.fillet".into(),
                },
                message: kind.diagnostic(),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Apply one planned update to the before-entity.
fn apply_update(
    before: &DbEntity,
    update: GeometryUpdate,
    command_name: &str,
) -> CadResult<DbEntity> {
    let mut after = before.clone();
    after.geometry = match (update, before.geometry.clone()) {
        (GeometryUpdate::Line { start, end }, SemanticGeometry::Line { .. }) => {
            SemanticGeometry::Line { start, end }
        }
        (
            GeometryUpdate::Arc { start, sweep },
            SemanticGeometry::Arc {
                center,
                normal,
                radius,
                ..
            },
        ) => SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        },
        (update, other) => {
            return Err(CadError::Invariant(format!(
                "{command_name} planned {update:?} for {other:?}"
            )));
        }
    };
    Ok(after)
}

/// The numeric value of a session tool parameter (these parameters are validated
/// as `Number`, so the flag arm is unreachable and falls back to `0.0`).
fn tool_number(session: &SessionState, parameter: ToolParameter) -> f64 {
    session.tool_parameter(parameter).number().unwrap_or(0.0)
}
