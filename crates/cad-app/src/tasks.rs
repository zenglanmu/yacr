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
    AcadrustImporter, ImportProgress, ImportProgressSink, ImportRequest, ImportedDrawing, Importer,
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
/// The progress receiver is drained by the host; the worker thread is joined
/// once when the job is dropped or observed terminal. Dropping a still-running
/// job cancels it, so a host that abandons a document does not leak a thread
/// parsing for a document nobody wants.
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
        self.join();
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
            // Dropping joins the cancelled worker.
            drop(job);
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
        let request = ImportRequest {
            document: self.document_id,
            database: cad_domain::DatabaseId(1),
            bytes,
            limits: cad_import_acadrust::ImportLimits::default(),
            generation: self.session.generation,
        };
        self.pending_open_label = Some(label.to_string());
        self.import_manager.start(request)
    }

    /// Cancel the running asynchronous open, if any.
    ///
    /// Keeps the job handle so the caller can still observe the terminal
    /// [`AsyncOpenPoll::Cancelled`]; the database is never published.
    pub fn cancel_async_open(&mut self) {
        self.import_manager.request_cancel();
        self.pending_open_label = None;
    }

    /// Poll the running asynchronous open.
    ///
    /// Only a result whose [`TaskStamp`] matches the manager's current stamp is
    /// published; a cancelled or superseded job yields [`AsyncOpenPoll::Cancelled`]
    /// and the current document is left untouched (F01 "旧任务丢弃").
    pub fn poll_async_open(&mut self) -> AsyncOpenPoll {
        let current = self.import_manager.current_stamp();
        let Some(job) = self.import_manager.current() else {
            return AsyncOpenPoll::Idle;
        };
        let progress = job.drain_progress();
        let stamp = job.stamp().clone();
        match job.try_take(&current) {
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
        }
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
}
