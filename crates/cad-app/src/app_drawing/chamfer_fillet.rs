use super::helpers::{corner_tolerance, plan_chamfer, plan_fillet, straight_line, TwoLineInput};
use super::*;

use crate::draw_tool::ToolParameter;

/// Which two-line edit a CHAMFER/FILLET command performs.
enum TwoLineSpec {
    /// CHAMFER with the two session distances (first line, second line).
    Chamfer { distance_a: f64, distance_b: f64 },
    /// FILLET with the session radius.
    Fillet { radius: f64 },
}

/// The two resolved picks (refs + raw pick points) a two-line edit acts on.
struct TwoLinePick<'a> {
    first: &'a SelectionRef,
    second: &'a SelectionRef,
    pick_first: Point3,
    pick_second: Point3,
}

impl Application {
    /// CHAMFER two picked straight lines (ADR 0016 decision C).
    ///
    /// The two picks are entity picks resolved at capture time; the distances are
    /// the session typed channel ([`ToolParameter::ChamferDistanceA`]/`B`), read
    /// here so the session stays the single authority (exactly like POLYGON's side
    /// count). One transaction and one undo step cover the two updates and, when a
    /// distance is non-zero, the one inserted LINE.
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
        let spec = TwoLineSpec::Chamfer {
            distance_a: tool_number(session, ToolParameter::ChamferDistanceA),
            distance_b: tool_number(session, ToolParameter::ChamferDistanceB),
        };
        let pick = TwoLinePick {
            first: &payload.first,
            second: &payload.second,
            pick_first: payload.pick_first,
            pick_second: payload.pick_second,
        };
        self.two_line_edit(command, session, pick, spec)
    }

    /// FILLET two picked straight lines (ADR 0016 decision B).
    ///
    /// Same capture shape as CHAMFER; the radius is the session typed channel
    /// ([`ToolParameter::FilletRadius`], default 0 ⇒ corner only). One transaction
    /// and one undo step cover the two updates and, for `R > 0`, the one inserted
    /// tangent ARC.
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
        let spec = TwoLineSpec::Fillet {
            radius: tool_number(session, ToolParameter::FilletRadius),
        };
        let pick = TwoLinePick {
            first: &payload.first,
            second: &payload.second,
            pick_first: payload.pick_first,
            pick_second: payload.pick_second,
        };
        self.two_line_edit(command, session, pick, spec)
    }

    /// The shared two-line edit: resolve both picks, plan the geometry, then
    /// commit two `update_entity` (plus an optional insert) in one transaction and
    /// one undo record.
    ///
    /// Every input is resolved to plain data before any write, so a refusal
    /// changes nothing (the database transaction is all-or-nothing and no history
    /// is recorded). The new entity (when there is one) goes on the **active
    /// layer** and model space — the ADR records the active-layer choice as ours
    /// because AutoCAD's rule is unverified.
    fn two_line_edit(
        &mut self,
        command: &Command,
        session: &SessionState,
        pick: TwoLinePick<'_>,
        spec: TwoLineSpec,
    ) -> CadResult<CommandOutcome> {
        let TwoLinePick {
            first,
            second,
            pick_first,
            pick_second,
        } = pick;
        let (command_name, label) = match spec {
            TwoLineSpec::Chamfer { .. } => ("CHAMFER", "chamfer lines"),
            TwoLineSpec::Fillet { .. } => ("FILLET", "fillet lines"),
        };
        if first.document != command.document || second.document != command.document {
            return Err(CadError::InvalidInput(format!(
                "{command_name} references belong to another document"
            )));
        }
        if first.entity == second.entity {
            return Err(CadError::InvalidInput(format!(
                "{command_name} needs two distinct lines; the same entity was picked twice"
            )));
        }
        let document_id = command.document;
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
        let first_entity = document
            .drawing
            .entity(first.entity)
            .cloned()
            .ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "{command_name} first line {} does not exist",
                    first.entity.0
                ))
            })?;
        let second_entity = document
            .drawing
            .entity(second.entity)
            .cloned()
            .ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "{command_name} second line {} does not exist",
                    second.entity.0
                ))
            })?;
        let input = TwoLineInput {
            first: straight_line(&first_entity.geometry, command_name)?,
            pick_first,
            second: straight_line(&second_entity.geometry, command_name)?,
            pick_second,
        };
        let tol = corner_tolerance();
        let edit = match spec {
            TwoLineSpec::Chamfer {
                distance_a,
                distance_b,
            } => plan_chamfer(&input, distance_a, distance_b, tol),
            TwoLineSpec::Fillet { radius } => plan_fillet(&input, radius, tol),
        }
        .map_err(CadError::InvalidInput)?;

        let first_id = first_entity.id;
        let second_id = second_entity.id;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        let mut after_first = first_entity.clone();
        after_first.geometry = SemanticGeometry::Line {
            start: edit.first.0,
            end: edit.first.1,
        };
        let mut after_second = second_entity.clone();
        after_second.geometry = SemanticGeometry::Line {
            start: edit.second.0,
            end: edit.second.1,
        };
        let inserted = edit.inserted.map(|geometry| {
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
            staged.update_entity(after_first.clone())?;
            staged.update_entity(after_second.clone())?;
            if let Some((_, entity)) = &inserted {
                staged.insert_entity(entity.clone())?;
            }
            staged.commit()?
        };

        // Read both committed entities back so each undo patch's `after` is the
        // exact stored value, never a re-derivation.
        let stored_first = drawing.entity(first_id).cloned().ok_or_else(|| {
            CadError::Invariant(format!("{command_name} first line vanished after commit"))
        })?;
        let stored_second = drawing.entity(second_id).cloned().ok_or_else(|| {
            CadError::Invariant(format!("{command_name} second line vanished after commit"))
        })?;
        let mut patches = vec![
            drawing_patch(first_id, Some(first_entity), Some(stored_first)),
            drawing_patch(second_id, Some(second_entity), Some(stored_second)),
        ];
        let mut objects = vec![ObjectId(first_id.0), ObjectId(second_id.0)];
        if let Some((entity_id, entity)) = &inserted {
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
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: match spec {
                    TwoLineSpec::Chamfer { .. } => "drawing.chamfer".into(),
                    TwoLineSpec::Fillet { .. } => "drawing.fillet".into(),
                },
                // Locale-neutral technical diagnostic (never a localized UI label;
                // the ADR fence forbids new CJK literals on the Rust side).
                message: match spec {
                    TwoLineSpec::Chamfer { .. } => {
                        "CHAMFER: both lines trimmed/extended (a new LINE joins them unless A=B=0)"
                            .to_string()
                    }
                    TwoLineSpec::Fillet { .. } => {
                        "FILLET: both lines trimmed/extended (a tangent ARC is inserted unless R=0)"
                            .to_string()
                    }
                },
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The numeric value of a session tool parameter (these parameters are validated
/// as `Number`, so the flag arm is unreachable and falls back to `0.0`).
fn tool_number(session: &SessionState, parameter: ToolParameter) -> f64 {
    session.tool_parameter(parameter).number().unwrap_or(0.0)
}
