//! `PRESSPULL`: move one face of the selected exact solid in place.
//!
//! The kernel operation is [`SolidOperation::OffsetFace`] (added 2026-11-25);
//! this module makes it reachable through the same one-pick capture
//! `CHAMFEREDGE` uses:
//!
//! * the target is the **current selection**: exactly one directly addressable
//!   exact `AcDb3dSolid` body in model space, with a face sub-element ref
//!   collapsing to its owning body (the shared
//!   [`crate::app_chamfer_edge::press_pull_target`] rules, `presspull.*` codes);
//! * the pick is the pixel pick's `PickHit.point` — a **face seed**, never a
//!   work-plane projection; the kernel resolves the nearest trimmed face from it
//!   with its own tolerance;
//! * the distance is the session typed channel
//!   ([`crate::ToolParameter::PressPullDistance`]), read at execute time:
//!   positive moves the face outward (the body grows), negative inward (the body
//!   shrinks). Zero is unset and refused by name; the magnitude is bounded by the
//!   seam's own sanity limit.
//!
//! The result replaces the selected row **in place**: same entity id, same
//! `AcDb3dSolid` type key, exactly one revision bump, exactly one undo entry, and
//! every refusal happens before the transaction. Kernel answers
//! (`solid.face_not_found`, `solid.offset_face_failed`, `solid.lift_*`) pass
//! through unchanged.
//!
//! # Evidence boundary
//!
//! `cad-app` links no kernel: the contracts here use a fake modeler, and the real
//! `solidcore` face move is exercised by `cad-kernel-solidcore/tests/offset_face.rs`.
//! Nothing here is GPU, pixel, carrier or performance evidence.

use super::*;
use cad_history::{drawing_patch, DrawingUndoRecord};
use cad_kernel_adapter::{
    execute_solid, SolidFaceSelector, SolidOperation, SolidOutcome, SolidProduct, SolidRequest,
    MAX_OFFSET_FACE_DISTANCE,
};

impl Application {
    /// `PRESSPULL`: move one face of the selected exact solid in place.
    ///
    /// Everything that can refuse happens before the transaction, so a failed
    /// run bumps no revision and records no undo entry.
    pub(crate) fn press_pull_face(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (body, seed) =
            match &command.payload {
                CommandPayload::PressPullFace { body, seed } => (body.clone(), *seed),
                _ => return Err(invalid(
                    "presspull.requires_payload",
                    "PRESSPULL takes the host-built PressPullFace { body, seed } payload; it is \
                     started by its one-pick capture, never by a typed line",
                )),
            };
        if !seed.x.is_finite() || !seed.y.is_finite() || !seed.z.is_finite() {
            return Err(invalid(
                "presspull.pick_missed",
                "the captured face seed point must be finite",
            ));
        }

        // The body is re-resolved here: it may have been erased or replaced
        // between the pick and Enter.
        let target = press_pull_target(self, command.document, std::slice::from_ref(&body))?;

        // The distance is validated **here**, not at tool start: the Measurements
        // control is the only way to fix an unset/out-of-range value.
        let distance = validate_press_pull_distance(
            session
                .tool_parameter(ToolParameter::PressPullDistance)
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
                "presspull.kernel_busy: a solid boolean is running; wait for it to settle before \
                 moving a face"
                    .into(),
            ));
        }

        let request = SolidRequest {
            stamp: TaskStamp::new(command.document, session.generation),
            operation: SolidOperation::OffsetFace {
                brep: &target.brep,
                selector: SolidFaceSelector::NearestToPoint(seed),
                distance,
            },
        };
        let result = execute_solid(modeler.as_ref(), &request, &|| false)?;

        let (brep, kernel_diagnostics) = match result.outcome {
            SolidOutcome::Success(SolidProduct::Body { brep, diagnostics }) => (brep, diagnostics),
            SolidOutcome::Success(SolidProduct::Mass(_)) => {
                return Err(CadError::Invariant(
                    "presspull.unexpected_mass: the modeler answered a face move with mass \
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

        let after_body =
            crate::app_drawing::solids::solid_body(brep, self.solid_tessellator.as_deref())?;
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
                    "presspull.selection_changed",
                    format!(
                        "entity {} vanished before the face move could be committed",
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
                let mut staged = drawing.begin_drawing_transaction("press/pull", transaction)?;
                staged.update_entity(after)?;
                staged.commit()?
            };
            let stored = drawing.entity(id).cloned().ok_or_else(|| {
                CadError::Invariant(format!("PRESSPULL: entity {} vanished after commit", id.0))
            })?;
            (change_set, before, stored, id)
        };
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "press/pull".into(),
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
        // Names the signed distance and the seed, never *which* face moved: the
        // seam returns only the product body.
        diagnostics.push(Diagnostic {
            object: Some(ObjectId(id.0)),
            code: "presspull.applied".into(),
            message: format!(
                "PRESSPULL: moved the face nearest the seed ({}, {}, {}) by {distance} along its \
                 outward normal",
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

/// The command's distance rule, mirroring [`MAX_OFFSET_FACE_DISTANCE`].
///
/// Either sign is meaningful (outward/inward); zero is the unset sentinel and is
/// refused here by name instead of reaching the seam's generic validator.
pub(crate) fn validate_press_pull_distance(value: f64) -> CadResult<f64> {
    if value.is_finite() && value != 0.0 && value.abs() <= MAX_OFFSET_FACE_DISTANCE {
        return Ok(value);
    }
    Err(invalid(
        "presspull.distance_invalid",
        format!(
            "the press/pull distance must be finite, non-zero and at most \
             {MAX_OFFSET_FACE_DISTANCE} in magnitude, got {value}"
        ),
    ))
}

/// A plan-level `PRESSPULL` refusal: `InvalidInput` carrying its stable code.
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}
