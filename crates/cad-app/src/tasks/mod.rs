//! Host-agnostic background import worker (spec F01, §4.10/§8.3).
//!
//! Importing a DWG is the one long-running operation a viewer must not run on
//! the UI thread. This module owns the *decision* half of running it in the
//! background: exactly one job may be current, a superseded or cancelled job
//! must never publish its database, and progress is delivered as a real event
//! stream rather than polled state.
//!
//! It is deliberately dependency-light: a plain [`std::thread`], a
//! [`std::sync::mpsc`] progress channel and [`cad_platform::CancellationToken`].
//! No Tokio, no platform API, so it is exercisable on a bare Linux host. The
//! worker thread runs the synchronous [`cad_import_acadrust::Importer`]; a host
//! that has a real executor can drive the same [`ImportJob`] contract without
//! this default implementation (see `docs/import-async.md`).
//!
//! Invariants encoded here (all covered by tests):
//!
//! * **One current job.** Starting a new import supersedes any running one; the
//!   superseded job is cancelled and its eventual result is discarded.
//! * **Cancelled/superseded results never publish.** [`ImportJob::try_take`]
//!   returns `Err(CadError::StaleResult)` unless the finished job's
//!   [`TaskStamp`] still matches the manager's current stamp.
//! * **Cancellation is reported as `Cancelled`, not `StaleResult`.** A job the
//!   host explicitly cancelled yields `CadError::Cancelled`; a result that lost
//!   the race to a newer job yields `StaleResult`. Both discard the database.
//! * **Cancel is asynchronous and idempotent.** [`ImportJob::cancel`] flips a
//!   token the importer polls at phase boundaries; the thread still returns a
//!   terminal error.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;

use cad_domain::{CadError, CadResult, DocumentId, TaskStamp};
use cad_import_acadrust::{
    AcadrustImporter, ImportPhase, ImportProgress, ImportProgressSink, ImportRequest,
    ImportedDrawing, Importer,
};
use cad_platform::CancellationToken;

use crate::host::HostController;

/// How a job reached its terminal state.
enum Terminal {
    /// The import finished and produced a database.
    Imported(Box<ImportedDrawing>),
    /// The import failed with a real error (corrupt data, limits, ...).
    Failed(CadError),
}

/// Monotonic source of job generations shared by every manager in a process.
///
/// A generation alone is not an identity: the published result carries a full
/// [`TaskStamp`] (document + generation), and the manager compares the whole
/// stamp so a job for another document can never be accepted here.
static JOB_GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_generation() -> u64 {
    JOB_GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// A running (or finished) background import.
///
/// The progress receiver is drained by the host; completed work is joined once.
/// Dropping live work requests cancellation and detaches, never joining a parser
/// on the event loop. Native hosts keep one slot until terminal before reopening.
pub struct ImportJob {
    stamp: TaskStamp,
    cancel: CancellationToken,
    progress: Receiver<ImportProgress>,
    result: Receiver<Terminal>,
    handle: Option<JoinHandle<()>>,
    /// Set once the terminal result has been taken (or the result was discarded
    /// as stale), so a second `try_take` cannot resurrect it.
    consumed: bool,
}

impl ImportJob {
    /// The stamp the eventual result will carry.
    pub fn stamp(&self) -> &TaskStamp {
        &self.stamp
    }

    /// Ask the worker to stop at the next phase/batch boundary.
    ///
    /// Safe to call repeatedly and from any thread. The worker still yields a
    /// terminal `Cancelled` result; callers that cancel because they superseded
    /// the job typically discard it via [`ImportManager`].
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Whether the host has asked this job to stop.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Drain every progress event queued so far, in emission order.
    ///
    /// Never blocks. A disconnected channel (worker gone) simply yields no more
    /// events; the terminal state is delivered through [`ImportJob::try_take`].
    pub fn drain_progress(&self) -> Vec<ImportProgress> {
        let mut out = Vec::new();
        while let Ok(p) = self.progress.try_recv() {
            out.push(p);
        }
        out
    }

    /// Take the terminal result if the worker has finished.
    ///
    /// * `Ok(Some(imported))` — finished successfully *and still current*.
    /// * `Ok(None)` — still running.
    /// * `Err(CadError::Cancelled)` — the job was cancelled; nothing publishes.
    /// * `Err(CadError::StaleResult)` — a newer job is current; discard.
    /// * `Err(other)` — the import genuinely failed.
    pub fn try_take(&mut self, current: &TaskStamp) -> CadResult<Option<ImportedDrawing>> {
        if self.consumed {
            return Ok(None);
        }
        match self.result.try_recv() {
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                // The thread is gone but produced no terminal value: treat it as
                // a cancelled job, never as a silent success.
                self.consumed = true;
                self.join();
                Err(CadError::Cancelled)
            }
            Ok(terminal) => {
                self.consumed = true;
                self.join();
                if self.cancel.is_cancelled() {
                    return Err(CadError::Cancelled);
                }
                // Stale-result guard: the stamp must match the manager's current
                // stamp *and* the job's own stamp, so a result for a superseded
                // document/generation is never published (F01 "旧任务丢弃").
                if current != &self.stamp {
                    return Err(CadError::StaleResult);
                }
                match terminal {
                    Terminal::Imported(drawing) => Ok(Some(*drawing)),
                    Terminal::Failed(e) => Err(e),
                }
            }
        }
    }

    fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            // The worker has already produced its terminal value, so this never
            // blocks on live work.
            let _ = handle.join();
        }
    }
}

impl Drop for ImportJob {
    fn drop(&mut self) {
        if !self.consumed {
            self.cancel();
        }
        if self.handle.as_ref().is_some_and(JoinHandle::is_finished) {
            self.join();
        }
        // Abandoned work owns its immutable input and observes cancellation;
        // dropping the handle detaches rather than blocking the event loop.
    }
}

/// The phase-boundary progress sink the worker uses.
///
/// A closed receiver (host dropped the job) is not an error: the import simply
/// stops being observable. The worker still runs to a terminal state so
/// cancellation/publish guards stay deterministic.
fn worker_progress(sender: Sender<ImportProgress>) -> impl ImportProgressSink + Send + 'static {
    move |progress: ImportProgress| {
        let _ = sender.send(progress);
    }
}

/// Owns the single current background import for one host.
///
/// The manager enforces "one current job": starting another cancels the old and
/// advances the generation, so the old job's stamp can no longer match and its
/// result is discarded even if it slips past cancellation.
pub struct ImportManager {
    current: Option<ImportJob>,
    generation: u64,
    document: DocumentId,
}

impl ImportManager {
    /// A manager for `document`, with no job running.
    pub fn new(document: DocumentId) -> Self {
        ImportManager {
            current: None,
            generation: 0,
            document,
        }
    }
    /// The stamp the current job's result must carry to be published.
    pub fn current_stamp(&self) -> TaskStamp {
        TaskStamp::new(self.document, self.generation)
    }

    /// Whether a job is running (its terminal result not yet taken).
    pub fn is_running(&self) -> bool {
        self.current.is_some()
    }

    /// Start an import, superseding any running job.
    ///
    /// Returns the new job's stamp. The worker thread is detached from the UI
    /// thread; the host drains it through the returned [`ImportJob`].
    pub fn start(&mut self, request: ImportRequest) -> TaskStamp {
        self.cancel_current();
        self.generation = next_generation();
        let stamp = self.current_stamp();

        let cancel = CancellationToken::default();
        let (progress_tx, progress_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker_cancel = cancel.clone();
        let worker_stamp = stamp.clone();

        let handle = std::thread::Builder::new()
            .name(format!("yacr-import-{}", stamp.generation))
            .spawn(move || {
                let importer = AcadrustImporter::new();
                let cancelled = {
                    let token = worker_cancel.clone();
                    move || token.is_cancelled()
                };
                let sink = worker_progress(progress_tx);
                let terminal = match importer.import_with_progress(&request, &cancelled, &sink) {
                    Ok(drawing) => Terminal::Imported(Box::new(drawing)),
                    Err(e) => Terminal::Failed(e),
                };
                // The receiver may already be gone (superseded + dropped). That
                // is fine: the terminal value is simply lost, which is exactly
                // what "stale result discarded" means.
                let _ = result_tx.send(terminal);
                let _ = worker_stamp;
            })
            .expect("spawn import worker thread");

        self.current = Some(ImportJob {
            stamp: stamp.clone(),
            cancel,
            progress: progress_rx,
            result: result_rx,
            handle: Some(handle),
            consumed: false,
        });
        stamp
    }

    /// The current job, if any, for draining progress / taking the result.
    pub fn current(&mut self) -> Option<&mut ImportJob> {
        self.current.as_mut()
    }

    /// Cancel and drop the current job; its result can no longer be published.
    ///
    /// Unlike the cancellation token, this also releases the slot: no further
    /// poll can observe the job at all. Superseding uses this path.
    pub fn cancel(&mut self) {
        self.cancel_current();
    }

    /// Flip the cancellation token of the current job but keep its handle, so
    /// the host can still poll the terminal `Cancelled` result.
    pub fn request_cancel(&self) {
        if let Some(job) = self.current.as_ref() {
            job.cancel();
        }
    }

    /// Clear a job whose result was successfully published.
    ///
    /// Distinct from [`ImportManager::cancel`] only in intent: the job already
    /// reached its terminal state, so this just releases the slot without
    /// flipping cancellation (which would be misleading).
    pub fn finish(&mut self) {
        self.current = None;
    }

    fn cancel_current(&mut self) {
        if let Some(job) = self.current.take() {
            job.cancel();
            // Dropping live work detaches; cancellation prevents publication.
            drop(job);
        }
    }
}

/// How a background import reached its terminal state, for a UI.
///
/// This is a projection of [`AsyncOpenPoll`]'s terminal variants with the
/// payload a progress panel needs; it never invents a state the worker did not
/// report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportTerminal {
    /// The import finished, was current, and its document was published.
    /// `entities` is the real entity count of the published database.
    Opened { entities: usize },
    /// The job was cancelled or superseded; nothing was published.
    Cancelled,
    /// The import failed with the real importer error.
    Failed { error: CadError },
}

/// The stable machine key of an [`ImportPhase`] (e.g. `"entities"`).
///
/// This keeps phase labels in the UI catalog keyed by a locale-independent
/// string, so the UI layer never has to depend on the importer crate.
pub fn import_phase_key(phase: ImportPhase) -> &'static str {
    match phase {
        ImportPhase::Reading => "reading",
        ImportPhase::Parsing => "parsing",
        ImportPhase::Tables => "tables",
        ImportPhase::Entities => "entities",
        ImportPhase::Resolving => "resolving",
        ImportPhase::Finishing => "finishing",
    }
}

/// A UI-facing, locale-independent snapshot of the asynchronous open (F01).
///
/// This is the pollable form the host UI binds to: it carries only real
/// values, so a UI can render an indeterminate bar when `entities_total` is
/// `None` and omit a byte count that was never measured. A host that does not
/// use the snapshot path can keep driving [`AsyncOpenPoll`] directly.
///
/// Terminal states are retained (so a failure stays visible); a new
/// [`HostController::begin_async_open`] supersedes them, and a genuinely idle
/// controller reports `None` from [`HostController::async_open_snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportProgressSnapshot {
    /// Whether an import is still running (false once terminal).
    pub running: bool,
    /// Latest real phase, or `None` before the first tick is observed.
    ///
    /// Never guessed: a job that has not emitted a tick yet reports no phase
    /// rather than a fabricated `Reading`.
    pub phase: Option<ImportPhase>,
    /// Entities normalised/inserted so far (real; `0` before the first tick).
    pub entities_done: usize,
    /// Total entities expected, when the format exposes it. Stays `None`
    /// otherwise, so a UI must render an indeterminate bar.
    pub entities_total: Option<usize>,
    /// Source bytes known so far, only when actually measurable.
    pub bytes: Option<u64>,
    /// Whether a cancel request can still be issued (false once requested or
    /// once the job reached a terminal state).
    pub cancellable: bool,
    /// Terminal outcome, if the job has finished.
    pub terminal: Option<ImportTerminal>,
}

impl ImportProgressSnapshot {
    /// The stable machine key of the latest phase (e.g. `"entities"`), if any.
    ///
    /// Locale-independent, so a UI crate can map it to a catalog label without
    /// depending on the importer crate.
    pub fn phase_key(&self) -> Option<&'static str> {
        self.phase.map(import_phase_key)
    }

    /// A running snapshot before any progress tick has arrived.
    pub fn running() -> Self {
        ImportProgressSnapshot {
            running: true,
            phase: None,
            entities_done: 0,
            entities_total: None,
            bytes: None,
            cancellable: true,
            terminal: None,
        }
    }

    /// A running snapshot reflecting one real progress tick.
    pub fn from_progress(progress: &ImportProgress, cancellable: bool) -> Self {
        ImportProgressSnapshot {
            running: true,
            phase: Some(progress.phase),
            entities_done: progress.entities_done,
            entities_total: progress.entities_total,
            bytes: progress.bytes,
            cancellable,
            terminal: None,
        }
    }

    /// A terminal snapshot. `cancellable` is always false once terminal.
    pub fn terminal(terminal: ImportTerminal) -> Self {
        ImportProgressSnapshot {
            running: false,
            phase: None,
            entities_done: 0,
            entities_total: None,
            bytes: None,
            cancellable: false,
            terminal: Some(terminal),
        }
    }

    /// Project one [`AsyncOpenPoll`] result into a snapshot.
    ///
    /// `previous` supplies the running fields when a poll carries no new tick
    /// (so an observed phase/count is never lost or invented). `Idle` yields
    /// `None`: there is genuinely nothing asynchronous to show.
    ///
    /// This is the single mapping from the manager's poll to the UI snapshot,
    /// so the two cannot drift.
    pub fn from_poll(
        poll: &AsyncOpenPoll,
        previous: Option<&ImportProgressSnapshot>,
    ) -> Option<Self> {
        match poll {
            AsyncOpenPoll::Idle => None,
            AsyncOpenPoll::Running { progress, .. } => {
                let mut snapshot = previous
                    .filter(|snapshot| snapshot.running)
                    .cloned()
                    .unwrap_or_else(ImportProgressSnapshot::running);
                if let Some(last) = progress.last() {
                    snapshot.phase = Some(last.phase);
                    snapshot.entities_done = last.entities_done;
                    snapshot.entities_total = last.entities_total;
                    // A later tick without a byte count must not erase one that
                    // was already measured.
                    snapshot.bytes = last.bytes.or(snapshot.bytes);
                }
                Some(snapshot)
            }
            AsyncOpenPoll::Opened { opened, .. } => {
                Some(ImportProgressSnapshot::terminal(ImportTerminal::Opened {
                    entities: opened.entities,
                }))
            }
            AsyncOpenPoll::Cancelled { .. } => {
                Some(ImportProgressSnapshot::terminal(ImportTerminal::Cancelled))
            }
            AsyncOpenPoll::Failed { error, .. } => {
                Some(ImportProgressSnapshot::terminal(ImportTerminal::Failed {
                    error: error.clone(),
                }))
            }
        }
    }
}

/// The outcome of polling an asynchronous open on a [`HostController`].
#[derive(Debug)]
pub enum AsyncOpenPoll {
    /// No job is running.
    Idle,
    /// The job is still running; `progress` is every event since the last poll.
    Running {
        progress: Vec<ImportProgress>,
        stamp: TaskStamp,
    },
    /// The job finished, was current, and its document was published. `progress`
    /// carries the events drained on this final poll (including the finish tick).
    Opened {
        opened: Box<crate::host::OpenedDrawing>,
        progress: Vec<ImportProgress>,
    },
    /// The job was cancelled or superseded; nothing was published.
    Cancelled { progress: Vec<ImportProgress> },
    /// The import failed; the previous document is untouched.
    Failed {
        error: CadError,
        progress: Vec<ImportProgress>,
    },
}

impl HostController {
    /// Start importing `bytes` on a worker thread (spec F01).
    ///
    /// `label` is remembered for the status line and the document name hint once
    /// the result is applied. The current document is **not** touched until
    /// [`HostController::poll_async_open`] publishes a current result. Starting a
    /// second open supersedes and cancels the first, so only the latest document
    /// can be published.
    pub fn begin_async_open(&mut self, bytes: Arc<[u8]>, label: &str) -> TaskStamp {
        self.pending_open_guard = self.drawing();
        let request = ImportRequest {
            document: self.document_id,
            database: cad_domain::DatabaseId(1),
            bytes,
            limits: cad_import_acadrust::ImportLimits::default(),
            generation: self.session.generation,
        };
        self.pending_open_label = Some(label.to_string());
        // Reset the UI snapshot to a fresh, tick-less running state so a stale
        // terminal state can never be shown for the new job.
        self.async_open = Some(crate::tasks::ImportProgressSnapshot::running());
        self.import_manager.start(request)
    }

    /// Cancel the running asynchronous open, if any.
    ///
    /// Keeps the job handle so the caller can still observe the terminal
    /// [`AsyncOpenPoll::Cancelled`]; the database is never published.
    pub fn cancel_async_open(&mut self) {
        self.import_manager.request_cancel();
        self.pending_open_label = None;
        if let Some(snapshot) = self.async_open.as_mut() {
            if snapshot.running {
                // Cancellation was requested; the cancel affordance must not
                // claim it can be issued again.
                snapshot.cancellable = false;
            }
        }
    }

    /// Poll the running asynchronous open.
    ///
    /// Only a result whose [`TaskStamp`] matches the manager's current stamp is
    /// published; a cancelled or superseded job yields [`AsyncOpenPoll::Cancelled`]
    /// and the current document is left untouched (F01 "旧任务丢弃").
    pub fn poll_async_open(&mut self) -> AsyncOpenPoll {
        if let Some(drawing) = &self.pending_open_guard {
            let unchanged = self
                .drawing()
                .is_some_and(|current| Arc::ptr_eq(&current, drawing));
            if !unchanged {
                self.cancel_async_open();
            }
        }
        let current = self.import_manager.current_stamp();
        let Some(job) = self.import_manager.current() else {
            // Idle: keep any retained terminal snapshot (a failure stays
            // visible) rather than clearing it to nothing.
            return AsyncOpenPoll::Idle;
        };
        let requested_cancel = job.is_cancelled();
        let progress = job.drain_progress();
        let stamp = job.stamp().clone();
        let poll = match job.try_take(&current) {
            Ok(None) => AsyncOpenPoll::Running { progress, stamp },
            Ok(Some(imported)) => match self.publish_imported(imported, &stamp) {
                Ok(opened) => {
                    self.import_manager.finish();
                    AsyncOpenPoll::Opened {
                        opened: Box::new(opened),
                        progress,
                    }
                }
                Err(error) => {
                    self.import_manager.finish();
                    AsyncOpenPoll::Failed { error, progress }
                }
            },
            Err(CadError::Cancelled) => {
                self.pending_open_label = None;
                self.import_manager.finish();
                AsyncOpenPoll::Cancelled { progress }
            }
            Err(CadError::StaleResult) => {
                // A newer job owns the slot: discard this result silently.
                self.import_manager.finish();
                AsyncOpenPoll::Cancelled { progress }
            }
            Err(error) => {
                self.import_manager.finish();
                AsyncOpenPoll::Failed { error, progress }
            }
        };
        // Project the poll into the retained UI snapshot. `from_poll` reuses the
        // previous running fields when this poll carried no new tick; a
        // cancellation request is reflected so the panel cannot offer cancel
        // twice. Idle never reaches here, so a terminal state is retained.
        let previous = self.async_open.take();
        let mut updated = crate::tasks::ImportProgressSnapshot::from_poll(&poll, previous.as_ref());
        if requested_cancel {
            if let Some(snapshot) = updated.as_mut() {
                if snapshot.running {
                    snapshot.cancellable = false;
                }
            }
        }
        self.async_open = updated;
        if !matches!(poll, AsyncOpenPoll::Running { .. }) {
            self.pending_open_guard = None;
        }
        poll
    }
}

#[cfg(test)]
mod tests;
