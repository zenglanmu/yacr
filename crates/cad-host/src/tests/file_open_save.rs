use super::*;

use std::path::Path;

/// The document's in-memory compatibility table (`Document.compat`).
///
/// Save As appends the export report after a durable write; the next `.ymodel`
/// save persists these rows. Read through the same public fields the ymodel
/// tests use.
fn document_compat(core: &HostCore) -> Vec<serde_json::Value> {
    let controller = core.controller.borrow();
    controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .map(|document| document.compat.clone())
        .unwrap_or_default()
}

// --- file contracts: open/save/plot -----------------------------------------

#[test]
fn a_synchronous_export_completes_inside_the_gesture() {
    let files = Rc::new(SyncSaveFiles::default());
    let core = core_with_caps(files.clone(), capabilities(false));
    // A document opened from a `.dxf`: SAVE would be the native archive, so the
    // lossy DXF write is the explicit EXPORT command now (ADR-0041 D1/D2).
    *core.source_target.borrow_mut() = Some(FileTarget::from_path("/tmp/plan.dxf"));
    // EXPORT: the synchronous platform synthesizes the target in `begin_save`, so
    // the export must run inside the gesture.
    core.request_export().unwrap();
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
        "the export names a .dxf target: {}",
        written[0].0
    );
    assert!(
        written[0].1.windows(7).any(|window| window == b"SECTION"),
        "the written save is an ASCII DXF document"
    );
    let status = core.last_status().expect("the export grades a status");
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

/// EXPORT of a drawing that carries a `3DSOLID` states that the file is written as
/// AC1024, because the ACIS payload is inline SAT text. The choice is made in the
/// exporter (`export.note.acis_sat_text_version`) and surfaces on the export status
/// instead of staying buried in the report.
#[test]
fn exporting_a_drawing_with_a_3dsolid_says_the_file_is_written_as_ac1024() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/dxf/synthetic-acis-cube.dxf");
    let bytes = std::fs::read(&fixture).expect("committed ACIS DXF fixture");
    let files = Rc::new(SyncSaveFiles::default());
    let core = core_with_caps(files.clone(), capabilities(false));
    core.controller
        .borrow_mut()
        .open_bytes(Arc::from(bytes), "synthetic-acis-cube.dxf")
        .expect("the ACIS fixture opens");
    core.request_export().unwrap();

    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "exactly one file is written");
    let text = String::from_utf8_lossy(&written[0].1);
    assert!(
        text.contains("3DSOLID"),
        "the imported exact body is written back as a 3DSOLID"
    );
    assert!(
        text.contains("AC1024"),
        "the file is written as AC1024 so the payload stays inline SAT text"
    );
    assert!(
        !text.contains("ACDSDATA"),
        "no ACDSDATA SAB blob is written"
    );
    drop(written);

    let status = core.last_status().expect("the save grades a status");
    let (key, args) = catalog(&status);
    // The fixture's drawing is a saved DXF with its own conversions, so the
    // status may be the partial variant; both variants must name the AC1024
    // choice instead of the plain "Saved DXF" line.
    assert_eq!(
        key, "save.exported_acis_sat_partial",
        "the save status names the AC1024 choice"
    );
    assert!(
        args.get("path").is_some_and(|path| path.ends_with(".dxf")),
        "the status still names the written path: {args:?}"
    );

    // The DXF export went through the registry but the ACIS note still drives
    // the status, and the report itself is now recorded on the document.
    let compat = document_compat(&core);
    let record = compat.last().expect("the export report is recorded");
    assert_eq!(record["connector"], "dxf");
    assert_eq!(record["direction"], "export");
    assert!(
        record["entries"].as_array().is_some_and(|entries| entries
            .iter()
            .any(|entry| entry["code"] == "export.note.acis_sat_text_version")),
        "the recorded report keeps the ACIS note code: {record}"
    );
}

#[test]
fn exporting_a_drawing_with_a_3dsolid_to_dwg_says_the_payload_is_sab() {
    // The same exact ACIS fixture, written to a DWG target: the exporter carries
    // the body as a `3DSOLID` whose payload is a binary SAB blob in the AcDs
    // section (`export.note.acis_sab_dwg`), and the status names that payload
    // instead of the plain experimental-DWG line.
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/dxf/synthetic-acis-cube.dxf");
    let bytes = std::fs::read(&fixture).expect("committed ACIS DXF fixture");
    let files = Rc::new(RecordingSaveFiles::new());
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/acis-cube-export.dwg"));
    let core = core_with_caps(files.clone(), capabilities(false));
    core.controller
        .borrow_mut()
        .open_bytes(Arc::from(bytes), "synthetic-acis-cube.dxf")
        .expect("the ACIS fixture opens");
    core.request_export().unwrap();

    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "exactly one file is written");
    assert_eq!(written[0].0, "/tmp/acis-cube-export.dwg");
    assert!(
        written[0].1.starts_with(b"AC10"),
        "the written file is DWG binary, not DXF text: {:?}",
        &written[0].1[..written[0].1.len().min(6)]
    );
    assert!(
        !written[0].1.windows(7).any(|window| window == b"3DSOLID"),
        "a DWG carries the payload as binary SAB, not as a textual 3DSOLID record"
    );
    drop(written);

    let status = core.last_status().expect("the save grades a status");
    let (key, args) = catalog(&status);
    // The partial variant is the only reachable one: writing an ACIS `3DSOLID`
    // always records the body as Converted (its display cache is dropped), so the
    // report can never be Complete and the clean key must not appear.
    assert_eq!(
        key, "save.exported_acis_sab_partial",
        "the status names the DWG ACIS payload instead of the plain experimental line"
    );
    assert_ne!(
        key, "save.exported_acis_sab",
        "a written ACIS body always makes the export partial, so the clean SAB key is unreachable"
    );
    assert_eq!(
        args.get("path"),
        Some(&"/tmp/acis-cube-export.dwg".to_string())
    );
    assert!(
        args.contains_key("converted") && args.contains_key("dropped"),
        "a partial status still carries both counts: {args:?}"
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
fn an_async_export_waits_for_the_picker_then_exports_on_the_tick() {
    // `FakeFiles::begin_save` is a no-op (a desktop dialog is asynchronous): the
    // core leaves the interaction pending and exports only once the platform
    // delivers the chosen target on a later drain.
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), false);
    core.request_export().unwrap();
    assert!(
        core.save_pending.get(),
        "an async picker leaves the interaction pending"
    );
    assert!(core.pending_save.borrow().is_none());
    files.events.borrow_mut().push(FileEvent::SaveTarget {
        purpose: SavePurpose::SaveExport,
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
fn an_explicit_path_opens_through_the_platform_open_funnel() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), false);
    let outcome = core
        .open_path(Path::new("/tmp/plan.rvt"))
        .expect("the platform read starts");
    assert_eq!(outcome, crate::PathOpenOutcome::Started);
    let target = files
        .opened_target
        .borrow()
        .clone()
        .expect("begin_open_target saw the caller's own path");
    assert_eq!(target.opaque, "/tmp/plan.rvt");
    assert_eq!(target.display_name, "/tmp/plan.rvt");
}

#[test]
fn an_explicit_path_while_a_load_runs_is_busy_and_is_not_started() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), false);
    core.request_open().expect("the chooser starts");
    assert!(core.loading(), "the picker read is pending");
    assert_eq!(
        core.open_path(Path::new("/tmp/second.dwg"))
            .expect("busy is an answer, not an error"),
        crate::PathOpenOutcome::Busy
    );
    assert!(
        files.opened_target.borrow().is_none(),
        "a busy session must not start a second platform open"
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

// --- EXPORT: target extension selects the writer, overwriting is allowed -----

/// A save platform whose chosen target (when set) is synthesized synchronously,
/// recording the suggested name/purpose and every write.
#[derive(Default)]
struct RecordingSaveFiles {
    calls: RefCell<Vec<(SavePurpose, String)>>,
    events: RefCell<Vec<FileEvent>>,
    written: RefCell<Vec<(String, Vec<u8>)>>,
    chosen: RefCell<Option<FileTarget>>,
}

impl RecordingSaveFiles {
    fn new() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            events: RefCell::new(Vec::new()),
            written: RefCell::new(Vec::new()),
            chosen: RefCell::new(None),
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
    fn set_source(&self, _target: Option<&FileTarget>) {}
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
fn a_dwg_target_writes_experimental_dwg_bytes_and_warns() {
    let files = Rc::new(RecordingSaveFiles::new());
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/plan-export.dwg"));
    let core = core_with_caps(files.clone(), capabilities(false));
    open_dwg_source(&core, "/tmp/plan.dwg");

    core.request_export().unwrap();
    assert!(!core.save_pending.get());
    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "exactly one file is written");
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
fn exporting_over_the_opened_dwg_source_is_allowed() {
    // ADR-0041 D4: choosing the opened source as the export target is a
    // deliberate user action, not a refusal. The host writes it (the platform's
    // atomic write keeps the previous bytes in `<name>.<ext>.bak`; that half is
    // pinned by the Linux platform test) and grades the experimental DWG status.
    let files = Rc::new(RecordingSaveFiles::new());
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/plan.dwg"));
    let core = core_with_caps(files.clone(), capabilities(false));
    open_dwg_source(&core, "/tmp/plan.dwg");

    core.request_export().unwrap();
    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "the source target is written");
    assert_eq!(written[0].0, "/tmp/plan.dwg");
    assert!(
        written[0].1.starts_with(b"AC10"),
        "the overwritten target holds DWG bytes"
    );
    drop(written);
    let status = core.last_status().unwrap();
    let (key, _) = catalog(&status);
    assert!(
        key == "save.dwg_exported_experimental" || key == "save.dwg_exported_partial",
        "the DWG status still names the experimental boundary: {key}"
    );
}

#[test]
fn exporting_to_a_dot_path_spelling_of_the_source_is_allowed() {
    // The normalized-path backstop is gone with the refusal: a different lexical
    // spelling of the same file is an ordinary target now.
    let files = Rc::new(RecordingSaveFiles::new());
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/./plan.dwg"));
    let core = core_with_caps(files.clone(), capabilities(false));
    open_dwg_source(&core, "/tmp/plan.dwg");

    core.request_export().unwrap();
    assert_eq!(
        files.written.borrow().len(),
        1,
        "the target is written, not refused"
    );
}

#[test]
fn a_connector_save_as_writes_registry_bytes_and_grades_its_own_status() {
    // A mesh document (the committed STL fixture imports as one model-space
    // mesh) is written through the registry for both a neutral-mesh key and
    // IFC. The status must name the connector format instead of the DXF wording.
    for (extension, magic) in [("stl", None), ("ifc", Some(&b"ISO-10303-21;"[..]))] {
        let files = Rc::new(RecordingSaveFiles::new());
        let target = format!("/tmp/mesh-export.{extension}");
        *files.chosen.borrow_mut() = Some(FileTarget::from_path(&target));
        let core = core_with_caps(files.clone(), capabilities(false));
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/stl/synthetic-quad.stl");
        let bytes = std::fs::read(&fixture).expect("committed STL fixture");
        core.deliver_external_open(FileTarget::from_path("/tmp/source.stl"), Arc::from(bytes))
            .expect("the STL fixture imports as a mesh document");
        // IFC (and SKP) refuse to guess a source length unit; STL carries none.
        core.controller
            .borrow_mut()
            .set_units(cad_domain::UnitContext::from_meter())
            .unwrap();

        core.request_export().unwrap();
        assert!(
            !core.save_pending.get(),
            "{extension}: the target is drained"
        );
        let written = files.written.borrow();
        assert_eq!(written.len(), 1, "{extension}: exactly one file is written");
        assert_eq!(written[0].0, target);
        assert!(
            !written[0].1.is_empty(),
            "{extension}: a connector export wrote no bytes"
        );
        if let Some(magic) = magic {
            assert!(
                written[0].1.starts_with(magic),
                "{extension}: bytes must start with {magic:?}"
            );
        }
        drop(written);

        let status = core.last_status().expect("the export grades a status");
        let (key, args) = catalog(&status);
        assert!(
            key == "save.exported_connector" || key == "save.exported_connector_partial",
            "{extension}: the status must name the connector export, got {key}"
        );
        assert_eq!(args.get("path"), Some(&target));
        assert_eq!(
            args.get("format"),
            Some(&extension.to_string()),
            "{extension}: the status names the connector key"
        );
        if key == "save.exported_connector_partial" {
            assert!(args.contains_key("converted") && args.contains_key("dropped"));
        }
    }
}

#[test]
fn a_connector_save_records_its_export_report_on_the_document() {
    let files = Rc::new(RecordingSaveFiles::new());
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/mesh-export.stl"));
    let core = core_with_caps(files.clone(), capabilities(false));
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/stl/synthetic-quad.stl");
    let bytes = std::fs::read(&fixture).expect("committed STL fixture");
    core.deliver_external_open(FileTarget::from_path("/tmp/source.stl"), Arc::from(bytes))
        .expect("the STL fixture imports as a mesh document");
    let before = document_compat(&core).len();

    core.request_export().unwrap();
    assert_eq!(files.written.borrow().len(), 1);

    let compat = document_compat(&core);
    assert_eq!(
        compat.len(),
        before + 1,
        "a successful export appends exactly one compat record"
    );
    let record = compat.last().expect("the export record");
    assert_eq!(record["connector"], "stl");
    assert_eq!(record["direction"], "export");
}

/// A `.glb` Save As writes the **binary** glTF container of the one `gltf`
/// connector: the status names the registry key the user typed (`glb`), while the
/// recorded report names the descriptor (`gltf`) — the report id is not the key.
#[test]
fn a_glb_save_as_writes_the_gltf_connector_and_records_its_report() {
    let files = Rc::new(RecordingSaveFiles::new());
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/mesh-export.glb"));
    let core = core_with_caps(files.clone(), capabilities(false));
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/stl/synthetic-quad.stl");
    let bytes = std::fs::read(&fixture).expect("committed STL fixture");
    core.deliver_external_open(FileTarget::from_path("/tmp/source.stl"), Arc::from(bytes))
        .expect("the STL fixture imports as a mesh document");
    // glTF positions are metres, so the writer refuses an unknown unit.
    core.controller
        .borrow_mut()
        .set_units(cad_domain::UnitContext::from_meter())
        .unwrap();
    let before = document_compat(&core).len();

    core.request_export().unwrap();

    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "exactly one file is written");
    assert_eq!(written[0].0, "/tmp/mesh-export.glb");
    assert!(
        written[0].1.starts_with(b"glTF"),
        "a `.glb` target must write the binary glTF container"
    );
    drop(written);

    let status = core.last_status().expect("the export grades a status");
    let (key, args) = catalog(&status);
    assert!(
        key == "save.exported_connector" || key == "save.exported_connector_partial",
        "the status must name the connector export, got {key}"
    );
    assert_eq!(args.get("format"), Some(&"glb".to_string()));

    let compat = document_compat(&core);
    assert_eq!(
        compat.len(),
        before + 1,
        "a successful export appends exactly one compat record"
    );
    let record = compat.last().expect("the export record");
    assert_eq!(
        record["connector"], "gltf",
        "the report id is the descriptor id, not the registry key"
    );
    assert_eq!(record["direction"], "export");
}

/// The compat table is a bounded recent history: repeated exports must not grow
/// the document (and its next `.ymodel`) without bound.
#[test]
fn export_records_are_capped_to_the_newest_32() {
    let files = Rc::new(RecordingSaveFiles::new());
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/mesh-export.stl"));
    let core = core_with_caps(files.clone(), capabilities(false));
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/stl/synthetic-quad.stl");
    let bytes = std::fs::read(&fixture).expect("committed STL fixture");
    core.deliver_external_open(FileTarget::from_path("/tmp/source.stl"), Arc::from(bytes))
        .expect("the STL fixture imports as a mesh document");

    for _ in 0..40 {
        core.request_export().unwrap();
    }
    let compat = document_compat(&core);
    let is_export = |row: &serde_json::Value| row["direction"].as_str() == Some("export");
    let export_rows = compat.iter().filter(|row| is_export(row)).count();
    assert_eq!(export_rows, 32, "the table keeps the newest 32 export rows");
    // The import provenance record is not part of the export history and is
    // never evicted by later exports.
    assert_eq!(compat.iter().filter(|row| !is_export(row)).count(), 1);
    assert_eq!(compat.last().expect("a record")["connector"], "stl");
}

/// A document whose units are unresolved drawing units refuses IFC and SKP by
/// name: neither writer may assume a length, and nothing is written.
#[test]
fn drawing_units_refuse_ifc_and_skp_by_name_without_writing() {
    for (extension, code) in [
        ("ifc", "export.ifc.units_unknown"),
        ("skp", "export.skp.units_unknown"),
    ] {
        let files = Rc::new(RecordingSaveFiles::new());
        let target = format!("/tmp/units-export.{extension}");
        *files.chosen.borrow_mut() = Some(FileTarget::from_path(&target));
        let core = core_with_caps(files.clone(), capabilities(false));
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/stl/synthetic-quad.stl");
        let bytes = std::fs::read(&fixture).expect("committed STL fixture");
        core.deliver_external_open(FileTarget::from_path("/tmp/source.stl"), Arc::from(bytes))
            .expect("the STL fixture imports as a mesh document");
        core.controller
            .borrow_mut()
            .set_units(cad_domain::UnitContext::drawing_units())
            .unwrap();

        core.request_export().unwrap();

        assert!(
            files.written.borrow().is_empty(),
            "{extension}: a refused unit must write nothing"
        );
        let status = core.last_status().expect("the refusal is graded");
        let (key, args) = catalog(&status);
        assert_eq!(key, "save.export_failed", "{extension}");
        let reason = args.get("reason").expect("the failure names its reason");
        assert!(
            reason.contains(code),
            "{extension}: expected {code}, got {reason}"
        );
    }
}

#[test]
fn exporting_over_the_opened_source_as_a_connector_target_is_allowed() {
    // ADR-0041 D4: no carrier-specific identity guard refuses this any more. The
    // previous bytes are the platform atomic write's job, not the host's.
    let files = Rc::new(RecordingSaveFiles::new());
    *files.chosen.borrow_mut() = Some(FileTarget::from_path("/tmp/source.stl"));
    let core = core_with_caps(files.clone(), capabilities(false));
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/stl/synthetic-quad.stl");
    let bytes = std::fs::read(&fixture).expect("committed STL fixture");
    core.deliver_external_open(FileTarget::from_path("/tmp/source.stl"), Arc::from(bytes))
        .expect("the STL fixture imports");

    core.request_export().unwrap();
    assert_eq!(
        files.written.borrow().len(),
        1,
        "the connector writer runs against the chosen target"
    );
    let status = core.last_status().unwrap();
    let (key, _) = catalog(&status);
    assert!(
        key == "save.exported_connector" || key == "save.exported_connector_partial",
        "a connector export keeps its own status: {key}"
    );
}

#[test]
fn export_on_a_dxf_source_suggests_the_dxf_name() {
    let files = Rc::new(RecordingSaveFiles::new());
    let core = core_with_caps(files.clone(), capabilities(false));
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    core.deliver_external_open(FileTarget::from_path("/tmp/plan.dxf"), Arc::from(bytes))
        .expect("the synthetic DXF source opens");

    core.request_export().unwrap();
    let calls = files.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, SavePurpose::SaveExport);
    assert_eq!(calls[0].1, "plan.dxf");
}
