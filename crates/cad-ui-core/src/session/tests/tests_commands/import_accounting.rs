//! Import/open accounting: the import counter and its discriminators.

use super::super::super::*;
use super::super::test_support::*;
use super::*;

/// Queue one synthetic-DWG `Opened` event through the fake file source.
fn import_synthetic(files: &Rc<FakeFiles>, name: &str) {
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path(name),
        bytes: Arc::from(SYNTHETIC_DWG.to_vec().into_boxed_slice()),
    });
}

/// The render session's observed incoming database pointer: the discriminator
/// component that changed on **every** authoritative publish (an edit commit
/// swaps the drawing `Arc` and the host re-syncs it each tick). Used by tests to
/// prove a non-import publish cannot move the import count.
fn incoming_ptr(session: &CadSession) -> Option<usize> {
    session
        .core
        .render
        .incoming_document()
        .borrow()
        .as_ref()
        .map(|database| Arc::as_ptr(database) as usize)
}

#[test]
fn open_admission_does_not_claim_a_completed_import() {
    let files = Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    });
    let session = session(files.clone());
    session.command("OPEN").expect("OPEN is admitted");
    // A pending picker keeps the old document and increments nothing.
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.import_count, 0);
    assert!(status.document_label.is_some());
    assert!(status.entity_count.is_some());
}

#[test]
fn failed_open_keeps_the_old_document_and_zero_imports() {
    let files = Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    });
    let session = session(files.clone());
    session.command("OPEN").expect("OPEN is admitted");
    files.events.borrow_mut().push(FileEvent::Failed {
        purpose: cad_platform::FilePurpose::Open,
        reason: "read failed".into(),
    });
    session.tick().expect("tick");
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.import_count, 0, "a failed open is not an import");
    assert!(status.document_label.is_some(), "the old document remains");
    assert!(status.entity_count.is_some());
    assert_eq!(status.error.as_deref(), Some("read failed"));
}

#[test]
fn successful_synthetic_import_counts_once_and_reports_real_entities() {
    let files = Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    });
    let session = session(files.clone());
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("synthetic-four-lines.dwg"),
        bytes: Arc::from(SYNTHETIC_DWG.to_vec().into_boxed_slice()),
    });
    session.command("OPEN").expect("OPEN is admitted");
    session.tick().expect("tick");
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.import_count, 1, "one verified successful import");
    assert_eq!(
        status.entity_count,
        Some(4),
        "the synthetic DWG has 4 lines"
    );
    assert_eq!(
        status.document_label.as_deref(),
        Some("synthetic-four-lines.dwg")
    );
    assert!(status.camera.is_some());
    assert_eq!(status.error, None);
    // Navigation and repeated snapshots never re-count.
    let mut move_input = move_input(5.0, 6.0);
    move_input.sequence = 2;
    session.handle_input(&move_input).unwrap();
    assert_eq!(session.snapshot(None, None, 0).import_count, 1);
}

#[test]
fn the_same_file_imported_twice_counts_twice() {
    let files = Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    });
    let session = session(files.clone());
    import_synthetic(&files, "synthetic-four-lines.dwg");
    session.command("OPEN").expect("OPEN is admitted");
    session.tick().expect("tick");
    assert_eq!(session.snapshot(None, None, 0).import_count, 1);

    // The exact same file again is a second authoritative import transition: the
    // host replaced the drawing (fresh generation) even though the identity is
    // byte-identical, so the counter advances once more.
    import_synthetic(&files, "synthetic-four-lines.dwg");
    session.command("OPEN").expect("OPEN is admitted");
    session.tick().expect("tick");
    assert_eq!(
        session.snapshot(None, None, 0).import_count,
        2,
        "a second successful import counts again even for the same file"
    );
}

#[test]
fn successful_erase_commit_does_not_increment_the_import_count() {
    let files = Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    });
    let session = session(files.clone());
    import_synthetic(&files, "synthetic-four-lines.dwg");
    session.command("OPEN").expect("OPEN is admitted");
    session.tick().expect("tick");
    assert_eq!(
        session.snapshot(None, None, 0).import_count,
        1,
        "the import"
    );

    // Select exactly one entity and erase it: a real edit commit, no import.
    let reference = {
        let c = session.core.controller.borrow();
        let drawing = c.drawing().expect("the imported document");
        let entity = drawing.model_space().first().expect("an entity").id;
        cad_app::selection::model_ref(c.document_id, entity)
    };
    let incoming_before = incoming_ptr(&session);
    session
        .core
        .controller
        .borrow_mut()
        .session
        .selection
        .replace([reference]);
    session.command("ERASE").expect("ERASE is admitted");
    // The host re-syncs the replaced drawing on the next tick, exactly like the
    // real GPU host: the render session's incoming database pointer changes.
    session.tick().expect("tick");
    let incoming_after = incoming_ptr(&session);
    assert_ne!(
        incoming_before, incoming_after,
        "an edit commit must have replaced the render incoming database (the defect's trigger)"
    );
    assert_eq!(
        session.snapshot(None, None, 0).import_count,
        1,
        "a successful edit must not move the import count"
    );
}

#[test]
fn refused_edit_does_not_increment_the_import_count() {
    let files = Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    });
    let session = session(files.clone());
    import_synthetic(&files, "synthetic-four-lines.dwg");
    session.command("OPEN").expect("OPEN is admitted");
    session.tick().expect("tick");
    assert_eq!(session.snapshot(None, None, 0).import_count, 1);

    // ERASE with an empty selection is an explicit refusal that changes nothing.
    let error = session
        .command("ERASE")
        .expect_err("an empty ERASE is refused");
    assert!(error.contains("non-empty selection"), "{error}");
    session.tick().expect("tick");
    let status = session.snapshot(None, None, 0);
    assert_eq!(
        status.import_count, 1,
        "a refused edit must not move the import count"
    );
    assert!(status.error.is_some(), "the refusal stays a real error");
}

#[test]
fn new_blank_document_does_not_increment_through_the_import_path() {
    let files = Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    });
    let session = session(files.clone());
    import_synthetic(&files, "synthetic-four-lines.dwg");
    session.command("OPEN").expect("OPEN is admitted");
    session.tick().expect("tick");
    assert_eq!(session.snapshot(None, None, 0).import_count, 1);

    session.command("NEW").expect("NEW is admitted");
    session.tick().expect("tick");
    assert_eq!(
        session.snapshot(None, None, 0).import_count,
        1,
        "a new/blank document is not an import"
    );
    // The blank document cleared the report, so a later genuine import counts.
    import_synthetic(&files, "synthetic-four-lines.dwg");
    session.command("OPEN").expect("OPEN is admitted");
    session.tick().expect("tick");
    assert_eq!(
        session.snapshot(None, None, 0).import_count,
        2,
        "a genuine import after NEW counts again"
    );
}

#[test]
fn a_changed_render_incoming_alone_does_not_recount() {
    // Pins the exact defect: identical `(document generation, import identity)`
    // with a *different* render incoming database pointer must not count. The
    // pointer changes on every authoritative publish (the host re-syncs the
    // drawing each tick and an edit commit swaps the `Arc`), so keying the import
    // discriminator on it made any edit produce a fresh key and a false
    // increment.
    let files = Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    });
    let session = session(files.clone());
    import_synthetic(&files, "synthetic-four-lines.dwg");
    session.command("OPEN").expect("OPEN is admitted");
    session.tick().expect("tick");
    let generation = session.core.request_context().document_generation;
    let identity = session
        .core
        .controller
        .borrow()
        .last_import_report
        .as_ref()
        .map(|report| report.identity.clone())
        .expect("a successful import left a report");
    let before = incoming_ptr(&session).expect("the render observed the drawing");

    // Swap the render incoming database for a distinct allocation while the
    // generation and identity stay put: exactly what an edit commit + tick does.
    let fresh = {
        let current = session
            .core
            .render
            .incoming_document()
            .borrow()
            .clone()
            .expect("incoming");
        Arc::new((*current).clone())
    };
    session.core.render.sync_drawing(Some(fresh));
    assert_ne!(
        incoming_ptr(&session),
        Some(before),
        "the incoming pointer must have changed"
    );
    assert_eq!(
        session.core.request_context().document_generation,
        generation,
        "the generation is unchanged (no import transition)"
    );
    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .last_import_report
            .as_ref()
            .map(|report| report.identity.clone()),
        Some(identity),
        "the import identity is unchanged"
    );
    assert_eq!(
        session.snapshot(None, None, 0).import_count,
        1,
        "a changed render incoming alone must not move the import count"
    );
}
