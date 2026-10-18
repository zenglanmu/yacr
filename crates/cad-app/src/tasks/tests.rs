use super::*;
use cad_domain::DocumentId;
use std::time::{Duration, Instant};

/// A synthetic, writer-produced AC1032 DWG with four LINE entities. It is a
/// committed contract fixture, not a vendor sample (see `fixtures/manifest`).
const SYNTHETIC_DWG: &[u8] = include_bytes!("../../../../fixtures/dwg/synthetic-four-lines.dwg");

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
    let determinate = ImportProgress::at(cad_import_acadrust::ImportPhase::Entities, 2, Some(4));
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
