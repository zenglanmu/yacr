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
use crate::tasks::{is_mesh_operand, mesh_geometry, MeshJobKind, MeshJobOperand, MeshJobPlan};
use cad_kernel_adapter::{BooleanOp, MeshBooleanOp, MeshData, MeshRepairMode, MeshWindingRule};

use super::solids::{boolean_operand_brep, describe_operand, invalid};

/// The ACIS-style class key every mesh boolean/repair result is stored under.
///
/// `cad-db` accepts it as a mesh row; it is deliberately **not** `AcDb3dSolid`
/// and carries no exact B-rep (ADR-0061 D4).
pub(crate) const MESH_TYPE_KEY: &str = "AcDbSubDMesh";

/// The stable refusal a carrier without a mesh modeler reports.
pub const MESH_MODELER_UNSUPPORTED: &str = "unsupported-runtime: mesh-modeler";

/// What actually happened when the command returned, per carrier.
///
/// Native hosts freeze a plan and start a worker, so the drawing settles later
/// and the message must not imply the result is already visible. `dom-canvas`
/// has no thread and runs the kernel inline: the command returns only after the
/// computation, and the next owner tick commits it, so saying "background"
/// there would be false (ADR-0061).
const MESH_JOB_SETTLEMENT: &str = if cfg!(target_arch = "wasm32") {
    "computed synchronously on the dom-canvas thread; the drawing changes on the next tick"
} else {
    "started in the background; the drawing changes when the job settles"
};

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
        started_label: &str,
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
            message: format!("{started_label}: {MESH_JOB_SETTLEMENT} ({operand_count} meshes)"),
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
            message: format!("{}: {MESH_JOB_SETTLEMENT}", mode.as_str()),
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

/// The safety valve on the combined OUTERSHELL input.
///
/// A shell union over more input than this is refused by name instead of tying
/// up a worker for an interaction the user cannot reasonably wait for. The
/// per-carrier `mesh.sync_budget` (dom-canvas inline execution) still applies.
pub(crate) const OUTERSHELL_MAX_INPUT_TRIANGLES: usize = 1_000_000;

/// The safety valve on the combined TRIMSOLID input (same shape as OUTERSHELL).
pub(crate) const TRIMSOLID_MAX_INPUT_TRIANGLES: usize = 1_000_000;

/// The safety valve on the combined SPLITSOLID input (same shape as TRIMSOLID).
pub(crate) const SPLITSOLID_MAX_INPUT_TRIANGLES: usize = 1_000_000;

impl Application {
    /// `OUTERSHELL`: union the selected closed mesh bodies into one shell.
    ///
    /// SketchUp's Outer Shell keeps only the outer surface of the selected
    /// solids, which for closed manifold bodies is exactly a mesh **union**
    /// through the same kernel the `UNION` command uses (ADR-0061). Unlike
    /// `UNION`, this command is mesh-only: an exact B-rep operand is refused by
    /// name instead of being silently tessellated, and every operand's topology
    /// is checked **before** a worker starts, so an open shell or a non-manifold
    /// row never reaches the kernel.
    ///
    /// Refusals (nothing written, no worker): `outershell.payload`,
    /// `outershell.needs_two`, `outershell.sub_element_not_supported`,
    /// `outershell.layer_mismatch`, `outershell.space_mismatch`,
    /// `outershell.not_mesh`, `outershell.open_shell`,
    /// `outershell.non_manifold`, `outershell.budget`, the carrier's
    /// `unsupported-runtime: mesh-modeler`, and Viewer's `PermissionDenied`.
    pub(crate) fn outer_shell(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "OUTERSHELL")?;
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        let refs = session.selection.refs().to_vec();
        if refs.len() < 2 {
            return Err(invalid(
                "outershell.needs_two",
                format!(
                    "OUTERSHELL needs at least two selected closed mesh bodies, got {}",
                    refs.len()
                ),
            ));
        }
        if refs
            .iter()
            .any(|reference| reference.sub_element.is_some() || !reference.instance.0.is_empty())
        {
            return Err(invalid(
                "outershell.sub_element_not_supported",
                "OUTERSHELL combines whole bodies; a sub-element or instance-path reference is not \
                 a body",
            ));
        }
        let (entities, diagnostics) = {
            let Some(document) = self.workspace.documents.get(&command.document) else {
                return Err(CadError::InvalidInput("document not open".into()));
            };
            let drawing = document.drawing.as_ref();
            Self::resolve_selection(
                drawing,
                command.document,
                &refs,
                "outershell",
                "OUTERSHELL",
                "combined",
                false,
            )?
        };
        if entities.len() < 2 {
            return Err(invalid(
                "outershell.needs_two",
                "OUTERSHELL needs at least two distinct selected bodies",
            ));
        }
        let layer = entities[0].layer;
        let space = entities[0].space.clone();
        for entity in &entities {
            if entity.layer != layer {
                return Err(invalid(
                    "outershell.layer_mismatch",
                    format!(
                        "entity {} is on layer {} but the first operand is on layer {}",
                        entity.id.0, entity.layer.0, layer.0
                    ),
                ));
            }
            if entity.space != space {
                return Err(invalid(
                    "outershell.space_mismatch",
                    format!(
                        "entity {} is in another space than the first operand; OUTERSHELL never \
                         crosses model/paper space",
                        entity.id.0
                    ),
                ));
            }
        }
        let mut total_triangles = 0usize;
        for entity in &entities {
            match classify_boolean_operand(entity) {
                Some(BooleanOperand::Mesh) => {}
                Some(BooleanOperand::Exact(_)) => {
                    return Err(invalid(
                        "outershell.not_mesh",
                        format!(
                            "entity {} is an exact solid; OUTERSHELL unions closed mesh bodies                              (use UNION for exact solids)",
                            entity.id.0
                        ),
                    ))
                }
                None => {
                    return Err(invalid(
                        "outershell.not_mesh",
                        format!(
                            "entity {} is {}; OUTERSHELL needs closed mesh bodies",
                            entity.id.0,
                            describe_operand(entity)
                        ),
                    ))
                }
            }
            let Some(mesh) = mesh_geometry(entity) else {
                return Err(invalid(
                    "outershell.not_mesh",
                    format!("entity {} carries no mesh geometry", entity.id.0),
                ));
            };
            total_triangles += mesh.triangles.len();
            if total_triangles > OUTERSHELL_MAX_INPUT_TRIANGLES {
                return Err(invalid(
                    "outershell.budget",
                    format!(
                        "the selected meshes hold {total_triangles} triangles, over the \
                         {OUTERSHELL_MAX_INPUT_TRIANGLES} OUTERSHELL input budget; reduce the \
                         selection"
                    ),
                ));
            }
            let data = MeshData::from_parts(mesh.vertices.clone(), mesh.triangles.clone());
            let topology = cad_kernel_adapter::inspect_topology_welded(&data);
            match topology.topology() {
                cad_kernel_adapter::MeshTopology::ClosedManifold => {}
                cad_kernel_adapter::MeshTopology::OpenShell => {
                    return Err(invalid(
                        "outershell.open_shell",
                        format!(
                            "entity {} is an open shell ({} boundary edge(s)); OUTERSHELL needs \
                             closed bodies",
                            entity.id.0, topology.boundary_edges
                        ),
                    ))
                }
                cad_kernel_adapter::MeshTopology::NonManifold => {
                    return Err(invalid(
                        "outershell.non_manifold",
                        format!(
                            "entity {} is non-manifold ({} edge(s) shared by three or more \
                             triangles); OUTERSHELL needs closed manifold bodies",
                            entity.id.0, topology.non_manifold_edges
                        ),
                    ))
                }
            }
        }
        self.boolean_meshes(
            command,
            BooleanOp::Union,
            entities,
            diagnostics,
            "OUTERSHELL",
        )
    }
}

impl Application {
    /// `TRIMSOLID`: subtract every cutter from the first selected mesh body and
    /// keep the cutters as their own rows.
    ///
    /// SketchUp Solid Tools **Trim** keeps both results: the trimmed target and
    /// the cutting solid. The kernel run is the same subtract fold `SUBTRACT`
    /// uses; the commit (this function's plan kind) deletes only the target and
    /// inserts the result, leaving each cutter row untouched. One transaction /
    /// one undo step, exactly like the other mesh operations.
    ///
    /// The **first** selected body is the target; every other selected body is a
    /// cutter. Every operand must be a closed manifold mesh (an exact solid is
    /// refused with a pointer to `SUBTRACT`), on the target's space, and the
    /// combined input is capped by [`TRIMSOLID_MAX_INPUT_TRIANGLES`].
    ///
    /// Refusals (nothing written, no worker): `trimsolid.payload`,
    /// `trimsolid.needs_two`, `trimsolid.sub_element_not_supported`,
    /// `trimsolid.space_mismatch`, `trimsolid.not_mesh`, `trimsolid.open_shell`,
    /// `trimsolid.non_manifold`, `trimsolid.budget`, the carrier's
    /// `unsupported-runtime: mesh-modeler`, and Viewer's `PermissionDenied`.
    pub(crate) fn trim_solid(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "TRIMSOLID")?;
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        let refs = session.selection.refs().to_vec();
        if refs.len() < 2 {
            return Err(invalid(
                "trimsolid.needs_two",
                format!(
                    "TRIMSOLID needs a target and at least one cutter body, got {} selection(s)",
                    refs.len()
                ),
            ));
        }
        if refs
            .iter()
            .any(|reference| reference.sub_element.is_some() || !reference.instance.0.is_empty())
        {
            return Err(invalid(
                "trimsolid.sub_element_not_supported",
                "TRIMSOLID combines whole bodies; a sub-element or instance-path reference is not \
                 a body",
            ));
        }
        let (entities, diagnostics) = {
            let Some(document) = self.workspace.documents.get(&command.document) else {
                return Err(CadError::InvalidInput("document not open".into()));
            };
            let drawing = document.drawing.as_ref();
            Self::resolve_selection(
                drawing,
                command.document,
                &refs,
                "trimsolid",
                "TRIMSOLID",
                "trimmed",
                false,
            )?
        };
        if entities.len() < 2 {
            return Err(invalid(
                "trimsolid.needs_two",
                "TRIMSOLID needs at least two distinct selected bodies",
            ));
        }
        let target_space = entities[0].space.clone();
        let mut total_triangles = 0usize;
        for entity in &entities {
            if entity.space != target_space {
                return Err(invalid(
                    "trimsolid.space_mismatch",
                    format!(
                        "entity {} is in another space than the target; TRIMSOLID never crosses \
                         model/paper space",
                        entity.id.0
                    ),
                ));
            }
            match classify_boolean_operand(entity) {
                Some(BooleanOperand::Mesh) => {}
                Some(BooleanOperand::Exact(_)) => {
                    return Err(invalid(
                        "trimsolid.not_mesh",
                        format!(
                            "entity {} is an exact solid; TRIMSOLID trims closed mesh bodies (use \
                             SUBTRACT for exact solids)",
                            entity.id.0
                        ),
                    ))
                }
                None => {
                    return Err(invalid(
                        "trimsolid.not_mesh",
                        format!(
                            "entity {} is {}; TRIMSOLID needs closed mesh bodies",
                            entity.id.0,
                            describe_operand(entity)
                        ),
                    ))
                }
            }
            let Some(mesh) = mesh_geometry(entity) else {
                return Err(invalid(
                    "trimsolid.not_mesh",
                    format!("entity {} carries no mesh geometry", entity.id.0),
                ));
            };
            total_triangles += mesh.triangles.len();
            if total_triangles > TRIMSOLID_MAX_INPUT_TRIANGLES {
                return Err(invalid(
                    "trimsolid.budget",
                    format!(
                        "the selected meshes hold {total_triangles} triangles, over the \
                         {TRIMSOLID_MAX_INPUT_TRIANGLES} TRIMSOLID input budget; reduce the \
                         selection"
                    ),
                ));
            }
            let data = MeshData::from_parts(mesh.vertices.clone(), mesh.triangles.clone());
            let topology = cad_kernel_adapter::inspect_topology_welded(&data);
            match topology.topology() {
                cad_kernel_adapter::MeshTopology::ClosedManifold => {}
                cad_kernel_adapter::MeshTopology::OpenShell => {
                    return Err(invalid(
                        "trimsolid.open_shell",
                        format!(
                            "entity {} is an open shell ({} boundary edge(s)); TRIMSOLID needs \
                             closed bodies",
                            entity.id.0, topology.boundary_edges
                        ),
                    ))
                }
                cad_kernel_adapter::MeshTopology::NonManifold => {
                    return Err(invalid(
                        "trimsolid.non_manifold",
                        format!(
                            "entity {} is non-manifold ({} edge(s) shared by three or more \
                             triangles); TRIMSOLID needs closed manifold bodies",
                            entity.id.0, topology.non_manifold_edges
                        ),
                    ))
                }
            }
        }
        self.trim_meshes(command, entities, diagnostics)
    }

    /// Start the trim job for an already-validated selection.
    ///
    /// Same worker/commit path as the mesh booleans; the plan kind (`Trim`) is
    /// what makes the commit keep the cutter rows. The result inherits the
    /// target's layer, space and draw order.
    fn trim_meshes(
        &mut self,
        command: &Command,
        entities: Vec<DbEntity>,
        mut diagnostics: Vec<Diagnostic>,
    ) -> CadResult<CommandOutcome> {
        let Some(modeler) = self.mesh_modeler.clone() else {
            return Err(CadError::Unsupported(MESH_MODELER_UNSUPPORTED.into()));
        };
        let Some(target) = entities.first() else {
            return Err(CadError::Invariant(
                "trimsolid.invalid_plan: TRIMSOLID needs a target and at least one cutter".into(),
            ));
        };
        let layer = target.layer;
        let space = target.space.clone();
        let draw_order = target.draw_order;
        let cutter_count = entities.len() - 1;
        let operands = entities.into_iter().map(MeshJobOperand::new).collect();
        let plan = MeshJobPlan {
            document: command.document,
            kind: MeshJobKind::Trim,
            layer,
            space,
            draw_order,
            operands,
        };
        self.start_mesh_job(plan, modeler)?;
        diagnostics.push(Diagnostic {
            object: None,
            code: "mesh.trim_started".into(),
            message: format!(
                "TRIMSOLID: {MESH_JOB_SETTLEMENT} (target + {cutter_count} cutter(s); the \
                 cutters are kept)"
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

impl Application {
    /// `SPLITSOLID`: cut the first selected mesh body by the union of the other
    /// selected bodies into an **outside** and an **inside** piece.
    ///
    /// The first selected body is the target; every other selected body is a
    /// cutter and is kept as its own row (a cutter is a tool, not the piece being
    /// split). The worker unions the cutters and runs `target − ∪cutters` and
    /// `target ∩ ∪cutters`, so the two results tile the target exactly; both
    /// results are inserted in one transaction / one undo step.
    ///
    /// Refusals (nothing written, no worker): `splitsolid.payload`,
    /// `splitsolid.needs_two`, `splitsolid.sub_element_not_supported`,
    /// `splitsolid.space_mismatch`, `splitsolid.not_mesh`, `splitsolid.open_shell`,
    /// `splitsolid.non_manifold`, `splitsolid.budget`, the carrier's
    /// `unsupported-runtime: mesh-modeler`, and Viewer's `PermissionDenied`.
    /// A cut that leaves one side empty fails the job with `splitsolid.no_intersection`
    /// or `splitsolid.empty_piece` and writes nothing.
    pub(crate) fn split_solid(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "SPLITSOLID")?;
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        let refs = session.selection.refs().to_vec();
        if refs.len() < 2 {
            return Err(invalid(
                "splitsolid.needs_two",
                format!(
                    "SPLITSOLID needs a target and at least one cutter body, got {} selection(s)",
                    refs.len()
                ),
            ));
        }
        if refs
            .iter()
            .any(|reference| reference.sub_element.is_some() || !reference.instance.0.is_empty())
        {
            return Err(invalid(
                "splitsolid.sub_element_not_supported",
                "SPLITSOLID combines whole bodies; a sub-element or instance-path reference is not \
                 a body",
            ));
        }
        let (entities, diagnostics) = {
            let Some(document) = self.workspace.documents.get(&command.document) else {
                return Err(CadError::InvalidInput("document not open".into()));
            };
            let drawing = document.drawing.as_ref();
            Self::resolve_selection(
                drawing,
                command.document,
                &refs,
                "splitsolid",
                "SPLITSOLID",
                "split",
                false,
            )?
        };
        if entities.len() < 2 {
            return Err(invalid(
                "splitsolid.needs_two",
                "SPLITSOLID needs at least two distinct selected bodies",
            ));
        }
        let target_space = entities[0].space.clone();
        let mut total_triangles = 0usize;
        for entity in &entities {
            if entity.space != target_space {
                return Err(invalid(
                    "splitsolid.space_mismatch",
                    format!(
                        "entity {} is in another space than the target; SPLITSOLID never crosses \
                         model/paper space",
                        entity.id.0
                    ),
                ));
            }
            match classify_boolean_operand(entity) {
                Some(BooleanOperand::Mesh) => {}
                Some(BooleanOperand::Exact(_)) => {
                    return Err(invalid(
                        "splitsolid.not_mesh",
                        format!(
                            "entity {} is an exact solid; SPLITSOLID splits closed mesh bodies \
                             (use SUBTRACT for exact solids)",
                            entity.id.0
                        ),
                    ))
                }
                None => {
                    return Err(invalid(
                        "splitsolid.not_mesh",
                        format!(
                            "entity {} is {}; SPLITSOLID needs closed mesh bodies",
                            entity.id.0,
                            describe_operand(entity)
                        ),
                    ))
                }
            }
            let Some(mesh) = mesh_geometry(entity) else {
                return Err(invalid(
                    "splitsolid.not_mesh",
                    format!("entity {} carries no mesh geometry", entity.id.0),
                ));
            };
            total_triangles += mesh.triangles.len();
            if total_triangles > SPLITSOLID_MAX_INPUT_TRIANGLES {
                return Err(invalid(
                    "splitsolid.budget",
                    format!(
                        "the selected meshes hold {total_triangles} triangles, over the \
                         {SPLITSOLID_MAX_INPUT_TRIANGLES} SPLITSOLID input budget; reduce the \
                         selection"
                    ),
                ));
            }
            let data = MeshData::from_parts(mesh.vertices.clone(), mesh.triangles.clone());
            let topology = cad_kernel_adapter::inspect_topology_welded(&data);
            match topology.topology() {
                cad_kernel_adapter::MeshTopology::ClosedManifold => {}
                cad_kernel_adapter::MeshTopology::OpenShell => {
                    return Err(invalid(
                        "splitsolid.open_shell",
                        format!(
                            "entity {} is an open shell ({} boundary edge(s)); SPLITSOLID needs \
                             closed bodies",
                            entity.id.0, topology.boundary_edges
                        ),
                    ))
                }
                cad_kernel_adapter::MeshTopology::NonManifold => {
                    return Err(invalid(
                        "splitsolid.non_manifold",
                        format!(
                            "entity {} is non-manifold ({} edge(s) shared by three or more \
                             triangles); SPLITSOLID needs closed manifold bodies",
                            entity.id.0, topology.non_manifold_edges
                        ),
                    ))
                }
            }
        }
        self.split_meshes(command, entities, diagnostics)
    }

    /// Start the split job for an already-validated selection.
    fn split_meshes(
        &mut self,
        command: &Command,
        entities: Vec<DbEntity>,
        mut diagnostics: Vec<Diagnostic>,
    ) -> CadResult<CommandOutcome> {
        let Some(modeler) = self.mesh_modeler.clone() else {
            return Err(CadError::Unsupported(MESH_MODELER_UNSUPPORTED.into()));
        };
        let Some(target) = entities.first() else {
            return Err(CadError::Invariant(
                "splitsolid.invalid_plan: SPLITSOLID needs a target and at least one cutter".into(),
            ));
        };
        let layer = target.layer;
        let space = target.space.clone();
        let draw_order = target.draw_order;
        let cutter_count = entities.len() - 1;
        let operands = entities.into_iter().map(MeshJobOperand::new).collect();
        let plan = MeshJobPlan {
            document: command.document,
            kind: MeshJobKind::Split,
            layer,
            space,
            draw_order,
            operands,
        };
        self.start_mesh_job(plan, modeler)?;
        diagnostics.push(Diagnostic {
            object: None,
            code: "mesh.split_started".into(),
            message: format!(
                "SPLITSOLID: {MESH_JOB_SETTLEMENT} (target + {cutter_count} cutter(s); two pieces, \
                 outside and inside; the cutters are kept)"
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
