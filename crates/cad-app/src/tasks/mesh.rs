//! P-MESH-1a/1b: one mesh job for mesh booleans and mesh repair.
//!
//! The shape mirrors [`super::solid::SolidBooleanManager`] exactly: one current
//! job per controller, a frozen plan, a live cancellation token, and a terminal
//! value that only publishes when its [`TaskStamp`] is still current. The
//! difference is what crosses the execution boundary:
//!
//! * the kernel seam is [`cad_kernel_adapter::MeshModeler`] (never
//!   `SolidModeler`), so the result is always a triangle mesh;
//! * the product is **not** exact B-rep geometry and the host never stores it as
//!   `Body3d { Solid, Exact }` (ADR-0061 D4);
//! * a boolean and a repair share this one slot, so the two mesh operations can
//!   never run at once and a document replacement/close aborts both.
//!
//! Native hosts run the frozen plan on a worker thread. `dom-canvas` has no OS
//! thread, so its wasm build runs the same plan **inline on the session thread**
//! (the deliberate ADR-0061 exception): the UI is blocked for the call, there is
//! no mid-call cancellation, and oversized operands are refused by
//! [`sync_budget_refusal`] before the kernel starts.
//!
//! The worker (or the synchronous wasm call) owns an immutable snapshot of every
//! operand. The host re-checks the exact rows (revision **and** full entity)
//! before committing, so an edit made while the job ran discards the result
//! instead of deleting newer geometry. A cancelled or superseded result never
//! touches `cad-db`.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;

use cad_db::DbEntity;
use cad_domain::{
    BodyKind, CadError, CadResult, Diagnostic, DocumentId, EntityId, LayerId, Revision,
    SemanticGeometry, SpaceId, TaskStamp,
};
use cad_kernel_adapter::{
    codes, execute_mesh, MeshBooleanOp, MeshData, MeshModeler, MeshOperation, MeshOutcome,
    MeshRepair, MeshRepairMode, MeshRequest, MeshTopology, MeshTopologyReport, MeshWindingRule,
};

use cad_platform::CancellationToken;

/// One operand as it existed when the mesh command started.
///
/// The frozen row is **moved** into the plan, never cloned: `DbEntity` carries
/// the source mesh, so a clone here would duplicate every vertex/triangle on
/// the command thread before the worker even starts. The neutral
/// [`MeshData`](cad_kernel_adapter::MeshData) view is derived once inside the
/// worker ([`Self::mesh`]); the same row is the exact snapshot the host compares
/// before publishing.
#[derive(Debug)]
pub(crate) struct MeshJobOperand {
    pub(crate) id: EntityId,
    pub(crate) revision: Revision,
    /// The immutable row the host compares before publishing.
    pub(crate) snapshot: DbEntity,
}

impl MeshJobOperand {
    /// Take ownership of one validated mesh row (no geometry clone).
    pub(crate) fn new(entity: DbEntity) -> Self {
        MeshJobOperand {
            id: entity.id,
            revision: entity.object.revision,
            snapshot: entity,
        }
    }

    /// The neutral triangle data handed to the kernel, derived on the worker.
    ///
    /// The caller validated the row before starting the job; `None` here means
    /// the frozen plan no longer describes a mesh and is an invariant failure,
    /// never a silent success.
    pub(crate) fn mesh(&self) -> Option<MeshData> {
        mesh_operand_data(&self.snapshot)
    }

    /// Vertex/triangle counts without copying the buffers.
    ///
    /// Used by the dom-canvas synchronous budget, which runs on the command
    /// thread and must not clone a whole mesh just to size it.
    #[cfg(any(target_arch = "wasm32", test))]
    fn geometry_counts(&self) -> (usize, usize) {
        mesh_geometry(&self.snapshot)
            .map(|mesh| (mesh.vertices.len(), mesh.triangles.len()))
            .unwrap_or((0, 0))
    }
}

/// The domain mesh behind a mesh-ish row, or `None`.
///
/// This accepts the two shapes the importers actually produce:
///
/// * `SemanticGeometry::Mesh` (STL/OBJ/PLY/OFF and any `.ymodel` mesh row);
/// * `Body3d { kind: Unknown, exact: Unavailable }` whose display mesh is the
///   only geometry (the SKP preview body, ADR-0049).
///
/// A `Sheet`/`Wire` body, or a body with exact geometry, is **not** a mesh
/// operand: a sheet has no volume and an exact body must go through the solid
/// seam, never be silently tessellated.
pub(crate) fn mesh_geometry(entity: &DbEntity) -> Option<&cad_domain::Mesh> {
    match &entity.geometry {
        SemanticGeometry::Mesh(mesh) if !mesh.triangles.is_empty() && !mesh.vertices.is_empty() => {
            Some(mesh)
        }
        SemanticGeometry::Body3d(body)
            if body.kind == BodyKind::Unknown
                && body.exact_brep().is_none()
                && !body.display.mesh.triangles.is_empty()
                && !body.display.mesh.vertices.is_empty() =>
        {
            Some(&body.display.mesh)
        }
        _ => None,
    }
}

/// The neutral triangle view of a mesh-ish row, or `None`.
///
/// The worker calls this once per operand; the buffers are copied into the
/// kernel-neutral `MeshData` only here, never on the command thread.
fn mesh_operand_data(entity: &DbEntity) -> Option<MeshData> {
    mesh_geometry(entity)
        .map(|mesh| MeshData::from_parts(mesh.vertices.clone(), mesh.triangles.clone()))
}

/// Whether this row is a mesh operand, without copying its geometry.
///
/// Classification runs on the command thread; building [`MeshData`] here would
/// clone the whole vertex/triangle buffers just to answer a yes/no question.
pub(crate) fn is_mesh_operand(entity: &DbEntity) -> bool {
    mesh_geometry(entity).is_some()
}

/// What the worker should compute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MeshJobKind {
    /// Union/subtract/intersect of two or more closed meshes.
    Boolean(MeshBooleanOp),
    /// Orientation or winding-number rebuild of one closed mesh.
    Repair {
        mode: MeshRepairMode,
        rule: MeshWindingRule,
    },
    /// SketchUp Solid Tools **Trim**: subtract the cutters (operands 1..) from
    /// the target (operand 0) and keep every cutter row untouched.
    ///
    /// The kernel operation is the same subtract fold as `Boolean(Subtract)`;
    /// only the commit differs (it deletes the target alone and keeps the
    /// cutters), so the kind is the single authority for both halves.
    Trim,
}

impl MeshJobKind {
    /// The stable lower-case tag used in a diagnostic/code prefix.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            MeshJobKind::Boolean(op) => op.as_str(),
            MeshJobKind::Repair { mode, .. } => mode.as_str(),
            MeshJobKind::Trim => "trim",
        }
    }
}

/// Everything the worker needs, frozen before the job starts.
///
/// Deliberately not `Clone`: the plan owns the only frozen copies of the
/// operand rows, and the manager shares the plan itself through `Arc`. Cloning
/// it would duplicate every operand's geometry.
#[derive(Debug)]
pub(crate) struct MeshJobPlan {
    pub(crate) document: DocumentId,
    pub(crate) kind: MeshJobKind,
    pub(crate) layer: LayerId,
    pub(crate) space: SpaceId,
    pub(crate) draw_order: i64,
    /// A boolean carries two or more; a repair carries exactly one.
    pub(crate) operands: Vec<MeshJobOperand>,
}

/// The largest total operand triangle count a **synchronous** dom-canvas mesh
/// job may carry.
///
/// `dom-canvas` has no worker thread, so the kernel runs inline on the session
/// thread and blocks the page for the whole call. This cap is a **liveness
/// guard**, not a performance target or a claim that a job at the limit is
/// fast: it refuses inputs that are obviously too large to finish inside an
/// interactive stall, with the stable `mesh.sync_budget` marker. The number is
/// deliberately conservative and may be raised only with measured browser
/// evidence (ADR-0061, dom-canvas exception).
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) const MESH_SYNC_MAX_TRIANGLES: usize = 200_000;

/// The stable refusal when a synchronous dom-canvas mesh job is over budget.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) const MESH_SYNC_BUDGET: &str = "mesh.sync_budget";

/// Refuse a dom-canvas mesh job whose frozen operands exceed the synchronous
/// budget.
///
/// Reads the rows in place (`geometry_counts` does not clone) because this runs
/// on the command thread. Oversized work is a named refusal before the kernel
/// is called, never a page that hangs until the browser kills it.
#[cfg(any(target_arch = "wasm32", test))]
fn sync_budget_refusal(plan: &MeshJobPlan) -> Option<CadError> {
    let mut vertices = 0usize;
    let mut triangles = 0usize;
    for operand in &plan.operands {
        let (operand_vertices, operand_triangles) = operand.geometry_counts();
        vertices = vertices.saturating_add(operand_vertices);
        triangles = triangles.saturating_add(operand_triangles);
    }
    if triangles <= MESH_SYNC_MAX_TRIANGLES {
        return None;
    }
    Some(CadError::Unsupported(format!(
        "{MESH_SYNC_BUDGET}: the selected meshes total {triangles} triangles \
         ({vertices} vertices); the synchronous dom-canvas kernel accepts at most \
         {MESH_SYNC_MAX_TRIANGLES} triangles. Split the operands or use a native carrier."
    )))
}

/// How the worker reached its terminal state.
enum Terminal {
    Product {
        mesh: MeshData,
        topology: MeshTopologyReport,
        repair: Option<MeshRepair>,
        diagnostics: Vec<Diagnostic>,
    },
    Failed(CadError),
}

fn next_generation() -> u64 {
    super::next_generation()
}

/// The stable prefix of the "worker stopped without a result" refusal.
pub const MESH_JOB_WORKER_PANICKED: &str = "mesh.worker_panicked";

fn worker_panicked() -> CadError {
    CadError::Invariant(format!(
        "{MESH_JOB_WORKER_PANICKED}: the mesh worker stopped without a result; the drawing is unchanged"
    ))
}

/// One running (or finished) background mesh job.
pub(crate) struct MeshJob {
    stamp: TaskStamp,
    cancel: CancellationToken,
    plan: Arc<MeshJobPlan>,
    result: Receiver<Terminal>,
    handle: Option<JoinHandle<()>>,
    consumed: bool,
}

impl MeshJob {
    pub(crate) fn plan(&self) -> &Arc<MeshJobPlan> {
        &self.plan
    }

    pub(crate) fn cancel(&self) {
        self.cancel.cancel();
    }

    fn try_take(&mut self, current: &TaskStamp) -> CadResult<Option<Terminal>> {
        if self.consumed {
            return Ok(None);
        }
        match self.result.try_recv() {
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                // A disconnect without a value means the worker panicked; it is
                // never reported as a user cancel. A requested cancel is only a
                // cancel when the thread really exited with a value.
                self.consumed = true;
                self.join()?;
                if self.cancel.is_cancelled() {
                    Err(CadError::Cancelled)
                } else {
                    Err(worker_panicked())
                }
            }
            Ok(terminal) => {
                self.consumed = true;
                self.join()?;
                if self.cancel.is_cancelled() {
                    return Err(CadError::Cancelled);
                }
                if current != &self.stamp {
                    return Err(CadError::StaleResult);
                }
                Ok(Some(terminal))
            }
        }
    }

    fn join(&mut self) -> CadResult<()> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        handle.join().map_err(|_| worker_panicked())
    }
}

impl Drop for MeshJob {
    fn drop(&mut self) {
        if !self.consumed {
            self.cancel();
        }
        if self.handle.as_ref().is_some_and(JoinHandle::is_finished) {
            let _ = self.join();
        }
    }
}

/// The terminal (or still-running) state of one manager poll.
#[derive(Debug)]
pub(crate) enum MeshJobTake {
    Idle,
    Running,
    Done {
        plan: Arc<MeshJobPlan>,
        mesh: MeshData,
        topology: MeshTopologyReport,
        /// Present for a repair product (`Partial`); `None` for a boolean.
        repair: Option<MeshRepair>,
        diagnostics: Vec<Diagnostic>,
    },
    Cancelled,
    Stale,
    Failed(CadError),
}

/// Owns the single current background mesh job for one controller.
pub(crate) struct MeshJobManager {
    current: Option<MeshJob>,
    generation: u64,
    document: DocumentId,
}

impl MeshJobManager {
    pub(crate) fn new(document: DocumentId) -> Self {
        MeshJobManager {
            current: None,
            generation: 0,
            document,
        }
    }

    pub(crate) fn current_stamp(&self) -> TaskStamp {
        TaskStamp::new(self.document, self.generation)
    }

    pub(crate) fn is_running(&self) -> bool {
        self.current.is_some()
    }

    /// Start a mesh job, superseding any running one.
    ///
    /// Native hosts run the frozen plan on a worker thread and keep the owner
    /// tick responsive. `dom-canvas` has no thread: the wasm build runs the same
    /// plan **inline** on the session thread, bounded by
    /// [`MESH_SYNC_MAX_TRIANGLES`] and the stable `mesh.sync_budget` refusal.
    /// That exception is deliberate (ADR-0061): there is no mid-call cancel and
    /// the page is blocked while the kernel runs.
    pub(crate) fn start(
        &mut self,
        plan: MeshJobPlan,
        modeler: Arc<dyn MeshModeler>,
    ) -> CadResult<TaskStamp> {
        #[cfg(target_arch = "wasm32")]
        {
            // Before superseding anything: an oversized synchronous job is
            // refused by name, leaving the previous job (if any) untouched.
            if let Some(error) = sync_budget_refusal(&plan) {
                return Err(error);
            }
            self.start_inline(plan, modeler)
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.cancel_current();
            self.document = plan.document;
            self.generation = next_generation();
            let stamp = self.current_stamp();

            let cancel = CancellationToken::default();
            let (result_tx, result_rx) = mpsc::channel();
            let worker_plan = Arc::new(plan);
            let worker_cancel = cancel.clone();
            let worker_stamp = stamp.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("yacr-mesh-job-{}", stamp.generation))
                .spawn({
                    let plan = worker_plan.clone();
                    move || {
                        let cancelled = {
                            let token = worker_cancel.clone();
                            move || token.is_cancelled()
                        };
                        let terminal = run_job(&plan, modeler.as_ref(), &worker_stamp, &cancelled);
                        let _ = result_tx.send(terminal);
                    }
                });
            let handle = match spawned {
                Ok(handle) => Some(handle),
                Err(error) => {
                    return Err(CadError::ResourceMissing(format!(
                        "mesh.worker_spawn_failed: could not start the mesh worker: {error}"
                    )));
                }
            };

            self.current = Some(MeshJob {
                stamp,
                cancel,
                plan: worker_plan,
                result: result_rx,
                handle,
                consumed: false,
            });
            Ok(self.current_stamp())
        }
    }

    /// Run one frozen plan on the current thread (`dom-canvas` / contract test).
    ///
    /// The terminal value goes into the same channel the worker path uses, so
    /// `poll` keeps the existing cancellation/staleness checks and the host keeps
    /// the one-transaction commit. There is no watcher thread: the kernel sees a
    /// cancellation token only before and after the call.
    #[cfg(any(target_arch = "wasm32", test))]
    fn start_inline(
        &mut self,
        plan: MeshJobPlan,
        modeler: Arc<dyn MeshModeler>,
    ) -> CadResult<TaskStamp> {
        self.cancel_current();
        self.document = plan.document;
        self.generation = next_generation();
        let stamp = self.current_stamp();

        let cancel = CancellationToken::default();
        let (result_tx, result_rx) = mpsc::channel();
        let worker_plan = Arc::new(plan);
        let worker_cancel = cancel.clone();
        let cancelled = move || worker_cancel.is_cancelled();
        let terminal = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_job(&worker_plan, modeler.as_ref(), &stamp, &cancelled)
        }))
        .unwrap_or_else(|_| Terminal::Failed(worker_panicked()));
        let _ = result_tx.send(terminal);

        self.current = Some(MeshJob {
            stamp,
            cancel,
            plan: worker_plan,
            result: result_rx,
            handle: None,
            consumed: false,
        });
        Ok(self.current_stamp())
    }

    pub(crate) fn poll(&mut self) -> MeshJobTake {
        let current = self.current_stamp();
        let Some(mut job) = self.current.take() else {
            return MeshJobTake::Idle;
        };
        let plan = job.plan().clone();
        match job.try_take(&current) {
            Ok(None) => {
                self.current = Some(job);
                MeshJobTake::Running
            }
            Ok(Some(Terminal::Product {
                mesh,
                topology,
                repair,
                diagnostics,
            })) => MeshJobTake::Done {
                plan,
                mesh,
                topology,
                repair,
                diagnostics,
            },
            Ok(Some(Terminal::Failed(error))) => MeshJobTake::Failed(error),
            Err(CadError::Cancelled) => MeshJobTake::Cancelled,
            Err(CadError::StaleResult) => MeshJobTake::Stale,
            Err(error) => MeshJobTake::Failed(error),
        }
    }

    pub(crate) fn request_cancel(&self) {
        if let Some(job) = self.current.as_ref() {
            job.cancel();
        }
    }

    pub(crate) fn cancel(&mut self) {
        self.cancel_current();
    }

    fn cancel_current(&mut self) {
        if let Some(job) = self.current.take() {
            job.cancel();
            drop(job);
        }
    }
}

/// Run one frozen plan through the mesh seam.
fn run_job(
    plan: &MeshJobPlan,
    modeler: &dyn MeshModeler,
    stamp: &TaskStamp,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Terminal {
    match plan.kind {
        MeshJobKind::Boolean(op) => run_boolean(plan, op, modeler, stamp, cancelled),
        // Trim is the same subtract fold; the commit keeps the cutters.
        MeshJobKind::Trim => run_boolean(plan, MeshBooleanOp::Subtract, modeler, stamp, cancelled),
        MeshJobKind::Repair { mode, rule } => {
            run_repair(plan, mode, rule, modeler, stamp, cancelled)
        }
    }
}

/// Fold a boolean left; `SUBTRACT` keeps the first operand as the minuend.
fn run_boolean(
    plan: &MeshJobPlan,
    op: MeshBooleanOp,
    modeler: &dyn MeshModeler,
    stamp: &TaskStamp,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Terminal {
    if plan.operands.len() < 2 {
        return Terminal::Failed(CadError::Invariant(
            "mesh.boolean_invalid_plan: a mesh boolean needs at least two operands".into(),
        ));
    }
    // Derive each operand's neutral view lazily on the worker: only the
    // accumulator and the current right operand are alive at once. The first
    // mesh is moved into the accumulator instead of cloned.
    let mut operands = plan.operands.iter();
    let Some(first) = operands.next() else {
        return Terminal::Failed(CadError::Invariant(
            "mesh.boolean_invalid_plan: a mesh boolean needs at least two operands".into(),
        ));
    };
    let Some(mut accumulated) = first.mesh() else {
        return Terminal::Failed(CadError::Invariant(
            "mesh.boolean_invalid_plan: a frozen operand is no longer a mesh".into(),
        ));
    };
    let mut diagnostics = Vec::new();
    for operand in operands {
        if cancelled() {
            return Terminal::Failed(CadError::Cancelled);
        }
        let Some(mesh) = operand.mesh() else {
            return Terminal::Failed(CadError::Invariant(
                "mesh.boolean_invalid_plan: a frozen operand is no longer a mesh".into(),
            ));
        };
        let request = MeshRequest {
            stamp: stamp.clone(),
            operation: MeshOperation::Boolean {
                op,
                left: &accumulated,
                right: &mesh,
            },
        };
        let result = match execute_mesh(modeler, &request, cancelled) {
            Ok(result) => result,
            Err(error) => return Terminal::Failed(error),
        };
        match result.outcome {
            MeshOutcome::Success {
                mesh,
                diagnostics: notes,
                ..
            } => {
                accumulated = mesh;
                diagnostics.extend(notes);
            }
            MeshOutcome::Partial { .. } => {
                // A boolean must not answer with a repair report; accepting the
                // mesh would hide a kernel contract violation.
                return Terminal::Failed(CadError::Invariant(
                    "mesh.boolean_unexpected_repair: the modeler answered a boolean with a repair \
                     product"
                        .into(),
                ));
            }
            MeshOutcome::Unsupported { reason } => {
                return Terminal::Failed(CadError::Unsupported(format!(
                    "{}: {}",
                    reason.code, reason.detail
                )))
            }
            MeshOutcome::Failed {
                diagnostics: failure,
            } => {
                return Terminal::Failed(CadError::Unsupported(format!(
                    "mesh.boolean_failed: {}",
                    failure_message(&failure)
                )))
            }
        }
    }
    if cancelled() {
        return Terminal::Failed(CadError::Cancelled);
    }
    finish_product(accumulated, None, diagnostics)
}

/// Repair one closed mesh. The seam's answer is `Partial` by contract, and the
/// product is still a mesh (never exact B-rep).
fn run_repair(
    plan: &MeshJobPlan,
    mode: MeshRepairMode,
    rule: MeshWindingRule,
    modeler: &dyn MeshModeler,
    stamp: &TaskStamp,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Terminal {
    let Some(operand) = plan.operands.first() else {
        return Terminal::Failed(CadError::Invariant(
            "mesh.repair_invalid_plan: a repair needs exactly one mesh".into(),
        ));
    };
    if plan.operands.len() != 1 {
        return Terminal::Failed(CadError::Invariant(
            "mesh.repair_invalid_plan: a repair needs exactly one mesh".into(),
        ));
    }
    if cancelled() {
        return Terminal::Failed(CadError::Cancelled);
    }
    let Some(mesh) = operand.mesh() else {
        return Terminal::Failed(CadError::Invariant(
            "mesh.repair_invalid_plan: a frozen operand is no longer a mesh".into(),
        ));
    };
    let request = MeshRequest {
        stamp: stamp.clone(),
        operation: MeshOperation::Repair {
            mesh: &mesh,
            mode,
            rule,
        },
    };
    let result = match execute_mesh(modeler, &request, cancelled) {
        Ok(result) => result,
        Err(error) => return Terminal::Failed(error),
    };
    match result.outcome {
        MeshOutcome::Partial {
            mesh,
            repair,
            diagnostics,
        } => finish_product(mesh, Some(repair), diagnostics),
        MeshOutcome::Success { mesh, .. } => {
            // The seam's contract says a repair is always `Partial`; if an
            // implementation disagrees, the mesh is still usable but the report
            // is absent, so the host must not claim what was repaired.
            finish_product(mesh, None, Vec::new())
        }
        MeshOutcome::Unsupported { reason } => Terminal::Failed(CadError::Unsupported(format!(
            "{}: {}",
            reason.code, reason.detail
        ))),
        MeshOutcome::Failed {
            diagnostics: failure,
        } => Terminal::Failed(CadError::Unsupported(format!(
            "mesh.repair_failed: {}",
            failure_message(&failure)
        ))),
    }
}

/// Enforce the ADR-0061 product gate: the returned mesh must be closed.
fn finish_product(
    mesh: MeshData,
    repair: Option<MeshRepair>,
    diagnostics: Vec<Diagnostic>,
) -> Terminal {
    let topology = cad_kernel_adapter::inspect_topology_welded(&mesh);
    if topology.topology != MeshTopology::ClosedManifold {
        return Terminal::Failed(CadError::Unsupported(format!(
            "{}: the mesh job produced a {} mesh ({} boundary, {} non-manifold \
             edge(s)); a stored mesh product must be a closed 2-manifold",
            codes::MESH_NOT_CLOSED,
            topology.topology.as_str(),
            topology.boundary_edges,
            topology.non_manifold_edges
        )));
    }
    Terminal::Product {
        mesh,
        topology,
        repair,
        diagnostics,
    }
}

fn failure_message(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .first()
        .map(|diagnostic| diagnostic.message.clone())
        .unwrap_or_else(|| "the modeler produced no mesh".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::DbObject;
    use cad_domain::{Mesh, ObjectId, Point3};

    fn plan_with_triangles(triangles: usize) -> MeshJobPlan {
        let entity = DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbSubDMesh".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Mesh(Mesh {
                vertices: vec![Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                }],
                triangles: vec![[0, 0, 0]; triangles],
                ..Default::default()
            }),
            draw_order: 0,
        };
        MeshJobPlan {
            document: DocumentId(1),
            kind: MeshJobKind::Repair {
                mode: MeshRepairMode::Orientation,
                rule: MeshWindingRule::Positive,
            },
            layer: LayerId(0),
            space: SpaceId::Model,
            draw_order: 0,
            operands: vec![MeshJobOperand::new(entity)],
        }
    }

    #[test]
    fn the_sync_budget_refuses_only_above_its_triangle_limit() {
        assert!(sync_budget_refusal(&plan_with_triangles(MESH_SYNC_MAX_TRIANGLES)).is_none());

        let over = plan_with_triangles(MESH_SYNC_MAX_TRIANGLES + 1);
        match sync_budget_refusal(&over) {
            Some(CadError::Unsupported(text)) => {
                assert!(text.contains(MESH_SYNC_BUDGET), "{text}");
                assert!(text.contains(&(MESH_SYNC_MAX_TRIANGLES + 1).to_string()));
                assert!(text.contains(&MESH_SYNC_MAX_TRIANGLES.to_string()));
            }
            other => panic!("expected a named sync-budget refusal, got {other:?}"),
        }
    }

    /// A modeler that always refuses, so the inline path can be exercised
    /// without a kernel (the terminal mapping is the contract under test).
    struct RefusingModeler;

    impl MeshModeler for RefusingModeler {
        fn registration(&self) -> cad_domain::Registration {
            cad_domain::Registration {
                type_key: "test.refusing-mesh".into(),
                version: 1,
                priority: 1,
                entity_types: Vec::new(),
                capabilities: Vec::new(),
            }
        }

        fn capabilities(&self) -> cad_kernel_adapter::MeshModelerCapabilities {
            cad_kernel_adapter::MeshModelerCapabilities::all()
        }

        fn execute(
            &self,
            request: &MeshRequest<'_>,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> CadResult<cad_kernel_adapter::MeshResult> {
            Ok(cad_kernel_adapter::MeshResult {
                stamp: request.stamp.clone(),
                outcome: MeshOutcome::Unsupported {
                    reason: cad_kernel_adapter::MeshUnsupportedReason {
                        code: "mesh.test_refusal",
                        operation: request.operation.family(),
                        detail: "the refusing modeler never computes".into(),
                    },
                },
            })
        }
    }

    #[test]
    fn the_inline_path_feeds_the_existing_poll_contract() {
        let mut manager = MeshJobManager::new(DocumentId(1));
        let stamp = manager
            .start_inline(plan_with_triangles(1), Arc::new(RefusingModeler))
            .expect("the inline path must accept a valid plan");
        assert_eq!(stamp, manager.current_stamp());

        match manager.poll() {
            MeshJobTake::Failed(CadError::Unsupported(text)) => {
                assert!(text.contains("mesh.test_refusal"), "{text}");
            }
            other => panic!("expected the modeler refusal through poll, got {other:?}"),
        }
    }
}
