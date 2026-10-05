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
/// string, so `cad-ui-slint` never has to depend on the importer crate.
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
/// This is the pollable form the Slint shell binds to: it carries only real
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
mod tests {
    use super::*;
    use cad_domain::DocumentId;
    use std::time::{Duration, Instant};

    /// A synthetic, writer-produced AC1032 DWG with four LINE entities. It is a
    /// committed contract fixture, not a vendor sample (see `fixtures/manifest`).
    const SYNTHETIC_DWG: &[u8] = include_bytes!("../../../fixtures/dwg/synthetic-four-lines.dwg");

    fn dwg_bytes() -> Arc<[u8]> {
        Arc::from(SYNTHETIC_DWG.to_vec().into_boxed_slice())
    }

    /// Poll until the job leaves `Running`, or fail after a short deadline.
    ///
    /// The worker parses ~20 KB of synthetic DWG, so this terminates quickly;
    /// the deadline only protects against a hung import.
    fn poll_until_terminal(controller: &mut HostController) -> AsyncOpenPoll {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match controller.poll_async_open() {
                AsyncOpenPoll::Running { .. } => {
                    assert!(Instant::now() < deadline, "import worker did not finish");
                    std::thread::sleep(Duration::from_millis(5));
                }
                other => return other,
            }
        }
    }

    /// Collect every progress event until the job leaves `Running`.
    ///
    /// Returns the accumulated events and whether the job published.
    fn collect_progress(controller: &mut HostController) -> (Vec<ImportProgress>, bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut all = Vec::new();
        loop {
            match controller.poll_async_open() {
                AsyncOpenPoll::Running { progress, .. } => {
                    all.extend(progress);
                    assert!(Instant::now() < deadline, "import worker did not finish");
                    std::thread::sleep(Duration::from_millis(5));
                }
                AsyncOpenPoll::Opened { progress, .. } => {
                    all.extend(progress);
                    return (all, true);
                }
                AsyncOpenPoll::Cancelled { progress } | AsyncOpenPoll::Failed { progress, .. } => {
                    all.extend(progress);
                    return (all, false);
                }
                AsyncOpenPoll::Idle => return (all, false),
            }
        }
    }

    #[test]
    fn async_open_publishes_a_real_synthetic_drawing() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller.begin_async_open(dwg_bytes(), "synthetic.dwg");
        match poll_until_terminal(&mut controller) {
            AsyncOpenPoll::Opened { opened, .. } => assert_eq!(opened.entities, 4),
            other => panic!("expected an opened drawing, got {other:?}"),
        }
        assert_eq!(controller.drawing().unwrap().entity_count(), 4);
        assert_eq!(controller.document_name_hint, "synthetic.dwg");
        assert!(!controller.application.can_undo(&controller.document_id));
    }

    #[test]
    fn async_result_cannot_replace_content_opened_while_worker_was_running() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller.begin_async_open(dwg_bytes(), "background.dwg");
        controller.open_bytes(dwg_bytes(), "newer.dwg").unwrap();
        let current = controller.drawing().unwrap();
        assert!(matches!(
            poll_until_terminal(&mut controller),
            AsyncOpenPoll::Cancelled { .. }
        ));
        assert!(Arc::ptr_eq(&current, &controller.drawing().unwrap()));
        assert_eq!(controller.document_name_hint, "newer.dwg");
    }

    #[test]
    fn drawing_edits_while_importing_cancel_publication_and_preserve_edits() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let before = controller.drawing().unwrap();
        let entities_before = before.entity_count();
        controller.begin_async_open(dwg_bytes(), "background.dwg");
        controller
            .create_line(
                cad_domain::Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                cad_domain::Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .unwrap();
        assert!(matches!(
            poll_until_terminal(&mut controller),
            AsyncOpenPoll::Cancelled { .. }
        ));
        // The imported background drawing (4 entities) was never published; the
        // local drawing edit survives.
        assert_eq!(
            controller.drawing().unwrap().entity_count(),
            entities_before + 1
        );
        // The cancelled import never replaced the document identity.
        assert_eq!(controller.document_name_hint, "yacr-demo");
    }

    #[test]
    fn async_progress_stream_is_ordered_and_monotonic() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller.begin_async_open(dwg_bytes(), "synthetic.dwg");
        let (events, opened) = collect_progress(&mut controller);
        assert!(opened, "job must publish");
        assert!(!events.is_empty(), "a real import must emit progress");

        // Read start/end, tables, entity batches, resolve, finish — in order.
        let index =
            |phase: cad_import_acadrust::ImportPhase| events.iter().position(|p| p.phase == phase);
        use cad_import_acadrust::ImportPhase::*;
        assert_eq!(events.first().map(|p| p.phase), Some(Reading));
        assert_eq!(events.last().map(|p| p.phase), Some(Finishing));
        assert!(index(Parsing).unwrap() < index(Tables).unwrap());
        assert!(index(Tables).unwrap() < index(Entities).unwrap());
        assert!(index(Entities).unwrap() < index(Resolving).unwrap());
        assert!(index(Resolving).unwrap() < index(Finishing).unwrap());

        // Entity counts rise and end at the real total (four lines + extras).
        let mut last = 0usize;
        let mut total = None;
        for p in events.iter().filter(|p| p.phase == Entities) {
            assert!(p.entities_done >= last);
            last = p.entities_done;
            if let Some(t) = p.entities_total {
                assert!(p.entities_done <= t);
                total = Some(t);
            }
        }
        assert_eq!(
            total,
            Some(4),
            "the synthetic file has exactly four entities"
        );
        assert_eq!(last, 4);
    }

    #[test]
    fn cancel_discards_the_database_and_reports_cancelled() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let demo_id = controller.drawing().unwrap().id();
        controller.begin_async_open(dwg_bytes(), "cancelled.dwg");
        controller.cancel_async_open();
        match poll_until_terminal(&mut controller) {
            AsyncOpenPoll::Cancelled { .. } => {}
            other => panic!("a cancelled open must report Cancelled, got {other:?}"),
        }
        // The previous document is untouched: a cancelled job publishes nothing.
        assert_eq!(controller.drawing().unwrap().id(), demo_id);
        assert_eq!(controller.document_name_hint, "yacr-demo");
    }

    #[test]
    fn superseded_job_is_not_published() {
        // Start job A, cancel it, then start job B. Only B's result may be
        // applied; A must have reported Cancelled and left nothing behind.
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let demo_id = controller.drawing().unwrap().id();

        let stamp_a = controller.begin_async_open(dwg_bytes(), "job-a.dwg");
        controller.cancel_async_open();
        match poll_until_terminal(&mut controller) {
            AsyncOpenPoll::Cancelled { .. } => {}
            other => panic!("job A must report Cancelled, got {other:?}"),
        }
        assert_eq!(
            controller.drawing().unwrap().id(),
            demo_id,
            "job A must not have published"
        );

        // Job B is a distinct generation (its stamp differs from A's).
        let stamp_b = controller.begin_async_open(dwg_bytes(), "job-b.dwg");
        assert_ne!(stamp_a, stamp_b);
        match poll_until_terminal(&mut controller) {
            AsyncOpenPoll::Opened { .. } => {}
            other => panic!("job B must publish, got {other:?}"),
        }
        assert_eq!(controller.drawing().unwrap().entity_count(), 4);
        assert_eq!(controller.document_name_hint, "job-b.dwg");
    }

    #[test]
    fn stale_stamp_is_discarded_by_the_job_guard() {
        // Build a real finished job, then take it under a *different* current
        // stamp: the guard must refuse to hand back a database (StaleResult).
        let mut manager = ImportManager::new(DocumentId(1));
        let request = ImportRequest {
            document: DocumentId(1),
            database: cad_domain::DatabaseId(1),
            bytes: dwg_bytes(),
            limits: cad_import_acadrust::ImportLimits::default(),
            generation: 0,
        };
        manager.start(request);
        let deadline = Instant::now() + Duration::from_secs(30);
        let other = TaskStamp::new(DocumentId(1), 999);
        loop {
            let job = manager.current().expect("job present");
            let _ = job.drain_progress();
            match job.try_take(&other) {
                Ok(None) => {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(CadError::StaleResult) => break,
                Ok(Some(_)) => panic!("stale stamp must not yield a database"),
                Err(e) => panic!("expected StaleResult, got {e:?}"),
            }
        }
    }

    #[test]
    fn malformed_dwg_fails_without_replacing_the_document() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let demo_id = controller.drawing().unwrap().id();
        let garbage: Arc<[u8]> = Arc::from(vec![0u8, 1, 2, 3].into_boxed_slice());
        controller.begin_async_open(garbage, "bad.dwg");
        match poll_until_terminal(&mut controller) {
            AsyncOpenPoll::Failed {
                error: CadError::CorruptData(_),
                ..
            } => {}
            other => panic!("expected a corrupt-data failure, got {other:?}"),
        }
        assert_eq!(controller.drawing().unwrap().id(), demo_id);
        assert_eq!(controller.document_name_hint, "yacr-demo");
    }

    #[test]
    fn snapshot_is_none_when_idle_and_never_fabricates_a_phase() {
        let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        // Nothing started: genuinely idle, no fabricated snapshot.
        assert_eq!(controller.async_open_snapshot(), None);

        // Projecting an Idle poll yields nothing, and a running snapshot before
        // any tick reports no phase rather than a guessed one.
        assert_eq!(
            ImportProgressSnapshot::from_poll(&AsyncOpenPoll::Idle, None),
            None
        );
        let started = ImportProgressSnapshot::running();
        assert!(started.running);
        assert_eq!(started.phase, None);
        assert_eq!(started.entities_done, 0);
        assert_eq!(started.entities_total, None);
        assert_eq!(started.bytes, None);
        assert!(started.cancellable);
        assert_eq!(started.terminal, None);
    }

    #[test]
    fn snapshot_keeps_unknown_totals_unknown_and_real_totals_exact() {
        // Indeterminate: total unknown stays None, bytes unknown stays None.
        let indeterminate = ImportProgress::at(cad_import_acadrust::ImportPhase::Parsing, 0, None);
        let snapshot = ImportProgressSnapshot::from_progress(&indeterminate, true);
        assert_eq!(snapshot.entities_total, None);
        assert_eq!(snapshot.bytes, None);
        assert!(snapshot.running);

        // Determinate: the real total is carried verbatim.
        let determinate =
            ImportProgress::at(cad_import_acadrust::ImportPhase::Entities, 2, Some(4));
        let snapshot = ImportProgressSnapshot::from_progress(&determinate, true);
        assert_eq!(snapshot.entities_done, 2);
        assert_eq!(snapshot.entities_total, Some(4));
    }

    #[test]
    fn snapshot_projection_retains_running_fields_and_maps_terminals() {
        // A Running poll with no tick keeps the previous real phase/counts so a
        // phase is never lost or invented.
        let previous = ImportProgressSnapshot {
            running: true,
            phase: Some(cad_import_acadrust::ImportPhase::Entities),
            entities_done: 3,
            entities_total: Some(4),
            bytes: Some(123),
            cancellable: true,
            terminal: None,
        };
        let running = AsyncOpenPoll::Running {
            progress: Vec::new(),
            stamp: TaskStamp::new(DocumentId(1), 1),
        };
        let projected = ImportProgressSnapshot::from_poll(&running, Some(&previous)).unwrap();
        assert_eq!(
            projected.phase,
            Some(cad_import_acadrust::ImportPhase::Entities)
        );
        assert_eq!(projected.entities_done, 3);
        assert_eq!(projected.entities_total, Some(4));
        assert_eq!(projected.bytes, Some(123));
        assert!(projected.running);

        // A later tick without bytes must not erase an already-measured count.
        let tick = ImportProgress::at(cad_import_acadrust::ImportPhase::Entities, 4, Some(4));
        let projected = ImportProgressSnapshot::from_poll(
            &AsyncOpenPoll::Running {
                progress: vec![tick],
                stamp: TaskStamp::new(DocumentId(1), 1),
            },
            Some(&projected),
        )
        .unwrap();
        assert_eq!(projected.bytes, Some(123), "measured bytes are retained");

        // Terminals map to explicit, non-running snapshots.
        let opened = ImportProgressSnapshot::from_poll(
            &AsyncOpenPoll::Opened {
                opened: Box::new(crate::host::OpenedDrawing {
                    entities: 4,
                    completeness_label: "完整".into(),
                    diagnostics: Vec::new(),
                }),
                progress: Vec::new(),
            },
            None,
        )
        .unwrap();
        assert!(!opened.running);
        assert_eq!(
            opened.terminal,
            Some(ImportTerminal::Opened { entities: 4 })
        );
        assert!(!opened.cancellable);

        let cancelled =
            ImportProgressSnapshot::from_poll(&AsyncOpenPoll::Cancelled { progress: vec![] }, None)
                .unwrap();
        assert_eq!(cancelled.terminal, Some(ImportTerminal::Cancelled));

        let failed = ImportProgressSnapshot::from_poll(
            &AsyncOpenPoll::Failed {
                error: CadError::CorruptData("bad".into()),
                progress: vec![],
            },
            None,
        )
        .unwrap();
        assert_eq!(
            failed.terminal,
            Some(ImportTerminal::Failed {
                error: CadError::CorruptData("bad".into())
            })
        );
        assert!(!failed.cancellable);
    }

    #[test]
    fn controller_snapshot_tracks_a_real_async_open_to_its_terminal() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller.begin_async_open(dwg_bytes(), "synthetic.dwg");
        // A fresh running snapshot with no fabricated phase/total.
        let running = controller.async_open_snapshot().expect("running snapshot");
        assert!(running.running);
        assert_eq!(running.phase, None);
        assert_eq!(running.entities_total, None);
        assert!(running.cancellable);

        match poll_until_terminal(&mut controller) {
            AsyncOpenPoll::Opened { .. } => {}
            other => panic!("expected an opened drawing, got {other:?}"),
        }
        let terminal = controller.async_open_snapshot().expect("retained terminal");
        assert!(!terminal.running);
        assert_eq!(
            terminal.terminal,
            Some(ImportTerminal::Opened { entities: 4 })
        );
        assert!(!terminal.cancellable);
    }

    #[test]
    fn cancelling_via_the_command_path_flips_cancellable_and_reports_cancelled() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        controller.begin_async_open(dwg_bytes(), "cancelled.dwg");
        assert!(controller.async_open_snapshot().unwrap().cancellable);

        // `CancelLoading` routes to the host cancel, so the UI's cancel
        // affordance works through the ordinary command path.
        controller
            .execute(crate::Command {
                schema_version: 1,
                id: crate::CommandId::CancelLoading,
                document: controller.document_id,
                viewport: controller.viewport_id,
                payload: crate::CommandPayload::None,
            })
            .unwrap();
        let snapshot = controller.async_open_snapshot().unwrap();
        assert!(snapshot.running);
        assert!(!snapshot.cancellable, "cancel cannot be issued twice");

        match poll_until_terminal(&mut controller) {
            AsyncOpenPoll::Cancelled { .. } => {}
            other => panic!("expected Cancelled, got {other:?}"),
        }
        let terminal = controller.async_open_snapshot().unwrap();
        assert_eq!(terminal.terminal, Some(ImportTerminal::Cancelled));
    }

    #[test]
    fn failed_open_retains_an_explicit_terminal_snapshot() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let garbage: Arc<[u8]> = Arc::from(vec![0u8, 1, 2, 3].into_boxed_slice());
        controller.begin_async_open(garbage, "bad.dwg");
        match poll_until_terminal(&mut controller) {
            AsyncOpenPoll::Failed { .. } => {}
            other => panic!("expected Failed, got {other:?}"),
        }
        // The failure must stay explicit after the job leaves the manager.
        assert!(matches!(controller.poll_async_open(), AsyncOpenPoll::Idle));
        let terminal = controller.async_open_snapshot().unwrap();
        assert!(!terminal.running);
        assert!(matches!(
            terminal.terminal,
            Some(ImportTerminal::Failed {
                error: CadError::CorruptData(_)
            })
        ));
    }
}
