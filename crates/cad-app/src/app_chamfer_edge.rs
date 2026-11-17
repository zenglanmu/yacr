//! ADR-0036 P2.0: `CHAMFEREDGE` — one exact edge chamfer on the selected body.
//!
//! The modeler capability already existed as a named refusal, so the command was
//! unreachable. This module makes it reachable through the host's ordinary
//! one-pick capture **without** inventing an edge identity and without a second
//! tolerance:
//!
//! * the target is the **current selection**: exactly one directly addressable
//!   exact `AcDb3dSolid` body in model space. A face sub-element ref (what a body
//!   click produces through `Mesh::face_sources`) collapses to its owning body,
//!   exactly like MASSPROP's routing; a block-instance path is refused;
//! * the pick is a **seed point** — the `PickHit.point` the host resolved through
//!   the one pick funnel — and travels to the modeler as
//!   `SolidEdgeSelector::NearestToPoint`. The kernel owns edge selection (its own
//!   `max(1e-9, 1e-3 · bbox diagonal)` rule), so `cad-app` adds no tolerance;
//! * the two set-back distances are the session typed channel
//!   ([`ToolParameter::ChamferDistanceA`] / `B`), read **at execute time**: the
//!   tool start deliberately does not validate them, because the distance
//!   controls are the only thing that can fix an out-of-range value;
//! * a body whose lift needs non-planar surfaces is **not** pre-judged here: the
//!   kernel answers `solid.lift_requires_planar` and that reason passes through
//!   unchanged.
//!
//! The result replaces the selected row **in place**: same entity id, same
//! `AcDb3dSolid` type key, exactly one revision bump, exactly one undo entry, and
//! every refusal happens before the transaction (no revision, no history). This
//! is the same in-place edit shape documented in
//! [`crate::app_drawing::modify_extra`].
//!
//! # Evidence boundary
//!
//! `cad-app` links no kernel: the contracts here use a fake modeler, and the real
//! `solidcore` chamfer is exercised by `cad-cli-tools/tests/solid_command.rs`.
//! Nothing here is GPU, pixel, carrier or performance evidence.

//! # Shared distance channel (by design)
//!
//! `ChamferMethod`/angle/`trim` are 2D `CHAMFER` concepts; CHAMFEREDGE always
//! consumes **both** `chamferDistanceA` and `chamferDistanceB` as 3D set-backs
//! from the one session channel, so a stale ANGLE-mode `B` becomes a real
//! set-back here.

use super::*;
use cad_db::DbEntity;
use cad_history::{drawing_patch, DrawingUndoRecord};
use cad_kernel_adapter::{
    execute_solid, SolidEdgeSelector, SolidOperation, SolidOutcome, SolidProduct, SolidRequest,
    MAX_CHAMFER_DISTANCE,
};

/// The collapsed, validated target of one `CHAMFEREDGE` run.
///
/// `entity` is a snapshot of the stored row (cloned so the caller can start a
/// transaction afterwards) and `brep` is its exact geometry, the only input the
/// kernel lifts. `body` is the normalised ref (document + entity, no instance
/// path and no sub-element), which is what the capture stores and the payload
/// carries.
#[derive(Debug, Clone)]
pub struct ChamferEdgeTarget {
    pub body: SelectionRef,
    pub entity: DbEntity,
    pub brep: BrepData,
}

/// Resolve the one body `CHAMFEREDGE` operates on.
///
/// Every refusal is stable and command-prefixed, and none of them writes
/// anything:
///
/// * `chamferedge.selection_required` — no entity selected;
/// * `chamferedge.selection_multiple` — more than one distinct body selected
///   (several refs to the same body, e.g. its faces, collapse to one);
/// * `chamferedge.not_addressable` — a block-instance member, or a sub-element
///   of non-body geometry;
/// * `chamferedge.not_a_solid` — a non-body entity, or a body row that is not a
///   solid (sheet/wire/unknown);
/// * `chamferedge.not_exact` — a solid body with no exact B-rep
///   (`BodyExact::Unavailable`);
/// * `chamferedge.not_model_space` — the row lives in paper space;
/// * `chamferedge.selection_changed` — the ref names another document or an
///   entity that no longer exists.
///
/// The collapse rule mirrors `app_massprop_body::route`: a sub-element ref is
/// resolved to its owning body when that body is a `Body3d`, so a face click is
/// a legal way to name a body.
pub fn chamfer_edge_target(
    app: &Application,
    document: DocumentId,
    references: &[SelectionRef],
) -> CadResult<ChamferEdgeTarget> {
    let drawing = app
        .workspace
        .documents
        .get(&document)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
        .drawing
        .as_ref();

    let mut bodies: Vec<&DbEntity> = Vec::with_capacity(references.len());
    for reference in references {
        if reference.document != document {
            return Err(invalid(
                "chamferedge.selection_changed",
                format!(
                    "the selected ref for entity {} belongs to another document",
                    reference.entity.0
                ),
            ));
        }
        if !reference.instance.0.is_empty() {
            return Err(invalid(
                "chamferedge.not_addressable",
                format!(
                    "entity {} is referenced through a block instance, which has no addressable \
                     body identity",
                    reference.entity.0
                ),
            ));
        }
        let entity = drawing.entity(reference.entity).ok_or_else(|| {
            invalid(
                "chamferedge.selection_changed",
                format!(
                    "entity {} does not exist in this drawing any more",
                    reference.entity.0
                ),
            )
        })?;
        if matches!(entity.geometry, SemanticGeometry::Body3d(_)) {
            if !bodies.iter().any(|body| body.id == entity.id) {
                bodies.push(entity);
            }
        } else if reference.sub_element.is_some() {
            // A sub-element of a LINE/CIRCLE/… has no body behind it.
            return Err(invalid(
                "chamferedge.not_addressable",
                format!(
                    "entity {} is not a 3D body, so its sub-element names no edge to chamfer",
                    reference.entity.0
                ),
            ));
        } else {
            return Err(invalid(
                "chamferedge.not_a_solid",
                format!(
                    "entity {} is {}, not an exact 3D solid body",
                    reference.entity.0,
                    describe_entity(entity)
                ),
            ));
        }
    }

    let Some(entity) = bodies.first().copied() else {
        return Err(invalid(
            "chamferedge.selection_required",
            "CHAMFEREDGE cuts an edge of the selected exact solid body; select one first",
        ));
    };
    if bodies.len() > 1 {
        return Err(invalid(
            "chamferedge.selection_multiple",
            format!(
                "CHAMFEREDGE cuts one body at a time, but {} distinct bodies are selected",
                bodies.len()
            ),
        ));
    }

    if entity.object.type_key != crate::app_drawing::solids::SOLID_TYPE_KEY {
        return Err(invalid(
            "chamferedge.not_a_solid",
            format!(
                "entity {} is a {} row; CHAMFEREDGE needs an {} row",
                entity.id.0,
                entity.object.type_key,
                crate::app_drawing::solids::SOLID_TYPE_KEY
            ),
        ));
    }
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        // Unreachable: the loop above only collects `Body3d` rows.
        return Err(CadError::Invariant(format!(
            "chamferedge.body_missing_route: entity {} is not a 3D body",
            entity.id.0
        )));
    };
    if body.kind != BodyKind::Solid {
        return Err(invalid(
            "chamferedge.not_a_solid",
            format!(
                "entity {} is a {:?} body, not a closed solid",
                entity.id.0, body.kind
            ),
        ));
    }
    let BodyExact::Exact(brep) = &body.exact else {
        return Err(invalid(
            "chamferedge.not_exact",
            format!(
                "entity {} carries no exact B-rep, so the kernel has nothing to lift",
                entity.id.0
            ),
        ));
    };
    if entity.space != SpaceId::Model {
        return Err(invalid(
            "chamferedge.not_model_space",
            format!(
                "entity {} is not in model space; CHAMFEREDGE never crosses model/paper space",
                entity.id.0
            ),
        ));
    }

    Ok(ChamferEdgeTarget {
        body: SelectionRef {
            document,
            entity: entity.id,
            instance: InstancePath::default(),
            sub_element: None,
        },
        entity: entity.clone(),
        brep: brep.as_ref().clone(),
    })
}

impl Application {
    /// `CHAMFEREDGE`: cut one edge of the selected exact solid in place.
    ///
    /// Everything that can refuse happens before the transaction, so a failed
    /// run bumps no revision and records no undo entry. The kernel is asked with
    /// the seed point and the two set-backs; its answers
    /// (`solid.lift_requires_planar`, `solid.edge_not_found`,
    /// `solid.chamfer_failed`, `solid.kernel_failed`) pass through unchanged.
    pub(crate) fn chamfer_solid_edge(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (body, seed) =
            match &command.payload {
                CommandPayload::ChamferSolidEdge { body, seed } => (body.clone(), *seed),
                _ => return Err(invalid(
                    "chamferedge.requires_payload",
                    "CHAMFEREDGE takes the host-built ChamferSolidEdge { body, seed } payload; \
                     it is started by its one-pick capture, never by a typed line",
                )),
            };
        if !seed.x.is_finite() || !seed.y.is_finite() || !seed.z.is_finite() {
            return Err(invalid(
                "chamferedge.pick_missed",
                "the captured edge seed point must be finite",
            ));
        }

        // The selection/text payload is re-resolved here: the entity may have
        // been erased or replaced between the pick and Enter.
        let target = chamfer_edge_target(self, command.document, std::slice::from_ref(&body))?;

        // The distances are validated **here**, not at tool start: the strip's
        // two controls are the only way to fix them.
        let distance = validate_chamfer_distance(
            session
                .tool_parameter(ToolParameter::ChamferDistanceA)
                .number()
                .unwrap_or(0.0),
        )?;
        let second = validate_chamfer_distance(
            session
                .tool_parameter(ToolParameter::ChamferDistanceB)
                .number()
                .unwrap_or(0.0),
        )?;

        let Some(modeler) = self.solid_modeler.clone() else {
            return Err(CadError::Unsupported(
                crate::app_drawing::solids::SOLID_MODELER_UNSUPPORTED.into(),
            ));
        };
        // The background boolean owns the kernel mutex: refuse by name rather
        // than block the command thread on it.
        if self.solid_boolean_running() {
            return Err(CadError::Unsupported(
                "chamferedge.kernel_busy: a solid boolean is running; wait for it to settle \
                 before chamfering an edge"
                    .into(),
            ));
        }

        let request = SolidRequest {
            stamp: TaskStamp::new(command.document, session.generation),
            operation: SolidOperation::Chamfer {
                brep: &target.brep,
                selector: SolidEdgeSelector::NearestToPoint(seed),
                distance,
                // A symmetric chamfer is the `None` form; only a genuinely
                // different B distance rides the asymmetric one.
                second: (second != distance).then_some(second),
            },
        };
        let result = execute_solid(modeler.as_ref(), &request, &|| false)?;

        let (brep, kernel_diagnostics) = match result.outcome {
            SolidOutcome::Success(SolidProduct::Body { brep, diagnostics }) => (brep, diagnostics),
            SolidOutcome::Success(SolidProduct::Mass(_)) => {
                return Err(CadError::Invariant(
                    "chamferedge.unexpected_mass: the modeler answered an edge chamfer with mass \
                     properties"
                        .into(),
                ))
            }
            // The kernel owns every capability and selection answer; re-prefixing
            // it here would hide which refusal actually fired.
            SolidOutcome::Unsupported { reason } => {
                return Err(CadError::Unsupported(format!(
                    "{}: {}",
                    reason.code, reason.detail
                )))
            }
            SolidOutcome::Failed { diagnostics } => {
                return Err(CadError::Unsupported(format!(
                    "solid.kernel_failed: {}",
                    crate::app_drawing::solids::kernel_failure_message(&diagnostics)
                )))
            }
        };

        let after_body = crate::app_drawing::solids::solid_body(brep)?;
        let document_id = command.document;
        let mut candidate = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            document.drawing.as_ref().clone()
        };
        let (change_set, before, stored, id) = {
            let drawing = &mut candidate;
            let before = drawing.entity(target.entity.id).cloned().ok_or_else(|| {
                invalid(
                    "chamferedge.selection_changed",
                    format!(
                        "entity {} vanished before the chamfer could be committed",
                        target.entity.id.0
                    ),
                )
            })?;
            let id = before.id;
            let mut after = before.clone();
            // The id, object metadata (including the `AcDb3dSolid` type key),
            // layer, space and draw order stay exactly what they were; only the
            // geometry is replaced.
            after.geometry = SemanticGeometry::Body3d(Box::new(after_body));
            let transaction = crate::app_drawing::next_transaction_id();
            let change_set = {
                let mut staged = drawing.begin_drawing_transaction("chamfer edge", transaction)?;
                staged.update_entity(after)?;
                staged.commit()?
            };
            let stored = drawing.entity(id).cloned().ok_or_else(|| {
                CadError::Invariant(format!(
                    "CHAMFEREDGE: entity {} vanished after commit",
                    id.0
                ))
            })?;
            (change_set, before, stored, id)
        };
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "chamfer edge".into(),
                patches: vec![drawing_patch(id, Some(before), Some(stored))],
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        let mut diagnostics = kernel_diagnostics;
        // Names the two set-backs and the seed, never *which* edge was cut: the
        // seam returns only the product body.
        diagnostics.push(Diagnostic {
            object: Some(ObjectId(id.0)),
            code: "chamferedge.applied".into(),
            message: format!(
                "CHAMFEREDGE: chamfered the edge nearest the seed ({}, {}, {}) with set-back \
                 A={distance}, B={second}",
                seed.x, seed.y, seed.z
            ),
        });
        Ok(CommandOutcome {
            objects: vec![ObjectId(id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// The seam's own valid set-back rule, reused as the command's refusal.
///
/// `SolidRequest::validate` applies the same `(0, MAX_CHAMFER_DISTANCE]` bound;
/// checking it here turns a malformed distance into the typed
/// `chamferedge.distance_invalid` code instead of a generic seam error.
pub(crate) fn validate_chamfer_distance(value: f64) -> CadResult<f64> {
    if value.is_finite() && value > 0.0 && value <= MAX_CHAMFER_DISTANCE {
        return Ok(value);
    }
    Err(invalid(
        "chamferedge.distance_invalid",
        format!(
            "the chamfer set-back must be finite and in (0, {MAX_CHAMFER_DISTANCE}], got {value}"
        ),
    ))
}

/// A plan-level `CHAMFEREDGE` refusal: `InvalidInput` carrying its stable code.
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// A short honest description of a rejected row, reusing the boolean wording.
fn describe_entity(entity: &DbEntity) -> String {
    crate::app_drawing::solids::describe_operand(entity)
}
