//! Host-side commit of the background solid boolean (ADR-0036 phase C2).
//!
//! The command (`app_drawing::solids::boolean_solids`) validates the selection,
//! snapshots the operands and starts a worker. This module is the other half:
//! [`HostController::poll_solid_boolean`] takes the terminal body and publishes
//! it, but only when the result is still current:
//!
//! * the manager stamp (document + generation) must match, so a superseded job
//!   can never publish;
//! * every operand must still exist **with the revision it had at start**, so an
//!   edit made while the worker ran discards the result instead of deleting the
//!   user's newer geometry;
//! * the deletion and the insert happen in one transaction with one undo record,
//!   including each operand's captured display state, so undo restores the whole
//!   group exactly.
//!
//! Nothing here runs on the worker: the worker returns a `Body3d`, the host owns
//! the database (ADR-0036 D1 / the `cad-db` authority rule).

use super::*;
use crate::app_drawing::next_transaction_id;
use crate::tasks::{SolidBooleanPlan, SolidBooleanTake};
use cad_db::{DbEntity, DbObject, EntityDisplayState, EntityRenderAttributes};
use cad_history::{drawing_patch_deleted, DrawingPatch, DrawingUndoRecord};

/// The outcome of polling the background solid boolean once.
///
/// `Done` carries the committed object id and the kernel's non-fatal notes;
/// `Stale` is the explicit "the result was discarded, nothing changed" answer
/// (the requested variant set calls this a stale result — it is reported
/// separately from `Cancelled` so a UI never claims the user cancelled).
#[derive(Debug)]
pub enum SolidBooleanPoll {
    /// No boolean job is current.
    Idle,
    /// The worker is still running.
    Running,
    /// The body was committed in one transaction; the operands are gone.
    Done {
        object: ObjectId,
        diagnostics: Vec<Diagnostic>,
    },
    /// The host cancelled the job; nothing was published.
    Cancelled,
    /// The result was discarded as stale (superseded, document replaced, or an
    /// operand changed between start and poll); nothing was published.
    Stale,
    /// The boolean genuinely failed; the drawing is untouched.
    Failed { error: CadError },
}

impl HostController {
    /// Poll the background boolean and publish a current, non-stale body.
    ///
    /// The drawing is never modified unless `Done` is returned.
    pub fn poll_solid_boolean(&mut self) -> SolidBooleanPoll {
        match self.application.poll_solid_boolean_take() {
            SolidBooleanTake::Idle => SolidBooleanPoll::Idle,
            SolidBooleanTake::Running => SolidBooleanPoll::Running,
            SolidBooleanTake::Cancelled => SolidBooleanPoll::Cancelled,
            SolidBooleanTake::Stale => SolidBooleanPoll::Stale,
            SolidBooleanTake::Failed(error) => SolidBooleanPoll::Failed { error },
            SolidBooleanTake::Done {
                plan,
                body,
                diagnostics,
            } => match self.commit_solid_boolean(&plan, *body) {
                Ok(object) => SolidBooleanPoll::Done {
                    object,
                    diagnostics,
                },
                Err(CadError::StaleResult) => SolidBooleanPoll::Stale,
                Err(error) => SolidBooleanPoll::Failed { error },
            },
        }
    }

    /// Ask the running boolean to stop. The job keeps its slot so the next poll
    /// reports [`SolidBooleanPoll::Cancelled`]; nothing is published.
    pub fn cancel_solid_boolean(&mut self) {
        self.application.cancel_solid_boolean();
    }

    /// Commit the folded body in one transaction and one undo step.
    ///
    /// Re-validates the plan against the **current** drawing (document id,
    /// operand existence and revision) before staging anything; a mismatch is
    /// `CadError::StaleResult` and leaves the drawing untouched.
    fn commit_solid_boolean(
        &mut self,
        plan: &SolidBooleanPlan,
        body: Body3d,
    ) -> CadResult<ObjectId> {
        if plan.document != self.document_id {
            return Err(CadError::StaleResult);
        }
        // Last representability gate before any staging: the worker already
        // refuses a multi-shell/void result, and re-checking here keeps the
        // commit seam honest even if a future caller hands it a body directly.
        let exact = body.exact_brep().ok_or_else(|| {
            CadError::Invariant(
                "solid.boolean_result_not_exact: the boolean worker returned a body without \
                 exact geometry"
                    .into(),
            )
        })?;
        crate::app_drawing::solids::validate_boolean_result(exact)?;
        let (candidate, operands, display_states, change_set, new_id, entity) = {
            let document = self
                .application
                .workspace
                .documents
                .get(&self.document_id)
                .ok_or(CadError::StaleResult)?;
            let mut candidate = document.drawing.as_ref().clone();
            let mut operands = Vec::with_capacity(plan.operands.len());
            let mut display_states = Vec::with_capacity(plan.operands.len());
            for operand in &plan.operands {
                let before = candidate
                    .entity(operand.id)
                    .cloned()
                    .ok_or(CadError::StaleResult)?;
                if before.object.revision != operand.revision || before != operand.snapshot {
                    return Err(CadError::StaleResult);
                }
                display_states.push(candidate.entity_display_state(before.id));
                operands.push(before);
            }
            let new_id = candidate.allocate_entity_id();
            let entity = DbEntity {
                object: DbObject {
                    id: ObjectId(new_id.0),
                    type_key: crate::app_drawing::solids::SOLID_TYPE_KEY.to_string(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: new_id,
                layer: plan.layer,
                space: plan.space.clone(),
                geometry: SemanticGeometry::Body3d(Box::new(body)),
                draw_order: plan.draw_order,
            };
            let transaction = next_transaction_id();
            let change_set = {
                let mut staged =
                    candidate.begin_drawing_transaction("boolean solids", transaction)?;
                for operand in &operands {
                    staged.delete_entity(operand.id)?;
                }
                staged.insert_entity(entity.clone())?;
                // The first operand's explicit display state travels with the
                // replacement: the boolean result is the same visible object, so
                // a per-entity colour/linetype/opacity override must not silently
                // fall back to ByLayer. `update_entity_display_state` supports an
                // id inserted by this very transaction.
                if let Some(state) = inherited_display_state(&display_states) {
                    staged.update_entity_display_state(new_id, state)?;
                }
                staged.commit()?
            };
            (
                candidate,
                operands,
                display_states,
                change_set,
                new_id,
                entity,
            )
        };

        // The deletes carry the display state each one prunes, so undo restores
        // the operands exactly (never a bare geometry row). The insert records the
        // inherited state too, so a **redo** restores the result with the same
        // override instead of dropping it to ByLayer.
        let inherited = inherited_display_state(&display_states).cloned();
        let mut patches: Vec<DrawingPatch> = operands
            .iter()
            .zip(display_states)
            .map(|(operand, state)| drawing_patch_deleted(operand.id, operand.clone(), state))
            .collect();
        patches.push(DrawingPatch {
            id: new_id,
            before: None,
            after: Some(entity),
            before_state: None,
            after_state: inherited,
        });
        let document_id = self.document_id;
        self.application.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: format!("{} solids", plan.op.as_str()),
                patches,
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        // The deleted operands are no longer addressable: drop their selection
        // refs rather than leaving a dangling set.
        self.application.reconcile_selection(&mut self.session);
        Ok(ObjectId(new_id.0))
    }
}

/// The first operand's display state, when it carries an explicit attribute
/// override.
///
/// `EntityRenderAttributes::default()` is the "no per-entity override" state (the
/// same test the transform/copy commands use). Visibility membership is **not**
/// inherited: a boolean operand is always a whole model-space row (block
/// instance paths are refused), so carrying it would only copy an unreachable
/// field. A plain body therefore inherits nothing, while an operand with an
/// explicit colour/linetype/lineweight/opacity keeps it.
pub(super) fn inherited_display_state(
    states: &[EntityDisplayState],
) -> Option<&EntityDisplayState> {
    states
        .first()
        .filter(|state| state.attributes != EntityRenderAttributes::default())
}
