//! P-MESH-1a/1b command layer: mesh booleans and mesh repair.
//!
//! `UNION`/`SUBTRACT`/`INTERSECT` are the **same** user commands as the exact
//! solid booleans (ADR-0061 P-MESH-1a): the operand class decides the seam. A
//! selection of exact `AcDb3dSolid` rows goes to `SolidModeler`; a selection of
//! closed mesh rows (`SemanticGeometry::Mesh`, or an imported
//! `Body3d { kind: Unknown }` display body) goes to `MeshModeler`. Mixing the two
//! classes refuses with `boolean.operand_class_mismatch` rather than tessellating
//! an exact body into a mesh (which would silently downgrade engineering
//! geometry).
//!
//! `MESHREPAIR` (orientation) and `MESHREBUILD` (winding-number rebuild) are the
//! P-MESH-1b repair commands. They take the one selected mesh, run the repair on
//! a worker, and let the host commit the closed product in one transaction. A
//! repair result is always stored as a mesh and never as exact B-rep geometry.
//!
//! Every refusal happens before a worker starts, so the drawing, its revision,
//! the id allocator and history stay untouched.

use super::*;
use crate::app_utility::require_none_payload;
use crate::tasks::{is_mesh_operand, MeshJobKind, MeshJobOperand, MeshJobPlan};
use cad_kernel_adapter::{BooleanOp, MeshBooleanOp, MeshRepairMode, MeshWindingRule};

use super::solids::{boolean_operand_brep, describe_operand, invalid, op_label};

/// The ACIS-style class key every mesh boolean/repair result is stored under.
///
/// `cad-db` accepts it as a mesh row; it is deliberately **not** `AcDb3dSolid`
/// and carries no exact B-rep (ADR-0061 D4).
pub(crate) const MESH_TYPE_KEY: &str = "AcDbSubDMesh";

/// The stable refusal a carrier without a mesh modeler reports.
pub const MESH_MODELER_UNSUPPORTED: &str = "unsupported-runtime: mesh-modeler";

/// The operand classes a boolean can accept.
pub(crate) enum BooleanOperand {
    /// One exact `Body3d { Solid, Exact }` stored as `AcDb3dSolid`.
    Exact(cad_domain::BrepData),
    /// One closed triangle mesh, either a `Mesh` row or an imported preview body.
    ///
    /// No payload: the row itself is moved into the job plan, which derives the
    /// neutral triangle view on the worker. Cloning the mesh here just to
    /// classify it would double every operand's buffers on the command thread.
    Mesh,
}

/// Classify one selected row for a boolean.
///
/// Returns `None` for anything that is neither an exact solid operand nor a mesh
/// body; the caller turns that into `boolean.non_solid_operand`.
pub(crate) fn classify_boolean_operand(entity: &DbEntity) -> Option<BooleanOperand> {
    if let Some(brep) = boolean_operand_brep(entity) {
        return Some(BooleanOperand::Exact(brep));
    }
    is_mesh_operand(entity).then_some(BooleanOperand::Mesh)
}

fn mesh_boolean_op(op: BooleanOp) -> MeshBooleanOp {
    match op {
        BooleanOp::Union => MeshBooleanOp::Union,
        BooleanOp::Subtract => MeshBooleanOp::Subtract,
        BooleanOp::Intersect => MeshBooleanOp::Intersect,
    }
}

impl Application {
    /// Start a mesh boolean for an already-validated selection.
    ///
    /// The caller (`boolean_solids`) has checked arity, whole-body references,
    /// layer/space agreement and that every operand is a mesh. This function
    /// only requires the carrier's modeler, snapshots the operands and starts
    /// the worker.
    pub(crate) fn boolean_meshes(
        &mut self,
        command: &Command,
        op: BooleanOp,
        entities: Vec<DbEntity>,
        mut diagnostics: Vec<Diagnostic>,
    ) -> CadResult<CommandOutcome> {
        let Some(modeler) = self.mesh_modeler.clone() else {
            return Err(CadError::Unsupported(MESH_MODELER_UNSUPPORTED.into()));
        };
        // The caller checked arity and layer/space agreement; the target is the
        // first operand's layer/space and the group's lowest draw order.
        let Some(first) = entities.first() else {
            return Err(CadError::Invariant(
                "mesh.boolean_invalid_plan: a mesh boolean needs at least two operands".into(),
            ));
        };
        let layer = first.layer;
        let space = first.space.clone();
        let draw_order = entities
            .iter()
            .map(|entity| entity.draw_order)
            .min()
            .unwrap_or(0);
        let operand_count = entities.len();
        let operands = entities.into_iter().map(MeshJobOperand::new).collect();
        let plan = MeshJobPlan {
            document: command.document,
            kind: MeshJobKind::Boolean(mesh_boolean_op(op)),
            layer,
            space,
            draw_order,
            operands,
        };
        self.start_mesh_job(plan, modeler)?;
        diagnostics.push(Diagnostic {
            object: None,
            code: "mesh.boolean_started".into(),
            message: format!(
                "{}: started in the background for {operand_count} meshes; the drawing changes \
                 when the job settles",
                op_label(op)
            ),
        });
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `MESHREPAIR` / `MESHREBUILD`: repair the one selected closed mesh.
    ///
    /// The kernel refuses an open shell with `mesh.not_closed`; this layer never
    /// fills holes (that is P-MESH-3).
    pub(crate) fn repair_mesh(
        &mut self,
        session: &SessionState,
        command: &Command,
        mode: MeshRepairMode,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, mode.as_str())?;
        let Some(modeler) = self.mesh_modeler.clone() else {
            return Err(CadError::Unsupported(MESH_MODELER_UNSUPPORTED.into()));
        };
        let refs = session.selection.refs().to_vec();
        if refs
            .iter()
            .any(|reference| reference.sub_element.is_some() || !reference.instance.0.is_empty())
        {
            return Err(invalid(
                "meshrepair.sub_element_not_supported",
                "mesh repair works on a whole mesh body; a sub-element or instance-path reference \
                 is not one",
            ));
        }
        if refs.len() != 1 {
            return Err(invalid(
                "meshrepair.needs_one",
                format!(
                    "{} needs exactly one selected mesh body, got {}",
                    mode.as_str(),
                    refs.len()
                ),
            ));
        }
        let (entity, diagnostics) = {
            let Some(document) = self.workspace.documents.get(&command.document) else {
                return Err(CadError::InvalidInput("document not open".into()));
            };
            let drawing = document.drawing.as_ref();
            let (operands, diagnostics) = Self::resolve_selection(
                drawing,
                command.document,
                &refs,
                "meshrepair",
                mode.as_str(),
                "repair",
                false,
            )?;
            let Some(entity) = operands.into_iter().next() else {
                return Err(invalid(
                    "meshrepair.needs_one",
                    "the selected mesh no longer exists",
                ));
            };
            if !is_mesh_operand(&entity) {
                return Err(invalid(
                    "meshrepair.not_mesh",
                    format!(
                        "entity {} is {}; mesh repair needs a mesh body",
                        entity.id.0,
                        describe_operand(&entity)
                    ),
                ));
            }
            (entity, diagnostics)
        };
        let rule = match mode {
            MeshRepairMode::Orientation => MeshWindingRule::Positive,
            MeshRepairMode::Rebuild => MeshWindingRule::Nonzero,
        };
        let layer = entity.layer;
        let space = entity.space.clone();
        let draw_order = entity.draw_order;
        let plan = MeshJobPlan {
            document: command.document,
            kind: MeshJobKind::Repair { mode, rule },
            layer,
            space,
            draw_order,
            operands: vec![MeshJobOperand::new(entity)],
        };
        self.start_mesh_job(plan, modeler)?;
        let mut diagnostics = diagnostics;
        diagnostics.push(Diagnostic {
            object: None,
            code: "mesh.repair_started".into(),
            message: format!(
                "{}: started in the background; the drawing changes when the job settles",
                mode.as_str()
            ),
        });
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}
