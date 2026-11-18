//! Native `.ymodel` open/save through the host file funnel (ADR-0037 D2/D3/D12).
//!
//! The archive itself is carrier-owned (`cad-store-zip` is unreachable from
//! `cad-*`), so these contracts drive a **fake** [`DocumentArchive`]: what is
//! under test is the host wiring — which pipeline a target selects, what is
//! installed, which revision is recorded, and that every refusal leaves both the
//! document and the target file untouched.

use super::*;

use cad_db::{DocumentSnapshot, FormatVersion, ManifestSummary};
use cad_domain::Revision;
use cad_platform::{ArchiveBytes, ArchiveError, ArchiveWrite, DocumentArchive, LoadedArchive};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Mutex;

const YMODEL_PATH: &str = "/tmp/plan.ymodel";

/// One `DocumentArchive::write` call, captured for assertions.
struct RecordedWrite {
    snapshot_revision: Revision,
    document_uuid: Option<String>,
    open_revision: u64,
    previous: Option<Vec<u8>>,
    dirty_is_empty: bool,
}

/// A fake carrier archive: one canned decode result, one canned encode answer.
struct FakeArchive {
    read: Result<LoadedArchive, ArchiveError>,
    writes: Mutex<Vec<RecordedWrite>>,
}

impl FakeArchive {
    fn new(loaded: LoadedArchive) -> Self {
        Self {
            read: Ok(loaded),
            writes: Mutex::new(Vec::new()),
        }
    }

    fn corrupt(marker: &'static str) -> Self {
        Self {
            read: Err(ArchiveError::new(
                marker,
                "entry tables/layers.json is truncated",
            )),
            writes: Mutex::new(Vec::new()),
        }
    }
}

impl DocumentArchive for FakeArchive {
    fn read(&self, _bytes: &[u8]) -> Result<LoadedArchive, ArchiveError> {
        self.read.clone()
    }

    fn write(&self, request: &ArchiveWrite<'_>) -> Result<ArchiveBytes, ArchiveError> {
        self.writes.lock().unwrap().push(RecordedWrite {
            snapshot_revision: request.snapshot.revision,
            document_uuid: request.document_uuid.map(str::to_string),
            open_revision: request.open_revision,
            previous: request.previous.map(<[u8]>::to_vec),
            dirty_is_empty: request.dirty.is_empty(),
        });
        Ok(ArchiveBytes {
            bytes: b"FAKE-YMODEL-BYTE-STREAM".to_vec(),
            // A real store echoes the revision it just encoded.
            saved_revision: request.snapshot.revision,
        })
    }
}

/// A platform with a tiny in-memory "disk": writes land there and reads come
/// back out, so the save-time re-read is a real round trip.
#[derive(Default)]
struct YmodelFiles {
    events: RefCell<Vec<FileEvent>>,
    disk: RefCell<BTreeMap<String, Vec<u8>>>,
    written: RefCell<Vec<(String, Vec<u8>)>>,
    source: RefCell<Option<FileTarget>>,
    saves: RefCell<Vec<(SavePurpose, String)>>,
}

impl PlatformFiles for YmodelFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        self.saves
            .borrow_mut()
            .push((purpose, default_name.to_string()));
        // A synchronous picker (the shape the browser/headless carriers use):
        // the target is the opened source when there is one, else the suggested
        // name in a synthetic directory.
        let target = match self.source.borrow().as_ref() {
            Some(source) => source.clone(),
            None => FileTarget {
                display_name: default_name.to_string(),
                opaque: format!("/tmp/{default_name}"),
            },
        };
        self.events
            .borrow_mut()
            .push(FileEvent::SaveTarget { purpose, target });
        Ok(())
    }
    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()> {
        self.written
            .borrow_mut()
            .push((target.display_name.clone(), bytes.to_vec()));
        self.disk
            .borrow_mut()
            .insert(target.opaque.clone(), bytes.to_vec());
        Ok(())
    }
    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        false
    }
    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.borrow_mut() = target.cloned();
    }
    fn read_target(&self, target: &FileTarget) -> Result<Vec<u8>, String> {
        self.disk
            .borrow()
            .get(&target.opaque)
            .cloned()
            .ok_or_else(|| "read-target: not found".to_string())
    }
    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

fn target() -> FileTarget {
    FileTarget {
        display_name: "plan.ymodel".to_string(),
        opaque: YMODEL_PATH.to_string(),
    }
}

fn manifest(saved_revision: Revision) -> ManifestSummary {
    ManifestSummary {
        format: FormatVersion {
            read: 1,
            write: 1,
            minor: 0,
        },
        document_uuid: "doc-uuid-1".to_string(),
        open_revision: Revision(0),
        saved_revision,
        foreign_entries: vec!["foreign/unknown.bin".to_string()],
    }
}

/// A real snapshot of a real database with one line in it, produced by the same
/// `snapshot_document` the save path uses.
fn archived_document() -> (DocumentSnapshot, Vec<u8>) {
    let mut source =
        cad_app::host::HostController::with_blank_document([800.0, 600.0]).expect("blank session");
    source.create_line(p(0.0, 0.0), p(10.0, 0.0)).expect("line");
    let snapshot = source.snapshot_document().expect("snapshot");
    (snapshot, b"ORIGINAL-YMODEL-BYTES".to_vec())
}

/// A core whose demo document is the "previous document" every failure must
/// leave untouched.
fn core_with(files: Rc<YmodelFiles>) -> HostCore {
    core_with_caps(files, capabilities(false))
}

fn open_ymodel(core: &HostCore, archive: Arc<dyn DocumentArchive>, bytes: &[u8]) -> CadResult<()> {
    core.set_document_archive(Some(archive));
    core.deliver_external_open(target(), Arc::from(bytes.to_vec()))
}

#[test]
fn opening_a_ymodel_target_installs_the_archived_document() {
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    let (snapshot, bytes) = archived_document();
    let revision = snapshot.revision;
    let archive = Arc::new(FakeArchive::new(LoadedArchive {
        snapshot,
        manifest: manifest(revision),
    }));
    let manifest_uuid = "doc-uuid-1".to_string();
    let generation = core.controller.borrow().session.generation;

    open_ymodel(&core, archive, &bytes).expect("the archive installs");

    let controller = core.controller.borrow();
    assert!(
        controller.session.generation > generation,
        "a content replacement rebuilds the session (no stale selection/undo)"
    );
    let drawing = controller
        .drawing()
        .expect("the archived document is installed");
    assert_eq!(drawing.entity_count(), 1, "the archived line is present");
    assert_eq!(controller.document_name_hint, "plan.ymodel");
    assert!(
        controller.last_import_report.is_none(),
        "a native open runs no connector and publishes no import report"
    );
    let persistence = controller.persistence().expect("persistence state");
    // The manifest revision seeds the in-memory revision (D2), so the document
    // is not dirty right after opening.
    assert_eq!(persistence.saved_revision, revision);
    assert_eq!(drawing.revision(), revision);
    assert_eq!(persistence.open_revision, Some(Revision(0)));
    assert_eq!(
        persistence.document_uuid.as_deref(),
        Some(manifest_uuid.as_str())
    );
    assert_eq!(persistence.foreign_entries, vec!["foreign/unknown.bin"]);
    assert!(
        persistence.source_sha256.is_some(),
        "the opened bytes are hashed"
    );
    assert!(!controller.is_dirty());
    // A real successful open arms the source guard and records the row.
    assert_eq!(
        files.source.borrow().as_ref().map(|t| t.opaque.clone()),
        Some(YMODEL_PATH.to_string())
    );
    assert!(
        core.import_snapshot().is_some(),
        "the open grades a terminal"
    );
    assert!(matches!(
        core.import_snapshot().and_then(|s| s.terminal),
        Some(ImportTerminal::Opened { entities: 1 })
    ));
}

#[test]
fn opening_a_ymodel_target_without_an_archive_refuses_by_name() {
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files);
    let before = core.controller.borrow().drawing().expect("demo document");
    let (_, bytes) = archived_document();

    // No archive installed: the carrier cannot read the format at all.
    let error = core
        .deliver_external_open(target(), Arc::from(bytes))
        .expect_err("the open must refuse");
    match &error {
        CadError::Unsupported(reason) => assert_eq!(reason, "unsupported-runtime: ymodel"),
        other => panic!("expected the explicit marker, got {other:?}"),
    }
    // The previous document is untouched, byte for byte (same Arc).
    let after = core.controller.borrow().drawing().expect("demo document");
    assert!(
        Arc::ptr_eq(&before, &after),
        "the open must not touch the document"
    );
    // The refusal is a real terminal, not a silent no-op.
    assert!(matches!(
        core.import_snapshot().and_then(|s| s.terminal),
        Some(ImportTerminal::Failed {
            error: CadError::Unsupported(reason)
        }) if reason == "unsupported-runtime: ymodel"
    ));
}

#[test]
fn a_corrupt_ymodel_archive_refuses_and_keeps_the_previous_document() {
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files);
    let before = core.controller.borrow().drawing().expect("demo document");
    let archive = Arc::new(FakeArchive::corrupt(
        cad_domain::ymodel::markers::INVALID_DOCUMENT,
    ));

    let error = open_ymodel(&core, archive, b"not really a zip")
        .expect_err("a corrupt archive must refuse");
    match &error {
        CadError::Unsupported(reason) => {
            assert!(
                reason.starts_with("ymodel.invalid_document: "),
                "the store marker is the contract, got {reason}"
            );
        }
        other => panic!("expected an unsupported refusal, got {other:?}"),
    }
    let after = core.controller.borrow().drawing().expect("demo document");
    assert!(
        Arc::ptr_eq(&before, &after),
        "the previous document survives"
    );
    assert!(
        core.controller.borrow().persistence().is_some(),
        "demo slot intact"
    );
    assert!(matches!(
        core.import_snapshot().and_then(|s| s.terminal),
        Some(ImportTerminal::Failed { .. })
    ));
}

#[test]
fn a_native_save_writes_the_archive_and_advances_the_saved_revision() {
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    let (snapshot, bytes) = archived_document();
    let opened_revision = snapshot.revision;
    files
        .disk
        .borrow_mut()
        .insert(YMODEL_PATH.to_string(), bytes.clone());
    let archive = Arc::new(FakeArchive::new(LoadedArchive {
        snapshot: snapshot.clone(),
        manifest: manifest(opened_revision),
    }));
    open_ymodel(&core, archive.clone(), &bytes).expect("open");

    // One real edit: the document is now dirty.
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 5.0), p(5.0, 5.0))
        .expect("edit");
    let edited_revision = {
        let controller = core.controller.borrow();
        let revision = controller.drawing().expect("drawing").revision();
        assert!(controller.is_dirty(), "the edit is not saved yet");
        revision
    };

    core.request_save().expect("save runs inside the gesture");

    let writes = archive.writes.lock().unwrap();
    assert_eq!(writes.len(), 1, "exactly one archive is encoded");
    assert_eq!(
        writes[0].snapshot_revision, edited_revision,
        "the snapshot is the current revision"
    );
    assert!(writes[0].dirty_is_empty, "Phase 1 rewrites every entry");
    assert_eq!(
        writes[0].previous.as_deref(),
        Some(bytes.as_slice()),
        "the target's previous bytes are handed to the archive"
    );
    assert_eq!(writes[0].document_uuid.as_deref(), Some("doc-uuid-1"));
    assert_eq!(
        writes[0].open_revision, 0,
        "the import baseline never advances"
    );
    drop(writes);

    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "one file written");
    assert_eq!(written[0].1, b"FAKE-YMODEL-BYTE-STREAM");
    drop(written);

    let controller = core.controller.borrow();
    let persistence = controller.persistence().expect("persistence");
    assert_eq!(
        persistence.saved_revision, edited_revision,
        "the archive's echoed revision is applied only after the write"
    );
    assert!(!controller.is_dirty(), "a durable save clears dirty");
    assert_eq!(persistence.document_uuid.as_deref(), Some("doc-uuid-1"));
    assert!(persistence.source_sha256.is_some());
}

#[test]
fn a_native_save_refuses_an_externally_modified_target() {
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    let (snapshot, bytes) = archived_document();
    let opened_revision = snapshot.revision;
    files
        .disk
        .borrow_mut()
        .insert(YMODEL_PATH.to_string(), bytes.clone());
    let archive = Arc::new(FakeArchive::new(LoadedArchive {
        snapshot: snapshot.clone(),
        manifest: manifest(opened_revision),
    }));
    open_ymodel(&core, archive.clone(), &bytes).expect("open");
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 1.0), p(1.0, 1.0))
        .expect("edit");

    // Somebody else wrote the file between open and save.
    let external = b"SOMEONE-ELSE-WROTE-THIS".to_vec();
    files
        .disk
        .borrow_mut()
        .insert(YMODEL_PATH.to_string(), external.clone());

    core.request_save()
        .expect("the save funnel grades a status");

    assert!(
        archive.writes.lock().unwrap().is_empty(),
        "a conflicting save must not even encode"
    );
    assert!(
        files.written.borrow().is_empty(),
        "a conflicting save must not write the target"
    );
    assert_eq!(
        files.disk.borrow().get(YMODEL_PATH),
        Some(&external),
        "the externally modified file is left alone"
    );
    let status = core.last_status().expect("a graded status");
    let (key, args) = catalog(&status);
    assert_eq!(key, "save.export_failed");
    assert!(
        args.get("reason")
            .is_some_and(|reason| reason.contains("ymodel.conflict")),
        "the conflict marker is the reason, got {:?}",
        args.get("reason")
    );
    assert!(
        core.controller.borrow().is_dirty(),
        "a refused save leaves the document dirty"
    );
}

#[test]
fn the_save_purpose_follows_the_source_format() {
    // `.dwg` source → the experimental DWG export (unchanged).
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    *core.source_target.borrow_mut() = Some(FileTarget::from_path("/tmp/plan.dwg"));
    core.request_save().expect("DWG save");
    assert_eq!(files.saves.borrow()[0].0, SavePurpose::SaveDwg);

    // `.ymodel` source → the native save.
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    *core.source_target.borrow_mut() = Some(target());
    core.request_save().expect("native save");
    assert_eq!(files.saves.borrow()[0].0, SavePurpose::SaveYmodel);

    // No source at all (blank/imported document) → the native save.
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    core.request_save().expect("native save");
    assert_eq!(files.saves.borrow()[0].0, SavePurpose::SaveYmodel);
    assert_eq!(files.saves.borrow()[0].1, "yacr-demo.ymodel");

    // `.dxf` source → the lossy DXF Save As (unchanged).
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    *core.source_target.borrow_mut() = Some(FileTarget::from_path("/tmp/plan.dxf"));
    core.request_save().expect("DXF save");
    assert_eq!(files.saves.borrow()[0].0, SavePurpose::SaveDxf);
}

#[test]
fn a_ymodel_save_without_an_archive_refuses_by_name() {
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    *core.source_target.borrow_mut() = Some(target());

    core.request_save().expect("the funnel grades a status");

    assert!(files.written.borrow().is_empty(), "nothing is written");
    let status = core.last_status().expect("status");
    let (key, args) = catalog(&status);
    assert_eq!(key, "save.export_failed");
    assert_eq!(
        args.get("reason").map(String::as_str),
        Some("unsupported-runtime: ymodel")
    );
}
