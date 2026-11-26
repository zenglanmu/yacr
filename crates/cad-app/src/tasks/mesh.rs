//! P-MESH-1a/1b: one background job for mesh booleans and mesh repair.
//!
//! The shape mirrors [`super::solid::SolidBooleanManager`] exactly: one current
//! job per controller, a frozen plan, a worker thread, a live cancellation
//! token, and a terminal value that only publishes when its [`TaskStamp`] is
//! still current. The difference is what crosses the worker:
//!
//! * the kernel seam is [`cad_kernel_adapter::MeshModeler`] (never
//!   `SolidModeler`), so the result is always a triangle mesh;
//! * the product is **not** exact B-rep geometry and the host never stores it as
//!   `Body3d { Solid, Exact }` (ADR-0061 D4);
//! * a boolean and a repair share this one slot, so the two mesh operations can
//!   never run at once and a document replacement/close aborts both.
//!
//! The worker owns an immutable snapshot of every operand. The host re-checks
//! the exact rows (revision **and** full entity) before committing, so an edit
//! made while the job ran discards the result instead of deleting newer
//! geometry. A cancelled or superseded result never touches `cad-db`.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;

use cad_db::DbEntity;
use cad_domain::{
    CadError, CadResult, Diagnostic, DocumentId, EntityId, LayerId, Revision, SpaceId, TaskStamp,
};
use cad_kernel_adapter::{
    codes, execute_mesh, MeshBooleanOp, MeshData, MeshModeler, MeshOperation, MeshOutcome,
    MeshRepair, MeshRepairMode, MeshRequest, MeshTopology, MeshTopologyReport, MeshWindingRule,
};

use cad_platform::CancellationToken;

/// One operand as it existed when the mesh command started.
#[derive(Clone, Debug)]
pub(crate) struct MeshJobOperand {
    pub(crate) id: EntityId,
    pub(crate) revision: Revision,
    /// The neutral geometry handed to the worker.
    pub(crate) mesh: MeshData,
    /// The immutable row the host compares before publishing.
    pub(crate) snapshot: DbEntity,
}

impl MeshJobOperand {
    /// Snapshot one row. The caller has already validated that it is a mesh.
    pub(crate) fn new(entity: &DbEntity, mesh: MeshData) -> Self {
        MeshJobOperand {
            id: entity.id,
            revision: entity.object.revision,
            mesh,
            snapshot: entity.clone(),
        }
    }
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
}

impl MeshJobKind {
    /// The stable lower-case tag used in a diagnostic/code prefix.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            MeshJobKind::Boolean(op) => op.as_str(),
            MeshJobKind::Repair { mode, .. } => mode.as_str(),
        }
    }
}

/// Everything the worker needs, frozen before the job starts.
#[derive(Clone, Debug)]
pub(crate) struct MeshJobPlan {
    pub(crate) document: DocumentId,
    pub(crate) kind: MeshJobKind,
    pub(crate) layer: LayerId,
    pub(crate) space: SpaceId,
    pub(crate) draw_order: i64,
    /// A boolean carries two or more; a repair carries exactly one.
    pub(crate) operands: Vec<MeshJobOperand>,
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
    pub(crate) fn start(
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
        let worker_cancel = cancel.clone();
        let worker_stamp = stamp.clone();
        let worker_plan = Arc::new(plan);

        let handle = std::thread::Builder::new()
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
        let handle = match handle {
            Ok(handle) => handle,
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
            handle: Some(handle),
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
    let Some(first) = plan.operands.first() else {
        return Terminal::Failed(CadError::Invariant(
            "mesh.boolean_invalid_plan: a mesh boolean needs at least two operands".into(),
        ));
    };
    if plan.operands.len() < 2 {
        return Terminal::Failed(CadError::Invariant(
            "mesh.boolean_invalid_plan: a mesh boolean needs at least two operands".into(),
        ));
    }
    let mut accumulated = first.mesh.clone();
    let mut diagnostics = Vec::new();
    for operand in &plan.operands[1..] {
        if cancelled() {
            return Terminal::Failed(CadError::Cancelled);
        }
        let request = MeshRequest {
            stamp: stamp.clone(),
            operation: MeshOperation::Boolean {
                op,
                left: &accumulated,
                right: &operand.mesh,
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
    let request = MeshRequest {
        stamp: stamp.clone(),
        operation: MeshOperation::Repair {
            mesh: &operand.mesh,
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
