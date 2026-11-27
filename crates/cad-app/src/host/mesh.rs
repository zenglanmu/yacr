//! Host-side commit of the background mesh boolean/repair (P-MESH-1a/1b).
//!
//! The command half (`app_drawing::mesh_solids`) validates the selection and
//! starts one worker; this module takes the terminal mesh and publishes it only
//! when the result is still current:
//!
//! * the manager stamp (document + generation) must match, so a superseded job
//!   can never publish;
//! * every operand must still exist with the revision **and full row** it had at
//!   start, so an edit made while the worker ran discards the result;
//! * the delete and the insert happen in one transaction with one undo record.
//!
//! The produced geometry is stored as [`SemanticGeometry::Mesh`] under
//! `AcDbSubDMesh`, with no `face_sources`, normals, UVs or material: a mesh
//! boolean/repair result is never exact B-rep geometry and must not claim the
//! source's face identity (ADR-0061 D4).

use super::*;
use crate::app_drawing::{next_transaction_id, MESH_TYPE_KEY};
use crate::tasks::{MeshJobPlan, MeshJobTake};
use cad_db::{DbEntity, DbObject};
use cad_history::{drawing_patch_deleted, DrawingPatch, DrawingUndoRecord};
use cad_kernel_adapter::{MeshData, MeshRepair};

use super::solids::inherited_display_state;

/// The outcome of polling the background mesh job once.
#[derive(Debug)]
pub enum MeshJobPoll {
    /// No mesh job is current.
    Idle,
    /// The worker is still running.
    Running,
    /// The mesh was committed in one transaction; the operands are gone.
    Done {
        object: ObjectId,
        /// The inside piece's row for a split; `None` for every other job.
        second_object: Option<ObjectId>,
        /// Present for a repair (`Partial`); `None` for a boolean.
        repair: Option<MeshRepair>,
        diagnostics: Vec<Diagnostic>,
    },
    /// The host cancelled the job; nothing was published.
    Cancelled,
    /// The result was discarded as stale (superseded, document replaced, or an
    /// operand changed between start and poll); nothing was published.
    Stale,
    /// The mesh operation genuinely failed; the drawing is untouched.
    Failed { error: CadError },
}

impl HostController {
    /// Poll the background mesh job and publish a current, non-stale mesh.
    ///
    /// The drawing is never modified unless `Done` is returned.
    pub fn poll_mesh_job(&mut self) -> MeshJobPoll {
        match self.application.poll_mesh_job_take() {
            MeshJobTake::Idle => MeshJobPoll::Idle,
            MeshJobTake::Running => MeshJobPoll::Running,
            MeshJobTake::Cancelled => MeshJobPoll::Cancelled,
            MeshJobTake::Stale => MeshJobPoll::Stale,
            MeshJobTake::Failed(error) => MeshJobPoll::Failed { error },
            MeshJobTake::Done {
                plan,
                mesh,
                topology: _topology,
                second,
                repair,
                diagnostics,
            } => match self.commit_mesh_job(&plan, mesh, second.map(|pair| pair.0), repair) {
                Ok((object, second_object)) => MeshJobPoll::Done {
                    object,
                    second_object,
                    repair,
                    diagnostics,
                },
                Err(CadError::StaleResult) => MeshJobPoll::Stale,
                Err(error) => MeshJobPoll::Failed { error },
            },
        }
    }

    /// Ask the running mesh job to stop. The slot is kept so the next poll
    /// reports [`MeshJobPoll::Cancelled`]; nothing is published.
    pub fn cancel_mesh_job(&mut self) {
        self.application.cancel_mesh_job();
    }

    /// Commit the mesh product(s) in one transaction and one undo step.
    ///
    /// `Trim`/`Split` delete only the target (operand 0) and keep every cutter;
    /// `Split` inserts two rows (outside then inside). Every result inherits the
    /// target's layer, space, draw order and display state.
    fn commit_mesh_job(
        &mut self,
        plan: &MeshJobPlan,
        mesh: MeshData,
        second: Option<MeshData>,
        _repair: Option<MeshRepair>,
    ) -> CadResult<(ObjectId, Option<ObjectId>)> {
        if plan.document != self.document_id {
            return Err(CadError::StaleResult);
        }
        if plan.operands.is_empty() {
            return Err(CadError::Invariant(
                "mesh.job_invalid_plan: a mesh job carries no operands".into(),
            ));
        }
        // Trim and Split keep every cutter (operand 1..) and replace only the
        // target; a boolean or repair replaces the whole operand group.
        let keep_cutters = matches!(
            plan.kind,
            crate::tasks::MeshJobKind::Trim | crate::tasks::MeshJobKind::Split
        );
        let (candidate, operands, display_states, change_set, entities) = {
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
            let inherited = if keep_cutters {
                display_states.first().cloned()
            } else {
                inherited_display_state(&display_states).cloned()
            };
            let mut products: Vec<MeshData> = vec![mesh];
            if let Some(second) = second {
                products.push(second);
            }
            let mut entities = Vec::with_capacity(products.len());
            for product in products {
                let new_id = candidate.allocate_entity_id();
                entities.push(DbEntity {
                    object: DbObject {
                        id: ObjectId(new_id.0),
                        type_key: MESH_TYPE_KEY.to_string(),
                        revision: Revision(0),
                        source_handle: None,
                    },
                    id: new_id,
                    layer: plan.layer,
                    space: plan.space.clone(),
                    // A mesh result is derived display geometry: no source face
                    // identity, normals, UVs or material travels with it.
                    geometry: SemanticGeometry::Mesh(cad_domain::Mesh {
                        vertices: product.vertices,
                        triangles: product.triangles,
                        ..Default::default()
                    }),
                    draw_order: plan.draw_order,
                });
            }
            let transaction = next_transaction_id();
            let change_set = {
                let mut staged =
                    candidate.begin_drawing_transaction("mesh operation", transaction)?;
                for (index, operand) in operands.iter().enumerate() {
                    if !keep_cutters || index == 0 {
                        staged.delete_entity(operand.id)?;
                    }
                }
                for entity in &entities {
                    staged.insert_entity(entity.clone())?;
                }
                if let Some(state) = inherited {
                    for entity in &entities {
                        staged.update_entity_display_state(entity.id, &state)?;
                    }
                }
                staged.commit()?
            };
            (candidate, operands, display_states, change_set, entities)
        };

        // A kept operand gets no delete patch, so undo cannot resurrect a second
        // copy of the cutter.
        let inherited = if keep_cutters {
            display_states.first().cloned()
        } else {
            inherited_display_state(&display_states).cloned()
        };
        let mut patches: Vec<DrawingPatch> = operands
            .iter()
            .zip(display_states)
            .enumerate()
            .filter(|(index, _)| !keep_cutters || *index == 0)
            .map(|(_, (operand, state))| drawing_patch_deleted(operand.id, operand.clone(), state))
            .collect();
        for entity in &entities {
            patches.push(DrawingPatch {
                id: entity.id,
                before: None,
                after: Some(entity.clone()),
                before_state: None,
                after_state: inherited.clone(),
            });
        }
        let document_id = self.document_id;
        self.application.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: format!("{} mesh", plan.kind.as_str()),
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
        self.application.reconcile_selection(&mut self.session);
        let mut ids = entities.iter().map(|entity| ObjectId(entity.id.0));
        let first = ids.next().ok_or_else(|| {
            CadError::Invariant("mesh.job_invalid_plan: a mesh job produced no row".into())
        })?;
        Ok((first, ids.next()))
    }
}
