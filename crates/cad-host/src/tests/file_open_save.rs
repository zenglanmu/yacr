use super::*;

use std::path::Path;

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
            "OFFSET supports LINE, CIRCLE and straight-segment LWPOLYLINE only".into()
        )),
        "OFFSET supports LINE, CIRCLE and straight-segment LWPOLYLINE only"
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

// --- external-reference wiring ----------------------------------------------

// Hand-written minimal AC1015 DXF (no vendor sample): one xref block named BASE
// with the xref flag (70 = 4) and path group 1, plus one model-space INSERT.
const HOST_XREF_DXF: &str = "\
0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n0\nENDSEC\n\
0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nBLOCK_RECORD\n5\n1\n100\nAcDbSymbolTable\n70\n1\n\
0\nBLOCK_RECORD\n5\n1F\n100\nAcDbSymbolTableRecord\n100\nAcDbBlockTableRecord\n2\n*Model_Space\n70\n0\n\
0\nBLOCK_RECORD\n5\n1B\n100\nAcDbSymbolTableRecord\n100\nAcDbBlockTableRecord\n2\nBASE\n70\n0\n\
0\nENDTAB\n0\nENDSEC\n\
0\nSECTION\n2\nBLOCKS\n\
0\nBLOCK\n5\n21\n100\nAcDbEntity\n8\n0\n100\nAcDbBlockBegin\n2\nBASE\n70\n4\n10\n0.0\n20\n0.0\n30\n0.0\n3\nBASE\n1\nbase.dxf\n\
0\nENDBLK\n5\n22\n100\nAcDbEntity\n8\n0\n100\nAcDbBlockEnd\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nINSERT\n5\n20\n100\nAcDbEntity\n8\n0\n100\nAcDbBlockReference\n2\nBASE\n10\n0.0\n20\n0.0\n30\n0.0\n\
0\nENDSEC\n0\nEOF\n";

// A minimal referenced drawing with a single model-space LINE.
const REFERENCED_LINE_DXF: &str = "\
0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n0\nENDSEC\n\
0\nSECTION\n2\nTABLES\n0\nENDSEC\n\
0\nSECTION\n2\nBLOCKS\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n5\n20\n100\nAcDbEntity\n8\n0\n100\nAcDbLine\n10\n0.0\n20\n0.0\n30\n0.0\n11\n1.0\n21\n0.0\n31\n0.0\n\
0\nENDSEC\n0\nEOF\n";

fn xref_temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "yacr-host-xref-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Write a host drawing and its sibling referenced file; returns the host path.
fn write_xref_pair(dir: &Path) -> PathBuf {
    std::fs::write(dir.join("base.dxf"), REFERENCED_LINE_DXF).expect("write referenced file");
    let host = dir.join("host.dxf");
    std::fs::write(&host, HOST_XREF_DXF).expect("write host drawing");
    host
}

fn reference_status(core: &HostCore) -> cad_db::ExternalReferenceStatus {
    core.controller
        .borrow()
        .drawing()
        .expect("a drawing is open")
        .external_reference("BASE")
        .expect("the reference is recorded")
        .status
}

#[test]
fn a_synchronous_open_resolves_xrefs_from_the_drawing_directory() {
    use cad_db::ExternalReferenceStatus;

    let dir = xref_temp_dir("sync");
    let host = write_xref_pair(&dir);
    let bytes: Arc<[u8]> = Arc::from(HOST_XREF_DXF.as_bytes().to_vec().into_boxed_slice());

    let files = Rc::new(FakeFiles::default());
    let core = core_with(files, false);
    core.deliver_external_open(FileTarget::from_path(&host.to_string_lossy()), bytes)
        .expect("the synchronous open succeeds");
    assert_eq!(reference_status(&core), ExternalReferenceStatus::Loaded);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_async_open_resolves_xrefs_from_the_drawing_directory() {
    use cad_db::ExternalReferenceStatus;

    let dir = xref_temp_dir("async");
    let host = write_xref_pair(&dir);
    let bytes: Arc<[u8]> = Arc::from(HOST_XREF_DXF.as_bytes().to_vec().into_boxed_slice());

    let files = Rc::new(FakeFiles::default());
    let core = core_with(files, true);
    core.deliver_external_open(FileTarget::from_path(&host.to_string_lossy()), bytes)
        .expect("the async open starts");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    // Wait for the import job itself (not the render presentation): the fake
    // render session never becomes "current", so `loading()` could keep a
    // presentation flag set even after the database publishes.
    while core
        .controller
        .borrow()
        .async_open_snapshot()
        .is_some_and(|snapshot| snapshot.running)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "import worker did not finish"
        );
        core.poll_loading().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(reference_status(&core), ExternalReferenceStatus::Loaded);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_open_without_a_real_path_keeps_references_unavailable() {
    use cad_db::ExternalReferenceStatus;

    let bytes: Arc<[u8]> = Arc::from(HOST_XREF_DXF.as_bytes().to_vec().into_boxed_slice());
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files, false);
    core.deliver_external_open(
        FileTarget {
            display_name: "host.dxf".into(),
            opaque: String::new(),
        },
        bytes,
    )
    .expect("the path-less open succeeds");
    assert_eq!(
        reference_status(&core),
        ExternalReferenceStatus::BrowserUnavailable
    );
}

// --- DWG Save As: experimental export, new file, never the source -----------

/// A save platform whose chosen target (when set) is synthesized synchronously,
/// recording the suggested name/purpose and every write. `detect_identity`
/// models the platform's own device+inode / SAF-URI guard: the browser has no
/// such identity, so its save funnel must rely on the host's normalized backstop.
#[derive(Default)]
struct RecordingSaveFiles {
    calls: RefCell<Vec<(SavePurpose, String)>>,
    events: RefCell<Vec<FileEvent>>,
    written: RefCell<Vec<(String, Vec<u8>)>>,
    source: RefCell<Option<FileTarget>>,
    chosen: RefCell<Option<FileTarget>>,
    detect_identity: Cell<bool>,
}

impl RecordingSaveFiles {
    fn new() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            events: RefCell::new(Vec::new()),
            written: RefCell::new(Vec::new()),
            source: RefCell::new(None),
            chosen: RefCell::new(None),
            detect_identity: Cell::new(true),
        }
    }
}

impl PlatformFiles for RecordingSaveFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        self.calls
            .borrow_mut()
            .push((purpose, default_name.to_string()));
        if let Some(target) = self.chosen.borrow().clone() {
            self.events
                .borrow_mut()
                .push(FileEvent::SaveTarget { purpose, target });
        }
        Ok(())
    }
    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()> {
        self.written
            .borrow_mut()
            .push((target.display_name.clone(), bytes.to_vec()));
        Ok(())
    }
    fn refuses_source_overwrite(&self, target: &FileTarget) -> bool {
        self.detect_identity.get()
            && self
                .source
                .borrow()
                .as_ref()
                .is_some_and(|source| source.opaque == target.opaque)
    }
    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.borrow_mut() = target.cloned();
    }
    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

/// Open `path` from bytes produced by the DWG exporter (synthetic source).
fn open_dwg_source(core: &HostCore, path: &str) {
    let (bytes, _report) = core.controller.borrow().export_dwg().unwrap();
    core.deliver_external_open(FileTarget::from_path(path), Arc::from(bytes))
        .expect("the synthetic DWG source opens");
    assert_eq!(
        core.source_target
            .borrow()
            .as_ref()
            .map(|t| t.opaque.as_str()),
        Some(path),
        "the successful open arms the host-side source identity"
    );
}

#[test]
fn save_on_a_dwg_source_suggests_a_new_export_name() {
    let files = Rc::new(RecordingSaveFiles::new());
    let core = core_with_caps(files.clone(), capabilities(false));
    open_dwg_source(&core, "/tmp/plan.dwg");

    core.request_save().unwrap();
    assert!(
        core.save_pending.get(),
        "a desktop picker leaves the save pending"
    );
    let calls = files.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, SavePurpose::SaveDwg);
    assert_eq!(calls[0].1, "plan-export.dwg");
    assert_ne!(calls[0].1, "plan.dwg", "the source name is never suggested");
    let status = core.last_status().unwrap();
    let (key, _) = catalog(&status);
    assert_eq!(
        key, "save.picker_waiting_dwg",
        "the DWG picker must warn before the write that the copy is experimental and the source is kept"
    );
}

#[test]
fn a_dwg_target_writes_experimental_dwg_bytes_and_warns() {
    let files = Rc::new(RecordingSaveFiles::new());
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/plan-export.dwg"));
    let core = core_with_caps(files.clone(), capabilities(false));
    open_dwg_source(&core, "/tmp/plan.dwg");

    core.request_save().unwrap();
    assert!(!core.save_pending.get());
    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "exactly one new file is written");
    assert_eq!(written[0].0, "/tmp/plan-export.dwg");
    assert!(
        written[0].1.starts_with(b"AC10"),
        "the written file is DWG binary, not DXF text: {:?}",
        &written[0].1[..written[0].1.len().min(6)]
    );
    let status = core.last_status().expect("the export grades a status");
    let (key, args) = catalog(&status);
    assert!(
        key == "save.dwg_exported_experimental" || key == "save.dwg_exported_partial",
        "a DWG write always states the experimental boundary: {key}"
    );
    assert_eq!(args.get("path"), Some(&"/tmp/plan-export.dwg".to_string()));
    if key == "save.dwg_exported_partial" {
        assert!(args.contains_key("converted") && args.contains_key("dropped"));
    }
}

#[test]
fn choosing_the_opened_dwg_source_is_refused_and_writes_nothing() {
    for detect_identity in [true, false] {
        let files = Rc::new(RecordingSaveFiles::new());
        files.detect_identity.set(detect_identity);
        *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/plan.dwg"));
        let core = core_with_caps(files.clone(), capabilities(false));
        open_dwg_source(&core, "/tmp/plan.dwg");

        core.request_save().unwrap();
        assert!(
            files.written.borrow().is_empty(),
            "the opened source is never written (detect_identity={detect_identity})"
        );
        let status = core.last_status().unwrap();
        let (key, _) = catalog(&status);
        assert_eq!(
            key, "save.refused_source_path",
            "the refusal is named (detect_identity={detect_identity})"
        );
    }
}

#[test]
fn the_normalized_backstop_also_refuses_a_dot_path_spelling_of_the_source() {
    let files = Rc::new(RecordingSaveFiles::new());
    files.detect_identity.set(false);
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/./plan.dwg"));
    let core = core_with_caps(files.clone(), capabilities(false));
    open_dwg_source(&core, "/tmp/plan.dwg");

    core.request_save().unwrap();
    assert!(files.written.borrow().is_empty());
    let status = core.last_status().unwrap();
    let (key, _) = catalog(&status);
    assert_eq!(key, "save.refused_source_path");
}

#[test]
fn save_on_a_dxf_source_keeps_the_dxf_purpose_and_name() {
    let files = Rc::new(RecordingSaveFiles::new());
    let core = core_with_caps(files.clone(), capabilities(false));
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    core.deliver_external_open(FileTarget::from_path("/tmp/plan.dxf"), Arc::from(bytes))
        .expect("the synthetic DXF source opens");

    core.request_save().unwrap();
    let calls = files.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, SavePurpose::SaveDxf);
    assert_eq!(calls[0].1, "plan.dxf");
}
