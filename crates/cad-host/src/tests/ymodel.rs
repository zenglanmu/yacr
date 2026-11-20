//! Native `.ymodel` open/save through the host file funnel (ADR-0037 D2/D3/D12).
//!
//! The archive itself is carrier-owned (`cad-store-zip` is unreachable from
//! `cad-*`), so these contracts drive a **fake** [`DocumentArchive`]: what is
//! under test is the host wiring — which pipeline a target selects, what is
//! installed, which revision is recorded, and that every refusal leaves both the
//! document and the target file untouched.

use super::*;

use cad_app::{Command, CommandId, CommandPayload};
use cad_db::{ChangeScope, DocumentSnapshot, FormatVersion, ManifestSummary};
use cad_domain::ymodel::{ModelKey, Shard};
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
    /// The base archive the write was told to encode from (`previous`).
    previous: Option<Vec<u8>>,
    /// The accumulated dirty scope the host handed to the write.
    dirty: ChangeScope,
}

/// A fake carrier archive: one canned decode result, one canned encode answer.
///
/// A successful `write` also retains its snapshot so a later `read` serves it
/// back: that is what makes a save → reopen round trip a real one here (a real
/// store would encode and decode the same content).
struct FakeArchive {
    read: Result<LoadedArchive, ArchiveError>,
    saved: Mutex<Option<DocumentSnapshot>>,
    writes: Mutex<Vec<RecordedWrite>>,
}

impl FakeArchive {
    fn new(loaded: LoadedArchive) -> Self {
        Self {
            read: Ok(loaded),
            saved: Mutex::new(None),
            writes: Mutex::new(Vec::new()),
        }
    }

    fn corrupt(marker: &'static str) -> Self {
        Self {
            read: Err(ArchiveError::new(
                marker,
                "entry tables/layers.json is truncated",
            )),
            saved: Mutex::new(None),
            writes: Mutex::new(Vec::new()),
        }
    }
}

impl DocumentArchive for FakeArchive {
    fn read(&self, _bytes: &[u8]) -> Result<LoadedArchive, ArchiveError> {
        // Reopening a file this fake wrote returns what was written.
        if let Some(snapshot) = self.saved.lock().unwrap().clone() {
            return Ok(LoadedArchive {
                manifest: manifest(snapshot.revision),
                snapshot,
            });
        }
        self.read.clone()
    }

    fn write(&self, request: &ArchiveWrite<'_>) -> Result<ArchiveBytes, ArchiveError> {
        self.writes.lock().unwrap().push(RecordedWrite {
            snapshot_revision: request.snapshot.revision,
            document_uuid: request.document_uuid.map(str::to_string),
            open_revision: request.open_revision,
            previous: request.previous.map(<[u8]>::to_vec),
            dirty: request.dirty.clone(),
        });
        *self.saved.lock().unwrap() = Some(request.snapshot.clone());
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
    /// A target the next `begin_save` must return instead of the source, so a
    /// Save As to a **different** file can be exercised.
    save_as: RefCell<Option<FileTarget>>,
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
        // the target is an explicit Save As when one was queued, else the opened
        // source, else the suggested name in a synthetic directory.
        let target = match self.save_as.borrow_mut().take() {
            Some(target) => target,
            None => match self.source.borrow().as_ref() {
                Some(source) => source.clone(),
                None => FileTarget {
                    display_name: default_name.to_string(),
                    opaque: format!("/tmp/{default_name}"),
                },
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
    let created = core
        .controller
        .borrow_mut()
        .create_line(p(0.0, 5.0), p(5.0, 5.0))
        .expect("edit");
    let new_entity = created.objects.first().expect("the new entity id").0;
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
    assert!(
        writes[0]
            .dirty
            .touches_shard(ModelKey::ModelSpace, Shard::of(new_entity)),
        "the accumulated scope names the new entity's shard, got {:?}",
        writes[0].dirty
    );
    assert!(
        writes[0].dirty.header,
        "a new row moves the id high-water mark, so document.json is dirty"
    );
    assert_eq!(
        writes[0].previous.as_deref(),
        Some(bytes.as_slice()),
        "the document's own opened bytes are the base of the write"
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
    assert!(
        controller.dirty_scope().is_empty(),
        "a durable save clears the accumulated scope"
    );
    assert_eq!(
        persistence.archive_bytes.as_deref(),
        Some(b"FAKE-YMODEL-BYTE-STREAM".as_slice()),
        "the committed bytes become the next save's base"
    );
    assert_eq!(persistence.document_uuid.as_deref(), Some("doc-uuid-1"));
    assert!(persistence.source_sha256.is_some());
}

#[test]
fn a_second_save_carries_only_the_edits_since_the_first_one() {
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    let (snapshot, bytes) = archived_document();
    let opened_revision = snapshot.revision;
    files
        .disk
        .borrow_mut()
        .insert(YMODEL_PATH.to_string(), bytes.clone());
    let archive = Arc::new(FakeArchive::new(LoadedArchive {
        snapshot,
        manifest: manifest(opened_revision),
    }));
    open_ymodel(&core, archive.clone(), &bytes).expect("open");
    assert!(
        core.controller.borrow().dirty_scope().is_empty(),
        "a freshly opened document has nothing outstanding"
    );

    // First edit, then save.
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 5.0), p(5.0, 5.0))
        .expect("first edit");
    core.request_save().expect("first save");
    assert!(
        core.controller.borrow().dirty_scope().is_empty(),
        "the scope is empty right after a durable save"
    );

    // Second edit: the accumulated set must be exactly this commit's scope, not
    // a union with the first (already saved) edit.
    let second = core
        .controller
        .borrow_mut()
        .create_line(p(0.0, 7.0), p(7.0, 7.0))
        .expect("second edit");
    let second_scope = second
        .changes
        .as_ref()
        .expect("a committed edit publishes its change set")
        .scope
        .clone();
    assert!(!second_scope.is_empty(), "the second edit is real");
    {
        let controller = core.controller.borrow();
        assert_eq!(
            controller.dirty_scope(),
            &second_scope,
            "only the second edit's scope is accumulated"
        );
    }
    core.request_save().expect("second save");
    assert!(
        core.controller.borrow().dirty_scope().is_empty(),
        "the second save clears it again"
    );

    let writes = archive.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(
        writes[1].dirty, second_scope,
        "the second write is narrowed by the second edit's scope only"
    );
}

#[test]
fn a_layer_rename_marks_the_layers_table_and_survives_a_save_and_reopen() {
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    let (snapshot, bytes) = archived_document();
    let opened_revision = snapshot.revision;
    files
        .disk
        .borrow_mut()
        .insert(YMODEL_PATH.to_string(), bytes.clone());
    let archive = Arc::new(FakeArchive::new(LoadedArchive {
        snapshot,
        manifest: manifest(opened_revision),
    }));
    open_ymodel(&core, archive.clone(), &bytes).expect("open");

    // A layer write path publishes an **empty** change list (no entity rows) and
    // only its scope says which table went stale.
    let (document_id, viewport_id) = {
        let controller = core.controller.borrow();
        (controller.document_id, controller.viewport_id)
    };
    core.controller
        .borrow_mut()
        .execute(Command {
            schema_version: 1,
            id: CommandId::RenameLayer,
            document: document_id,
            viewport: viewport_id,
            payload: CommandPayload::RenameLayer {
                layer: "0".into(),
                name: "WALLS".into(),
            },
        })
        .expect("the rename applies");
    assert!(
        core.controller
            .borrow()
            .dirty_scope()
            .tables
            .contains(&cad_db::TableKind::Layers),
        "a layer rename must dirty the layer table even with no entity changes"
    );

    let renamed_revision = {
        let controller = core.controller.borrow();
        let revision = controller.drawing().expect("drawing").revision();
        assert!(revision.0 > opened_revision.0, "the rename is a revision");
        revision
    };
    core.request_save().expect("save");
    {
        let writes = archive.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].snapshot_revision, renamed_revision);
        assert!(
            writes[0].dirty.tables.contains(&cad_db::TableKind::Layers),
            "the layer table is named in the encoded dirty set"
        );
    }

    // Reopen the file the save just wrote: the renamed table row is there.
    let reopened = b"FAKE-YMODEL-BYTE-STREAM".to_vec();
    open_ymodel(&core, archive.clone(), &reopened).expect("reopen");
    let controller = core.controller.borrow();
    let drawing = controller.drawing().expect("reopened drawing");
    assert!(
        drawing.layers().any(|layer| layer.name == "WALLS"),
        "the renamed layer survived the save/reopen round trip"
    );
    assert!(
        !drawing.layers().any(|layer| layer.name == "0"),
        "the old name is gone from the reopened document"
    );
    assert!(
        controller.dirty_scope().is_empty(),
        "reopening starts with no accumulated scope"
    );
}

#[test]
fn opening_a_document_clears_the_accumulated_scope() {
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    let (snapshot, bytes) = archived_document();
    let opened_revision = snapshot.revision;
    files
        .disk
        .borrow_mut()
        .insert(YMODEL_PATH.to_string(), bytes.clone());
    let archive = Arc::new(FakeArchive::new(LoadedArchive {
        snapshot,
        manifest: manifest(opened_revision),
    }));
    open_ymodel(&core, archive, &bytes).expect("open");
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 2.0), p(2.0, 2.0))
        .expect("edit");
    assert!(
        !core.controller.borrow().dirty_scope().is_empty(),
        "the edit accumulated a scope"
    );

    // The scope described the replaced content; a New must not carry it over.
    core.controller
        .borrow_mut()
        .new_blank_document()
        .expect("a blank document replaces the edited one");
    assert!(
        core.controller.borrow().dirty_scope().is_empty(),
        "New starts with no accumulated scope"
    );
}

#[test]
fn a_save_as_keeps_the_source_archive_and_the_accumulated_scope() {
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    let (snapshot, bytes) = archived_document();
    let opened_revision = snapshot.revision;
    files
        .disk
        .borrow_mut()
        .insert(YMODEL_PATH.to_string(), bytes.clone());
    let archive = Arc::new(FakeArchive::new(LoadedArchive {
        snapshot,
        manifest: manifest(opened_revision),
    }));
    open_ymodel(&core, archive.clone(), &bytes).expect("open");
    let created = core
        .controller
        .borrow_mut()
        .create_line(p(0.0, 3.0), p(3.0, 3.0))
        .expect("edit");
    let new_entity = created.objects.first().expect("the new entity id").0;

    // Save As: a different, not-yet-existing target. The platform would hand this
    // back from the picker; the fake returns it instead of the source.
    let copy = FileTarget {
        display_name: "copy.ymodel".to_string(),
        opaque: "/tmp/copy.ymodel".to_string(),
    };
    *files.save_as.borrow_mut() = Some(copy.clone());
    core.request_save()
        .expect("the Save As runs inside the gesture");

    let writes = archive.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0].previous.as_deref(),
        Some(bytes.as_slice()),
        "a Save As carries the document's own archive, never the destination's"
    );
    assert!(
        !writes[0].dirty.is_empty(),
        "the accumulated scope travels with the Save As"
    );
    assert!(
        writes[0]
            .dirty
            .touches_shard(ModelKey::ModelSpace, Shard::of(new_entity)),
        "the edit's shard is in the dirty set, got {:?}",
        writes[0].dirty
    );
    drop(writes);

    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "one file written");
    assert_eq!(
        written[0].0, "copy.ymodel",
        "the Save As target was written"
    );
    drop(written);

    let controller = core.controller.borrow();
    assert!(
        controller.dirty_scope().is_empty(),
        "the Save As is durable, so the scope is cleared"
    );
    assert_eq!(
        controller
            .persistence()
            .expect("persistence")
            .archive_bytes
            .as_deref(),
        Some(b"FAKE-YMODEL-BYTE-STREAM".as_slice()),
        "the saved bytes become the next base"
    );
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

#[test]
fn import_report_and_source_rows_survive_a_save_and_reopen() {
    // ADR-0037 D11/D14: `tables/compat.json` and `tables/sources.json` are
    // host-owned document metadata. They must travel with the archive and come
    // back on the installed document, exactly like the database tables do.
    let files = Rc::new(YmodelFiles::default());
    let core = core_with(files.clone());
    let (snapshot, bytes) = archived_document();
    let opened_revision = snapshot.revision;
    files
        .disk
        .borrow_mut()
        .insert(YMODEL_PATH.to_string(), bytes.clone());
    let archive = Arc::new(FakeArchive::new(LoadedArchive {
        snapshot,
        manifest: manifest(opened_revision),
    }));
    open_ymodel(&core, archive.clone(), &bytes).expect("open");

    let compat = serde_json::json!({
        "connector": "dxf",
        "direction": "import",
        "level": "L1",
        "counts": null,
        "completeness": {"kind": "partial", "reasons": ["spline flags dropped"]},
        "capabilities": [],
        "entries": [],
    });
    let source = serde_json::json!({
        "source_id": "uuid:0",
        "identifier": "plan.dxf",
        "kind": "drawing",
        "checksum": null,
        "retained": false,
        "connector": "dxf",
        "open_revision": null,
    });
    {
        let mut controller = core.controller.borrow_mut();
        let document_id = controller.document_id;
        let document = controller
            .application
            .workspace
            .documents
            .get_mut(&document_id)
            .expect("the opened document");
        document.compat = vec![compat.clone()];
        document.sources = vec![source.clone()];
    }

    core.request_save().expect("save");

    // The snapshot the archive was asked to encode carries both rows.
    let saved = archive
        .saved
        .lock()
        .unwrap()
        .clone()
        .expect("the write captured a snapshot");
    assert_eq!(
        saved.tables.get(cad_db::TableKind::Compat),
        Some(&[compat.clone()][..])
    );
    assert_eq!(
        saved.tables.get(cad_db::TableKind::Sources),
        Some(&[source.clone()][..])
    );

    // Reopen the bytes the save just wrote: the rows are installed on the new
    // document (they never enter `cad-db`).
    let reopened = b"FAKE-YMODEL-BYTE-STREAM".to_vec();
    open_ymodel(&core, archive, &reopened).expect("reopen");
    let controller = core.controller.borrow();
    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .expect("the reopened document");
    assert_eq!(document.compat, vec![compat]);
    assert_eq!(document.sources, vec![source]);
    assert_eq!(
        document.drawing.entity_count(),
        1,
        "the database content is unaffected by the metadata tables"
    );
}
