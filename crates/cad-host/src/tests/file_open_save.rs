use super::*;

// --- file contracts: open/save/plot -----------------------------------------

#[test]
fn a_synchronous_save_completes_inside_the_gesture() {
    let files = Rc::new(SyncSaveFiles::default());
    let core = core_with_caps(files.clone(), capabilities(false));
    // Save As DXF: the synchronous platform synthesizes the target in
    // `begin_save`, so the export must run inside the gesture.
    core.request_save().unwrap();
    assert!(
        !core.save_pending.get(),
        "the synchronous target is drained"
    );
    assert!(
        core.pending_save.borrow().is_none(),
        "a completed save leaves no pending interaction"
    );
    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "exactly one file is written");
    assert!(
        written[0].0.ends_with(".dxf"),
        "save names a .dxf target: {}",
        written[0].0
    );
    assert!(
        written[0].1.windows(7).any(|window| window == b"SECTION"),
        "the written save is an ASCII DXF document"
    );
    let status = core.last_status().expect("the save grades a status");
    let (key, args) = catalog(&status);
    assert_eq!(key, "save.exported");
    assert_eq!(args.get("path"), Some(&written[0].0));
    drop(written);

    // Plot on a document with no paper layout: the funnel still completes in the
    // gesture and grades the explicit no-layout failure, never picker-waiting.
    core.request_plot().unwrap();
    assert!(!core.save_pending.get());
    assert!(core.pending_save.borrow().is_none());
    let status = core.last_status().expect("plot grades a status");
    let (key, args) = catalog(&status);
    assert_eq!(key, "plot.export_failed");
    assert!(
        args.get("reason")
            .is_some_and(|reason| reason.contains("plot.no_layout")),
        "the graded reason names the real no-layout failure: {args:?}"
    );
}

#[test]
fn a_synchronous_plot_writes_real_vector_content() {
    // A committed fixture with one paper-space layout, so the vector plot has
    // real stroke geometry. Opening is synchronous on this fake platform.
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/plot/a4-layout.dwg");
    let bytes = std::fs::read(&fixture).expect("committed plot fixture");
    let files = Rc::new(SyncSaveFiles::default());
    let core = core_with_caps(files.clone(), capabilities(false));
    core.controller
        .borrow_mut()
        .open_bytes(Arc::from(bytes), "a4-layout.dwg")
        .unwrap();
    core.request_plot().unwrap();
    let written = files.written.borrow();
    assert_eq!(written.len(), 1);
    assert!(
        written[0].0.ends_with(".svg"),
        "the default plot format is svg: {}",
        written[0].0
    );
    assert!(
        written[0].1.windows(4).any(|window| window == b"<svg"),
        "the written plot is a real SVG document, not an empty file"
    );
    let has = |needle: &[u8]| {
        written[0]
            .1
            .windows(needle.len())
            .any(|window| window == needle)
    };
    assert!(
        has(b"<polyline") || has(b"<polygon") || has(b"<path"),
        "the written plot contains real stroke geometry, not just a prolog"
    );
}

#[test]
fn an_async_save_waits_for_the_picker_then_exports_on_the_tick() {
    // `FakeFiles::begin_save` is a no-op (a desktop dialog is asynchronous): the
    // core leaves the interaction pending and exports only once the platform
    // delivers the chosen target on a later drain.
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), false);
    core.request_save().unwrap();
    assert!(
        core.save_pending.get(),
        "an async picker leaves the interaction pending"
    );
    assert!(core.pending_save.borrow().is_none());
    files.events.borrow_mut().push(FileEvent::SaveTarget {
        purpose: SavePurpose::SaveDxf,
        target: FileTarget::from_path("/tmp/chosen.dxf"),
    });
    core.drain_files().unwrap();
    core.poll_pending_save().unwrap();
    assert!(!core.save_pending.get());
    assert!(core.pending_save.borrow().is_none());
    let status = core.last_status().expect("the export grades a status");
    let (key, args) = catalog(&status);
    assert_eq!(key, "save.exported");
    assert_eq!(args.get("path"), Some(&"/tmp/chosen.dxf".to_string()));
}

#[test]
fn unbacked_new_plot_save_are_refused_without_side_effects() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with_caps(
        files,
        HostCapabilities {
            open: true,
            new: false,
            save: false,
            plot: false,
            import_worker: false,
        },
    );
    // New must not replace the drawing.
    let before = core.controller.borrow().drawing();
    assert!(matches!(
        core.request_new().unwrap_err(),
        CadError::Unsupported(_)
    ));
    assert!(
        Arc::ptr_eq(
            &before.unwrap(),
            &core.controller.borrow().drawing().unwrap()
        ),
        "an unbacked New must not replace the document"
    );
    // Plot/Save must not queue a target that will never be exported.
    assert!(matches!(
        core.request_plot().unwrap_err(),
        CadError::Unsupported(_)
    ));
    assert!(matches!(
        core.request_save().unwrap_err(),
        CadError::Unsupported(_)
    ));
    assert!(core.pending_save.borrow().is_none());
    assert!(!core.save_pending.get());
}

#[test]
fn error_text_unwraps_unsupported_and_not_implemented() {
    assert_eq!(
        error_text(&CadError::Unsupported(
            "OFFSET supports LINE and CIRCLE only".into()
        )),
        "OFFSET supports LINE and CIRCLE only"
    );
    assert_eq!(
        error_text(&CadError::NotImplemented("web save")),
        "web save"
    );
    // Other variants keep their Display (Debug) form.
    assert_eq!(
        error_text(&CadError::InvalidInput("bad".into())),
        "InvalidInput(\"bad\")"
    );
}

// --- open funnel: cancel/supersession/async publish -------------------------

#[test]
fn cancelled_open_is_discarded_and_progress_is_cancellable_then_idle() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    core.begin_open_attempt();
    core.open_pending.set(true);

    // While the platform read runs, the drawer shows a running, cancellable job.
    let running = core.import_snapshot().expect("running snapshot");
    assert!(running.running && running.cancellable);

    // Cancel: the read is still shown, but the cancel affordance is disabled.
    core.cancel_loading();
    let cancelled = core.import_snapshot().expect("running snapshot");
    assert!(cancelled.running && !cancelled.cancellable);

    // The read completes *after* the cancel: the bytes must be discarded.
    let drawing_before = core.controller.borrow().drawing();
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/after-cancel.dwg"),
        bytes: Arc::from(vec![1u8, 2, 3, 4]),
    });
    core.drain_files().unwrap();

    assert!(
        !core.open_pending.get(),
        "the delivery clears the pending read"
    );
    assert_eq!(
        core.import_snapshot(),
        None,
        "nothing is pending once the cancelled read is discarded"
    );
    let drawing_after = core.controller.borrow().drawing();
    assert!(
        Arc::ptr_eq(&drawing_before.unwrap(), &drawing_after.unwrap()),
        "a cancelled open must not replace the document"
    );
    assert_eq!(
        files.source.borrow().as_ref(),
        None,
        "a cancelled open must not arm the source-overwrite guard"
    );
}

#[test]
fn delivered_open_retains_the_platform_target_until_publish() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    core.begin_open_attempt();
    core.open_pending.set(true);
    let target = FileTarget {
        display_name: "drawing.dwg".into(),
        opaque: "saf://handle/42".into(),
    };
    files.events.borrow_mut().push(FileEvent::Opened {
        target: target.clone(),
        bytes: Arc::from(vec![1u8, 2, 3]),
    });
    core.drain_files().unwrap();
    assert_eq!(core.open_target.borrow().as_ref(), Some(&target));
}

#[test]
fn failed_async_publish_keeps_the_current_document() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    let demo_id = core.controller.borrow().drawing().unwrap().id();
    core.begin_open_attempt();
    core.open_pending.set(true);
    // Invalid bytes: the background import must fail and publish nothing.
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/not-a-drawing.dwg"),
        bytes: Arc::from(vec![0u8, 1, 2, 3]),
    });
    core.drain_files().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while core.loading() {
        assert!(
            std::time::Instant::now() < deadline,
            "import worker did not finish"
        );
        core.poll_loading().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        core.controller.borrow().drawing().unwrap().id(),
        demo_id,
        "a failed import must keep the current document"
    );
    assert!(!core.presenting_open.get(), "presentation must be cleared");
}

#[test]
fn synchronous_delivered_open_arms_the_guard_with_the_platform_target() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), false);
    // Valid, importable bytes: round-trip the demo drawing through the lossy DXF
    // export so the synchronous install can succeed.
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    core.begin_open_attempt();
    core.open_pending.set(true);
    let target = FileTarget {
        display_name: "drawing.dxf".into(),
        opaque: "saf://handle/42".into(),
    };
    files.events.borrow_mut().push(FileEvent::Opened {
        target: target.clone(),
        bytes: Arc::from(bytes),
    });
    core.drain_files().unwrap();
    assert_eq!(
        files.source.borrow().as_ref(),
        Some(&target),
        "the guard arms with the platform's opaque handle, not the display name"
    );
}
