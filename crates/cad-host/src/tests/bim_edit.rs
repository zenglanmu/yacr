//! Host contracts for the BIM property edit (`SETENTITYPROPERTY` over a
//! `bim.pset.{set}.{property}` slot).
//!
//! These drive `HostCore` through `EngineClient` (no UI, no GPU). The claims are
//! the ones the store depends on:
//!
//! * an edit reaches the drawing database through the one selected-property
//!   interaction and dirties exactly the element shard it changed (a save that
//!   did not learn the shard is dirty would raw-copy the old `bim` bytes);
//! * the value the host hands the archive survives a save and a second,
//!   no-edit save (the raw-copy tier), including `has_bim()`;
//! * a non-BIM document refuses the key and never invents a `bim` block.
//!
//! `cad-store-zip` is carrier-owned, so the archive here is the same capturing
//! fake [`super::ymodel_bim`] uses: the snapshot and dirty scope the host hands
//! the archive are the only authority this layer can see.

use super::*;

use cad_db::{
    BimValue, ChangeScope, DocumentSnapshot, DrawingDatabase, FormatVersion, ManifestSummary,
};
use cad_domain::ymodel::{shard_of, ModelKey};
use cad_domain::{EntityId, Revision};
use cad_platform::{ArchiveBytes, ArchiveError, ArchiveWrite, DocumentArchive, LoadedArchive};
use std::path::Path;
use std::sync::Mutex;

const IFC_FIXTURE: &str = "synthetic-wall-semantics.ifc";
const WALL_A_GLOBAL_ID: &str = "2O2Fr$t4X7Zf8NOew3FLSJ";

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/ifc")
        .join(IFC_FIXTURE)
}

/// Import the committed IFC fixture into the active document (the synchronous
/// host open path).
fn open_bim_fixture(core: &HostCore) {
    let path = fixture_path();
    let bytes = std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "committed IFC fixture {} must be readable: {error}",
            path.display()
        )
    });
    core.controller
        .borrow_mut()
        .open_bytes(Arc::from(bytes), IFC_FIXTURE)
        .expect("the synthetic IFC fixture imports");
}

fn wall_a(core: &HostCore) -> EntityId {
    let controller = core.controller.borrow();
    let drawing = controller.drawing().expect("the imported document");
    drawing
        .bim_elements()
        .iter()
        .find(|(_, bim)| bim.source.id.as_deref() == Some(WALL_A_GLOBAL_ID))
        .map(|(id, _)| *id)
        .expect("Wall A has a BIM block")
}

/// The first stored `Text` slot of an element: its canonical row key and value.
///
/// A text draft needs no unit or numeric grammar, so the host test edits one
/// without guessing the fixture's measure types.
fn text_slot(drawing: &DrawingDatabase, entity: EntityId) -> (String, String) {
    let element = drawing
        .bim_element(entity)
        .expect("the element has BIM data");
    for (set_index, set) in element.psets.iter().enumerate() {
        for (property_index, property) in set.properties.iter().enumerate() {
            if let BimValue::Text(text) = &property.value {
                return (
                    format!("bim.pset.{set_index}.{property_index}"),
                    text.clone(),
                );
            }
        }
    }
    panic!("the fixture element must carry at least one text property");
}

fn stored_text(core: &HostCore, entity: EntityId, key: &str) -> String {
    let slot = cad_app::selection_bim::BimPropertyKey::parse(key).expect("a canonical slot");
    let controller = core.controller.borrow();
    let drawing = controller.drawing().expect("a document");
    match &drawing
        .bim_element(entity)
        .expect("the element has BIM data")
        .psets[slot.set]
        .properties[slot.property]
        .value
    {
        BimValue::Text(text) => text.clone(),
        other => panic!("{key} is no longer text: {other:?}"),
    }
}

fn dispatch_property(core: &HostCore, key: &str, value: &str) -> Result<(), RequestError> {
    EngineClient::dispatch(
        core,
        EngineRequest::Interaction(InteractionRequest::SetSelectedProperty {
            key: key.into(),
            value: value.into(),
        }),
    )
    .map(|_| ())
}

// --- the capturing archive ---------------------------------------------------

/// One write the host handed the archive: the snapshot and the dirty scope.
type CapturedWrite = (DocumentSnapshot, ChangeScope, u32);

#[derive(Default)]
struct BimEditArchive {
    writes: Mutex<Vec<CapturedWrite>>,
    saved: Mutex<Option<(DocumentSnapshot, u32)>>,
}

impl BimEditArchive {
    fn writes(&self) -> Vec<CapturedWrite> {
        self.writes.lock().unwrap().clone()
    }
}

impl DocumentArchive for BimEditArchive {
    fn read(&self, _bytes: &[u8]) -> Result<LoadedArchive, ArchiveError> {
        match self.saved.lock().unwrap().clone() {
            Some((snapshot, minor)) => Ok(LoadedArchive {
                manifest: manifest(snapshot.revision, minor),
                snapshot,
            }),
            None => Err(ArchiveError::new(
                cad_domain::ymodel::markers::INVALID_DOCUMENT,
                "nothing was written before this read",
            )),
        }
    }

    fn write(&self, request: &ArchiveWrite<'_>) -> Result<ArchiveBytes, ArchiveError> {
        // The BIM tier is 7; `FORMAT_MINOR` is now the top format constant
        // (16, groups), not the tier this document earns.
        let minor = if request.snapshot.has_bim() { 7 } else { 4 };
        self.writes
            .lock()
            .unwrap()
            .push((request.snapshot.clone(), request.dirty.clone(), minor));
        *self.saved.lock().unwrap() = Some((request.snapshot.clone(), minor));
        Ok(ArchiveBytes {
            bytes: format!("BIM-EDIT-REV-{}", request.snapshot.revision.0).into_bytes(),
            saved_revision: request.snapshot.revision,
        })
    }
}

fn manifest(saved_revision: Revision, minor: u32) -> ManifestSummary {
    ManifestSummary {
        format: FormatVersion {
            read: 1,
            write: 1,
            minor,
        },
        document_uuid: "doc-uuid-bim-edit".to_string(),
        open_revision: Revision(0),
        saved_revision,
        foreign_entries: Vec::new(),
    }
}

/// A platform with an in-memory disk (an in-place save re-reads its target) and
/// a synchronous save target.
#[derive(Default)]
struct BimEditFiles {
    events: RefCell<Vec<FileEvent>>,
    disk: RefCell<std::collections::BTreeMap<String, Vec<u8>>>,
    written: RefCell<Vec<(String, Vec<u8>)>>,
    source: RefCell<Option<FileTarget>>,
}

impl PlatformFiles for BimEditFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        let target = self.source.borrow().clone().unwrap_or_else(|| FileTarget {
            display_name: default_name.to_string(),
            opaque: format!("/tmp/{default_name}"),
        });
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

fn save_target() -> FileTarget {
    FileTarget {
        display_name: "bim-edit.ymodel".to_string(),
        opaque: "/tmp/bim-edit.ymodel".to_string(),
    }
}

fn bim_core() -> (HostCore, Arc<BimEditArchive>, Rc<BimEditFiles>) {
    let files = Rc::new(BimEditFiles::default());
    let platform = HostPlatform {
        files: files.clone(),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let core =
        HostCore::new(options(), platform, RenderSessionHandle::default()).expect("host core");
    let archive = Arc::new(BimEditArchive::default());
    core.set_document_archive(Some(archive.clone()));
    (core, archive, files)
}

fn save(core: &HostCore, files: &BimEditFiles) -> Vec<u8> {
    core.request_save()
        .expect("the native save completes inside the gesture");
    files
        .written
        .borrow()
        .last()
        .map(|(_, bytes)| bytes.clone())
        .expect("a completed save writes the archive")
}

fn reopen(core: &HostCore, bytes: &[u8]) {
    core.deliver_external_open(save_target(), Arc::from(bytes.to_vec()))
        .expect("the written archive reopens");
}

// --- contracts ---------------------------------------------------------------

#[test]
fn editing_a_bim_property_dirties_its_element_shard() {
    let core = core();
    open_bim_fixture(&core);
    let wall = wall_a(&core);
    let (key, original) = {
        let controller = core.controller.borrow();
        text_slot(&controller.drawing().expect("the document"), wall)
    };
    select_entity(&core, wall);
    let before = core
        .controller
        .borrow()
        .drawing()
        .expect("the document")
        .revision();

    dispatch_property(&core, &key, "edited-by-host").expect("one text slot write");

    assert_eq!(stored_text(&core, wall, &key), "edited-by-host");
    assert_ne!(original, "edited-by-host");
    let controller = core.controller.borrow();
    assert_eq!(
        controller.drawing().expect("the document").revision(),
        Revision(before.0 + 1),
        "one transaction"
    );
    // The `bim` block lives in the element row, so exactly that shard is dirty.
    assert!(
        controller
            .dirty_scope()
            .shards
            .contains(&(ModelKey::ModelSpace, shard_of(wall.0))),
        "the edited element shard must be dirty: {:?}",
        controller.dirty_scope().shards
    );
    let document = controller.document_id;
    assert!(controller.application.can_undo(&document), "one undo step");
}

#[test]
fn a_bim_property_edit_survives_a_second_save() {
    let (core, archive, files) = bim_core();
    open_bim_fixture(&core);
    let wall = wall_a(&core);
    let (key, _) = {
        let controller = core.controller.borrow();
        text_slot(&controller.drawing().expect("the document"), wall)
    };
    select_entity(&core, wall);

    dispatch_property(&core, &key, "survives-two-saves").expect("the edit applies");

    // First save: no previous archive, so the whole document is written and the
    // edit's shard is part of the dirty scope.
    let bytes = save(&core, &files);
    {
        let controller = core.controller.borrow();
        assert!(controller.dirty_scope().is_empty(), "save clears the scope");
    }
    // Second save with no edit in between: the scope is empty, which is exactly
    // the raw-copy tier `cad-store-zip` needs to copy the shard through.
    let second = save(&core, &files);

    let writes = archive.writes();
    assert_eq!(writes.len(), 2);
    assert!(
        writes[0]
            .1
            .shards
            .contains(&(ModelKey::ModelSpace, shard_of(wall.0))),
        "the first save must be told the element shard is dirty"
    );
    assert!(
        writes[1].1.is_empty(),
        "the no-edit save is the raw-copy tier: {:?}",
        writes[1].1
    );
    assert!(writes[1].0.has_bim(), "save 2 still hands over BIM data");
    assert_eq!(writes[1].2, 7, "the BIM tier is preserved");
    assert!(!bytes.is_empty() && !second.is_empty());

    // Reopen the second save: the edited value and the BIM block are there.
    reopen(&core, &second);
    assert_eq!(stored_text(&core, wall, &key), "survives-two-saves");
    let controller = core.controller.borrow();
    let snapshot = controller.snapshot_document().expect("snapshot");
    assert!(
        snapshot.has_bim(),
        "the reopened document keeps its BIM data"
    );
}

#[test]
fn an_edited_bim_block_in_a_non_bim_document_is_refused() {
    let core = core();
    let document = core.controller.borrow().document_id;
    // The demo document: real geometry, no BIM data at all.
    select_entity(&core, EntityId(1));
    let before = core
        .controller
        .borrow()
        .drawing()
        .expect("the document")
        .revision();

    let error = dispatch_property(&core, "bim.pset.0.0", "1").expect_err("no BIM block to edit");
    match error {
        RequestError::Execution(CadError::InvalidInput(reason)) => {
            assert!(reason.contains("property.unsupported_key"), "{reason}");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }

    let controller = core.controller.borrow();
    let drawing = controller.drawing().expect("the document");
    assert_eq!(drawing.revision(), before, "a refusal writes nothing");
    assert!(
        drawing.bim_elements().is_empty(),
        "a refused edit must not fabricate a BIM block"
    );
    assert!(!controller.snapshot_document().expect("snapshot").has_bim());
    assert!(!controller.application.can_undo(&document));
}
