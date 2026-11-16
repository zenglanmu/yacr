//! ADR-0036 phase C2: a real **background** job for `UNION`/`SUBTRACT`/`INTERSECT`.
//!
//! Phase C1 ran a single primitive create synchronously on the command thread
//! and documented that as a bounded carve-out (one primitive, shape-relative
//! tessellation). A boolean is not bounded: the kernel walk grows with the
//! operand count and the geometry, so it must run on a worker thread with a live
//! cancellation probe (ADR-0036 P1/C1 explicitly forbids reusing the carve-out).
//!
//! The shape mirrors [`super::ImportJob`]:
//!
//! * exactly one job is current per controller (starting another supersedes the
//!   old one and advances the generation);
//! * a cancelled or superseded result never publishes ([`SolidBooleanJob`]
//!   compares the full [`TaskStamp`] and reports `CadError::StaleResult` /
//!   `CadError::Cancelled` instead of a body);
//! * the worker owns an immutable snapshot of the operands and returns only a
//!   terminal value — it never touches `cad-db`, the session or the renderer;
//! * dropping an unconsumed job requests cancellation and detaches.
//!
//! The plan is snapshotted (entity ids, revisions and exact B-reps) before the
//! thread starts, so the eventual result is validated against the state the
//! operands actually had when the user asked for the boolean. The host discards
//! it when any operand changed in the meantime (a *second* staleness gate on top
//! of the stamp: revisions catch an edit under the same document/generation).

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;

use cad_db::DbEntity;
use cad_domain::{
    Body3d, BrepData, CadError, CadResult, Diagnostic, DocumentId, EntityId, LayerId, Revision,
    SpaceId, TaskStamp,
};
use cad_kernel_adapter::{
    execute_solid, BooleanOp, SolidModeler, SolidOperation, SolidOutcome, SolidProduct,
    SolidRequest,
};

use cad_platform::CancellationToken;

/// One operand as it existed when the command started.
///
/// `brep` is the exact snapshot handed to the worker. `revision` and `snapshot`
/// are the staleness keys the host re-checks before publishing: this build's
/// write path does not advance a per-object `revision` on every edit, so the
/// full immutable row is compared as well (a moved/edited operand must discard
/// the result, not be deleted by it).
#[derive(Clone)]
pub(crate) struct SolidBooleanOperand {
    pub(crate) id: EntityId,
    pub(crate) revision: Revision,
    pub(crate) brep: BrepData,
    pub(crate) snapshot: DbEntity,
}

/// Everything the worker needs, frozen before the job starts.
#[derive(Clone)]
pub(crate) struct SolidBooleanPlan {
    pub(crate) document: DocumentId,
    pub(crate) op: BooleanOp,
    pub(crate) layer: LayerId,
    pub(crate) space: SpaceId,
    pub(crate) draw_order: i64,
    /// Two or more operands, in selection order. `SUBTRACT` subtracts every
    /// later operand from the first; `UNION`/`INTERSECT` fold left.
    pub(crate) operands: Vec<SolidBooleanOperand>,
}

/// How the worker reached its terminal state.
enum Terminal {
    /// The folded result, tessellated into the display cache C1 builds.
    ///
    /// Boxed because a `Body3d` carries the exact B-rep; a job result is taken
    /// once and immediately published.
    Body {
        body: Box<Body3d>,
        diagnostics: Vec<Diagnostic>,
    },
    /// A real error: cancelled, unsupported operand, kernel failure, a
    /// tessellation refusal, ... Never an empty success.
    Failed(CadError),
}

/// Monotonic job generations, shared with the import manager in this process.
///
/// The counter is process-wide so a stamp can never alias across managers; the
/// stamped document still scopes acceptance to one controller.
fn next_generation() -> u64 {
    super::next_generation()
}

/// The stable prefix of the "worker stopped without a result" refusal.
///
/// It is a **panic**, never a user cancel: the host reports it through
/// `solid.boolean_failed` and deliberately never includes the panic payload.
pub const SOLID_BOOLEAN_WORKER_PANICKED: &str = "solid.boolean_worker_panicked";

fn worker_panicked() -> CadError {
    CadError::Invariant(format!(
        "{SOLID_BOOLEAN_WORKER_PANICKED}: the solid boolean worker stopped without a result; the drawing is unchanged"
    ))
}

/// One running (or finished) background boolean.
pub(crate) struct SolidBooleanJob {
    stamp: TaskStamp,
    cancel: CancellationToken,
    plan: Arc<SolidBooleanPlan>,
    result: Receiver<Terminal>,
    handle: Option<JoinHandle<()>>,
    /// Set once the terminal result has been taken (or discarded), so a second
    /// poll cannot resurrect it.
    consumed: bool,
}

impl SolidBooleanJob {
    /// The frozen plan (operands, target layer/space/draw order).
    pub(crate) fn plan(&self) -> &Arc<SolidBooleanPlan> {
        &self.plan
    }

    /// Ask the worker to stop at the next operation boundary.
    pub(crate) fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Take the terminal result if the worker has finished.
    ///
    /// * `Ok(None)` — still running.
    /// * `Ok(Some((body, diagnostics)))` — finished, still current, usable.
    /// * `Err(Cancelled)` — the host cancelled it; nothing publishes.
    /// * `Err(StaleResult)` — a newer job owns the slot; discard.
    /// * `Err(other)` — the boolean genuinely failed.
    fn try_take(
        &mut self,
        current: &TaskStamp,
    ) -> CadResult<Option<(Box<Body3d>, Vec<Diagnostic>)>> {
        if self.consumed {
            return Ok(None);
        }
        match self.result.try_recv() {
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                // The sender is dropped without a value: the worker panicked (or
                // was aborted). A panic is **never** reported as a cancel, and a
                // requested cancel is only honoured when the thread really exited
                // cleanly.
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
                // A panic *after* the send is still a panic: never publish a body
                // from a worker that did not exit cleanly.
                self.join()?;
                if self.cancel.is_cancelled() {
                    return Err(CadError::Cancelled);
                }
                if current != &self.stamp {
                    return Err(CadError::StaleResult);
                }
                match terminal {
                    Terminal::Body { body, diagnostics } => Ok(Some((body, diagnostics))),
                    Terminal::Failed(error) => Err(error),
                }
            }
        }
    }

    /// Join the worker, mapping a panic to [`worker_panicked`].
    ///
    /// The panic payload is dropped here: it may carry internal data and the
    /// host reports a stable code instead.
    fn join(&mut self) -> CadResult<()> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        handle.join().map_err(|_| worker_panicked())
    }
}

impl Drop for SolidBooleanJob {
    fn drop(&mut self) {
        if !self.consumed {
            self.cancel();
        }
        if self.handle.as_ref().is_some_and(JoinHandle::is_finished) {
            // A finished-but-unconsumed job may have panicked; the result is
            // already unrunnable, so the join result carries no information.
            let _ = self.join();
        }
        // Live work is detached: it observes the token and its result is lost.
    }
}

/// The terminal (or still-running) state of one manager poll.
pub(crate) enum SolidBooleanTake {
    /// No job is current.
    Idle,
    /// The job is still running.
    Running,
    /// The job finished, was current, and produced a body. The caller still
    /// re-validates the operand snapshots before committing.
    Done {
        plan: Arc<SolidBooleanPlan>,
        body: Box<Body3d>,
        diagnostics: Vec<Diagnostic>,
    },
    /// The host cancelled it; nothing may publish.
    Cancelled,
    /// A newer job superseded it; nothing may publish.
    Stale,
    /// The boolean failed (unsupported operand, kernel failure, tessellation).
    Failed(CadError),
}

/// Owns the single current background boolean for one controller.
pub(crate) struct SolidBooleanManager {
    current: Option<SolidBooleanJob>,
    generation: u64,
    document: DocumentId,
}

impl SolidBooleanManager {
    /// A manager with no job running. The document is adopted from the first
    /// plan (`start`), because an `Application` exists before its document does.
    pub(crate) fn new(document: DocumentId) -> Self {
        SolidBooleanManager {
            current: None,
            generation: 0,
            document,
        }
    }

    /// The stamp the current job's result must carry to be published.
    pub(crate) fn current_stamp(&self) -> TaskStamp {
        TaskStamp::new(self.document, self.generation)
    }

    /// Whether a boolean is running (its terminal result not yet taken).
    ///
    /// The primitive handler refuses while this is true: both sides of the seam
    /// share one kernel mutex, so a synchronous C1 create would block behind a
    /// running boolean instead of failing fast.
    pub(crate) fn is_running(&self) -> bool {
        self.current.is_some()
    }

    /// Start a boolean, superseding any running job.
    ///
    /// The caller has already validated the operands and refuses before this
    /// point; the worker only executes the frozen plan.
    pub(crate) fn start(
        &mut self,
        plan: SolidBooleanPlan,
        modeler: Arc<dyn SolidModeler>,
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
            .name(format!("yacr-solid-boolean-{}", stamp.generation))
            .spawn({
                let plan = worker_plan.clone();
                move || {
                    let cancelled = {
                        let token = worker_cancel.clone();
                        move || token.is_cancelled()
                    };
                    let terminal = run_boolean(&plan, modeler.as_ref(), &worker_stamp, &cancelled);
                    // The receiver may already be gone (superseded + dropped):
                    // the terminal value is then simply lost.
                    let _ = result_tx.send(terminal);
                }
            });
        let handle = match handle {
            Ok(handle) => handle,
            Err(error) => {
                // The plan was not started: report it instead of fabricating a
                // job slot (the manager stays empty, so a later poll is Idle).
                return Err(CadError::ResourceMissing(format!(
                    "solid.boolean_spawn_failed: could not start the boolean worker: {error}"
                )));
            }
        };

        self.current = Some(SolidBooleanJob {
            stamp,
            cancel,
            plan: worker_plan,
            result: result_rx,
            handle: Some(handle),
            consumed: false,
        });
        Ok(self.current_stamp())
    }

    /// Take the current job's terminal state, releasing the slot when it is
    /// terminal. Never commits anything: publishing is the host's job.
    pub(crate) fn poll(&mut self) -> SolidBooleanTake {
        let current = self.current_stamp();
        let Some(mut job) = self.current.take() else {
            return SolidBooleanTake::Idle;
        };
        let plan = job.plan().clone();
        match job.try_take(&current) {
            Ok(None) => {
                self.current = Some(job);
                SolidBooleanTake::Running
            }
            Ok(Some((body, diagnostics))) => SolidBooleanTake::Done {
                plan,
                body,
                diagnostics,
            },
            Err(CadError::Cancelled) => SolidBooleanTake::Cancelled,
            Err(CadError::StaleResult) => SolidBooleanTake::Stale,
            Err(error) => SolidBooleanTake::Failed(error),
        }
    }

    /// Flip the cancellation token but keep the slot, so the host can still
    /// observe the terminal `Cancelled` state.
    pub(crate) fn request_cancel(&self) {
        if let Some(job) = self.current.as_ref() {
            job.cancel();
        }
    }

    /// Cancel and release the slot: no later poll can publish this job.
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

/// Fold the operands through the kernel, then tessellate the final B-rep.
///
/// `UNION`/`INTERSECT` fold left (`((a op b) op c)`); `SUBTRACT` takes the first
/// operand as the minuend and subtracts every later one. Every kernel answer is
/// mapped exactly like C1: `Unsupported`/`Failed` become a named `Unsupported`
/// refusal on the job's terminal, never an empty success.
fn run_boolean(
    plan: &SolidBooleanPlan,
    modeler: &dyn SolidModeler,
    stamp: &TaskStamp,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Terminal {
    debug_assert!(
        plan.operands.len() >= 2,
        "a boolean plan always carries at least two operands"
    );
    // Release guard: a one-operand plan would otherwise return operand 0's
    // B-rep as a "result", delete the operand and insert a copy (a silent data
    // change). Both command-layer validators already require two, so this is
    // defensive, but it is the last shape that could fake a success.
    let Some(first) = plan.operands.first() else {
        return Terminal::Failed(CadError::Invariant(
            "solid.boolean_invalid_plan: a boolean needs at least two operands".into(),
        ));
    };
    if plan.operands.len() < 2 {
        return Terminal::Failed(CadError::Invariant(
            "solid.boolean_invalid_plan: a boolean needs at least two operands".into(),
        ));
    }
    let mut accumulated = first.brep.clone();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    for operand in &plan.operands[1..] {
        if cancelled() {
            return Terminal::Failed(CadError::Cancelled);
        }
        let request = SolidRequest {
            stamp: stamp.clone(),
            operation: SolidOperation::Boolean {
                op: plan.op,
                left: &accumulated,
                right: &operand.brep,
            },
        };
        let result = match execute_solid(modeler, &request, cancelled) {
            Ok(result) => result,
            Err(error) => return Terminal::Failed(error),
        };
        match result.outcome {
            SolidOutcome::Success(SolidProduct::Body {
                brep,
                diagnostics: notes,
            }) => {
                accumulated = brep;
                diagnostics.extend(notes);
            }
            SolidOutcome::Success(SolidProduct::Mass(_)) => {
                return Terminal::Failed(CadError::Invariant(
                    "solid.boolean_unexpected_mass: the modeler answered a boolean with mass \
                     properties"
                        .into(),
                ))
            }
            SolidOutcome::Unsupported { reason } => {
                return Terminal::Failed(CadError::Unsupported(format!(
                    "{}: {}",
                    reason.code, reason.detail
                )))
            }
            SolidOutcome::Failed {
                diagnostics: failure,
            } => {
                return Terminal::Failed(CadError::Unsupported(format!(
                    "solid.kernel_failed: {}",
                    crate::app_drawing::solids::kernel_failure_message(&failure)
                )))
            }
        }
    }
    if cancelled() {
        return Terminal::Failed(CadError::Cancelled);
    }
    match crate::app_drawing::solids::solid_body(accumulated) {
        Ok(body) => Terminal::Body {
            body: Box::new(body),
            diagnostics,
        },
        Err(error) => Terminal::Failed(error),
    }
}
