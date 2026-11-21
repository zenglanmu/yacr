//! BIM semantics through the host `.ymodel` funnel (ADR-0042 Phase 1).
//!
//! The end-to-end claim: an IFC file imported through the one connector registry,
//! saved as a native `.ymodel` and reopened gives back the same
//! `cad_db::BimElement` / `cad_db::BimDefinition` rows, the same `bim_key`
//! components, and the same property-panel BIM rows.
//!
//! `cad-store-zip` is carrier-owned (`scripts/check-architecture.py` keeps it out
//! of every shared `cad-*` layer), so these contracts drive a **capturing** fake
//! [`DocumentArchive`] exactly like [`super::ymodel`] and
//! [`super::texture_round_trip`]: the snapshot the host hands the archive is the
//! only authority this layer can see, and the fake serves that snapshot back on
//! reopen — which is what a real store's encode → decode round trip does.
//!
//! The fake also models the carrier's `manifest.format.minor` decision from
//! public snapshot state ([`DocumentSnapshot::has_bim`] and the resources tier),
//! i.e. the same three tiers `cad-store-zip::minor` documents. The real stamp and
//! the archive entry naming are pinned by that crate's own
//! `tests/bim_minor.rs`; here the projection turns "a save dropped the BIM block"
//! into a failing manifest number instead of a silent downgrade.

use super::*;

use cad_db::{
    BimDefinitionKind, BimDefinitions, BimElement, BimKey, DocumentSnapshot, DrawingDatabase,
    FormatVersion, ManifestSummary,
};
use cad_domain::ymodel::FORMAT_MINOR;
use cad_domain::{EntityId, Revision};
use cad_platform::{ArchiveBytes, ArchiveError, ArchiveWrite, DocumentArchive, LoadedArchive};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

/// The committed synthetic IFC4 fixture (two storeys, two walls, a type, a
/// `Pset_WallCommon` with bool/identifier/complex values and a material).
const IFC_FIXTURE: &str = "synthetic-wall-semantics.ifc";

/// Wall A's `GlobalId` in the fixture; its BIM block is the representative row.
const WALL_A_GLOBAL_ID: &str = "2O2Fr$t4X7Zf8NOew3FLSJ";

const YMODEL_PATH: &str = "/tmp/bim-model.ymodel";

/// One element's BIM state captured on one side of a save/reopen round trip.
#[derive(Debug, Clone, PartialEq)]
struct DocumentBim {
    elements: BTreeMap<EntityId, BimElement>,
    definitions: BimDefinitions,
    /// The logical component of every element, captured through `bim_key` so the
    /// round trip also proves the grouping key is unchanged.
    keys: Vec<(EntityId, BimKey)>,
}

/// Capture a database's whole BIM state through the read-only accessors.
fn snapshot_bim(drawing: &DrawingDatabase) -> DocumentBim {
    DocumentBim {
        elements: drawing.bim_elements().clone(),
        definitions: drawing.bim_definitions().clone(),
        keys: drawing
            .bim_elements()
            .keys()
            .map(|id| {
                (
                    *id,
                    drawing
                        .bim_key(*id)
                        .expect("a BIM element always has a logical-component key"),
                )
            })
            .collect(),
    }
}

fn count_kind(definitions: &BimDefinitions, kind: BimDefinitionKind) -> usize {
    definitions.iter().filter(|row| row.kind() == kind).count()
}

/// A fake carrier archive: one captured snapshot per `write`, served back on the
/// next `read`.
#[derive(Default)]
struct BimArchive {
    writes: Mutex<Vec<DocumentSnapshot>>,
    minors: Mutex<Vec<u32>>,
    saved: Mutex<Option<(DocumentSnapshot, u32)>>,
}

impl BimArchive {
    fn writes(&self) -> Vec<DocumentSnapshot> {
        self.writes.lock().unwrap().clone()
    }

    fn minors(&self) -> Vec<u32> {
        self.minors.lock().unwrap().clone()
    }
}

impl DocumentArchive for BimArchive {
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
        // The carrier's per-document minor decision, from the snapshot alone.
        let minor = stand_in_document_minor(request.snapshot);
        self.writes.lock().unwrap().push(request.snapshot.clone());
        self.minors.lock().unwrap().push(minor);
        *self.saved.lock().unwrap() = Some((request.snapshot.clone(), minor));
        Ok(ArchiveBytes {
            bytes: format!("BIM-YMODEL-REV-{}", request.snapshot.revision.0).into_bytes(),
            saved_revision: request.snapshot.revision,
        })
    }
}

/// The `manifest.format.minor` the carrier store stamps, projected from the
/// snapshot with the documented three tiers (ADR-0037 D13, ADR-0040 D12,
/// ADR-0042 D6).
///
/// `cad-host` cannot link `cad-store-zip` (architecture boundary), so this mirrors
/// `cad-store-zip::minor::document_minor` over the public snapshot state. It is
/// derived **from what the host handed the archive**: if a save drops the `bim`
/// blocks or the definition table, this projection drops to 4/5 and the test
/// fails, which is the regression the contract is about.
fn stand_in_document_minor(snapshot: &DocumentSnapshot) -> u32 {
    if snapshot.has_bim() {
        return FORMAT_MINOR;
    }
    const FORMAT_MINOR_PLAIN: u32 = 4;
    const FORMAT_MINOR_RESOURCES: u32 = 5;
    let has_resource_table = snapshot.tables.get(cad_db::TableKind::Resources).is_some();
    if snapshot.header.embed_resources || !snapshot.resources.is_empty() || has_resource_table {
        return FORMAT_MINOR_RESOURCES;
    }
    FORMAT_MINOR_PLAIN
}

fn manifest(saved_revision: Revision, minor: u32) -> ManifestSummary {
    ManifestSummary {
        format: FormatVersion {
            read: 1,
            write: 1,
            minor,
        },
        document_uuid: "doc-uuid-bim".to_string(),
        open_revision: Revision(0),
        saved_revision,
        foreign_entries: Vec::new(),
    }
}

/// A platform with a tiny in-memory disk (an in-place save re-reads its target to
/// detect an external modification) and a synchronous save target.
#[derive(Default)]
struct BimFiles {
    events: RefCell<Vec<FileEvent>>,
    disk: RefCell<BTreeMap<String, Vec<u8>>>,
    written: RefCell<Vec<(String, Vec<u8>)>>,
    source: RefCell<Option<FileTarget>>,
}

impl PlatformFiles for BimFiles {
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

fn target() -> FileTarget {
    FileTarget {
        display_name: "bim-model.ymodel".to_string(),
        opaque: YMODEL_PATH.to_string(),
    }
}

fn bim_core() -> (HostCore, Arc<BimArchive>, Rc<BimFiles>) {
    let files = Rc::new(BimFiles::default());
    let platform = HostPlatform {
        files: files.clone(),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let core =
        HostCore::new(options(), platform, RenderSessionHandle::default()).expect("host core");
    let archive = Arc::new(BimArchive::default());
    core.set_document_archive(Some(archive.clone()));
    (core, archive, files)
}

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/ifc")
        .join(IFC_FIXTURE)
}

/// Import the committed IFC fixture through the real connector registry into the
/// active document slot (the synchronous host open path).
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

/// Run SAVE and return the bytes the platform wrote.
fn save(core: &HostCore, files: &BimFiles) -> Vec<u8> {
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
    core.deliver_external_open(target(), Arc::from(bytes.to_vec()))
        .expect("the written archive reopens");
}

/// Whether a value carries CJK copy (the property panel supplies localized
/// labels from the catalog; values must stay source data / ASCII tokens).
fn has_cjk(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(
            character as u32,
            0x3000..=0x303F   // CJK punctuation
            | 0x3400..=0x4DBF // CJK extension A
            | 0x4E00..=0x9FFF // CJK unified ideographs
            | 0xF900..=0xFAFF // compatibility ideographs
            | 0xFF00..=0xFFEF // full-width forms
        )
    })
}

#[test]
fn ifc_bim_survives_a_ymodel_save_and_reopen() {
    let (core, _archive, files) = bim_core();
    open_bim_fixture(&core);

    let before = {
        let controller = core.controller.borrow();
        let drawing = controller.drawing().expect("the imported document");
        assert!(
            !drawing.bim_elements().is_empty(),
            "the IFC fixture must produce BIM elements, not just geometry"
        );
        assert!(
            drawing
                .bim_elements()
                .values()
                .any(|bim| bim.category.as_deref() == Some("IfcWall")),
            "the imported rows keep the IFC product class"
        );
        let captured = snapshot_bim(&drawing);
        assert!(
            count_kind(&captured.definitions, BimDefinitionKind::Level) >= 2,
            "the fixture declares two storeys, got {} level rows",
            count_kind(&captured.definitions, BimDefinitionKind::Level)
        );
        assert!(
            count_kind(&captured.definitions, BimDefinitionKind::Type) >= 1,
            "the fixture declares an IfcWallType"
        );
        assert!(
            count_kind(&captured.definitions, BimDefinitionKind::Material) >= 1,
            "the fixture associates a material"
        );
        captured
    };

    let before_wall = before
        .elements
        .values()
        .find(|bim| bim.source.id.as_deref() == Some(WALL_A_GLOBAL_ID))
        .expect("Wall A's BIM block")
        .clone();

    let bytes = save(&core, &files);
    reopen(&core, &bytes);

    let after = {
        let controller = core.controller.borrow();
        snapshot_bim(&controller.drawing().expect("the reopened document"))
    };

    // Whole-state equality: same element rows, same definition table, same keys.
    assert_eq!(
        after, before,
        "the BIM rows must survive save → reopen unchanged"
    );

    // Field-by-field for the representative element, so a failure names the field
    // instead of dumping both maps.
    let after_wall = after
        .elements
        .values()
        .find(|bim| bim.source.id.as_deref() == Some(WALL_A_GLOBAL_ID))
        .expect("Wall A's BIM block after reopen");
    assert_eq!(after_wall.category, before_wall.category, "category");
    assert_eq!(after_wall.name, before_wall.name, "name");
    assert_eq!(after_wall.source, before_wall.source, "source");
    assert_eq!(after_wall.level, before_wall.level, "level reference");
    assert_eq!(
        after_wall.element_type, before_wall.element_type,
        "type reference"
    );
    assert_eq!(
        after_wall.material, before_wall.material,
        "material reference"
    );
    assert_eq!(after_wall.psets, before_wall.psets, "property sets");
    assert_eq!(after_wall.evidence, before_wall.evidence, "evidence");

    // The references still resolve on the reopened database (not dangling ids).
    let controller = core.controller.borrow();
    let reopened = controller.drawing().expect("the reopened document");
    for reference in [
        after_wall.level.as_deref(),
        after_wall.element_type.as_deref(),
        after_wall.material.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        assert!(
            reopened.bim_definition(reference).is_some(),
            "the reopened document must still resolve {reference:?}"
        );
    }
}

#[test]
fn a_bim_document_is_written_at_minor_six() {
    let (core, archive, files) = bim_core();
    open_bim_fixture(&core);

    save(&core, &files);
    // A second save in the same session: the tier must not drift.
    save(&core, &files);

    assert_eq!(
        archive.minors(),
        vec![FORMAT_MINOR, FORMAT_MINOR],
        "a BIM-bearing document is stamped minor {FORMAT_MINOR} on every save"
    );
    assert_eq!(FORMAT_MINOR, 6, "ADR-0042 D6 raised the ceiling to 6");
    for (index, snapshot) in archive.writes().iter().enumerate() {
        assert!(
            snapshot.has_bim(),
            "save {index} handed the archive a snapshot with no BIM data"
        );
    }
}

#[test]
fn a_bim_free_document_stays_at_minor_four() {
    let (core, archive, files) = bim_core();
    // The default demo drawing: real geometry (a line, a circle, …), no BIM data.
    let drawing = core.controller.borrow().drawing().expect("demo document");
    assert!(
        drawing.entity_count() > 0,
        "the regression must run on a non-empty ordinary drawing"
    );
    assert!(
        drawing.bim_elements().is_empty(),
        "the demo document carries no BIM data"
    );
    drop(drawing);

    save(&core, &files);

    assert_eq!(
        archive.minors(),
        vec![4],
        "an ordinary drawing must not be stamped with the BIM tier"
    );
    let writes = archive.writes();
    assert_eq!(writes.len(), 1);
    assert!(!writes[0].has_bim(), "no BIM row may be invented on save");
}

#[test]
fn reopening_a_bim_document_exposes_bim_property_rows() {
    let (core, _archive, files) = bim_core();
    open_bim_fixture(&core);
    let bytes = save(&core, &files);
    reopen(&core, &bytes);

    let wall = {
        let controller = core.controller.borrow();
        let drawing = controller.drawing().expect("the reopened document");
        drawing
            .bim_elements()
            .iter()
            .find(|(_, bim)| bim.source.id.as_deref() == Some(WALL_A_GLOBAL_ID))
            .map(|(id, _)| *id)
            .expect("Wall A still has a BIM block after reopen")
    };
    select_entity(&core, wall);

    let properties = core
        .controller
        .borrow()
        .selection_properties()
        .expect("one selected BIM element has properties");
    assert_eq!(properties.count, 1);

    for key in [
        "bim_category",
        "bim_level",
        "bim_type",
        "bim_material",
        "bim_source",
        "bim_properties",
    ] {
        let row = properties.rows.iter().find(|row| row.key == key);
        let row = row.unwrap_or_else(|| {
            panic!(
                "missing {key} row; present keys: {:?}",
                properties
                    .rows
                    .iter()
                    .map(|row| row.key)
                    .collect::<Vec<_>>()
            )
        });
        let value = row
            .value
            .as_deref()
            .unwrap_or_else(|| panic!("{key} must carry the declared value"));
        assert!(!value.is_empty(), "{key} must not be blank");
        assert!(
            !has_cjk(value),
            "{key} must be source data, not localized copy: {value:?}"
        );
    }

    // The definition references resolve to the source's names, not to raw ids.
    let value_of = |key: &str| {
        properties
            .rows
            .iter()
            .find(|row| row.key == key)
            .and_then(|row| row.value.clone())
            .unwrap_or_default()
    };
    assert_eq!(value_of("bim_category"), "IfcWall");
    assert_eq!(value_of("bim_level"), "Level 0");
    assert_eq!(value_of("bim_type"), "Basic Wall 200mm");
    assert_eq!(value_of("bim_material"), "Concrete");
    assert_eq!(
        value_of("bim_properties"),
        "Pset_WallCommon (2)",
        "the two single-value properties are summarized; the complex value is not flattened"
    );
}

#[test]
fn a_plain_mesh_element_has_no_bim_rows() {
    // Same document: an entity created after the BIM import carries no `bim`
    // block, so the panel must not show BIM rows for it.
    let (core, _archive, _files) = bim_core();
    open_bim_fixture(&core);
    let created = core
        .controller
        .borrow_mut()
        .create_line(p(0.0, 0.0), p(10.0, 0.0))
        .expect("a new line in the BIM document");
    let line = EntityId(created.objects.first().expect("the new line's id").0);
    {
        let controller = core.controller.borrow();
        let drawing = controller.drawing().expect("the BIM document");
        assert!(
            drawing.bim_element(line).is_none(),
            "the new line must have no BIM block"
        );
        assert!(
            !drawing.bim_elements().is_empty(),
            "the imported BIM elements are still there"
        );
    }
    select_entity(&core, line);
    let properties = core
        .controller
        .borrow()
        .selection_properties()
        .expect("the line has properties");
    let bim_keys: Vec<&str> = properties
        .rows
        .iter()
        .filter(|row| row.key.starts_with("bim_"))
        .map(|row| row.key)
        .collect();
    assert!(
        bim_keys.is_empty(),
        "a plain element must not expose BIM rows: {bim_keys:?}"
    );

    // A plain mesh document (no BIM data at all): same answer for its mesh.
    let (mesh_core, _archive, _files) = bim_core();
    let stl = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/stl/synthetic-quad.stl");
    let bytes = std::fs::read(&stl).expect("committed STL fixture");
    mesh_core
        .deliver_external_open(FileTarget::from_path("/tmp/source.stl"), Arc::from(bytes))
        .expect("the STL fixture imports as a mesh document");
    let mesh = {
        let controller = mesh_core.controller.borrow();
        let drawing = controller.drawing().expect("the mesh document");
        assert!(
            drawing.bim_elements().is_empty(),
            "a plain STL import must not invent BIM data"
        );
        drawing
            .model_space()
            .first()
            .map(|entity| entity.id)
            .expect("the STL fixture has a mesh entity")
    };
    select_entity(&mesh_core, mesh);
    let properties = mesh_core
        .controller
        .borrow()
        .selection_properties()
        .expect("the mesh has properties");
    let bim_keys: Vec<&str> = properties
        .rows
        .iter()
        .filter(|row| row.key.starts_with("bim_"))
        .map(|row| row.key)
        .collect();
    assert!(
        bim_keys.is_empty(),
        "a plain mesh must not expose BIM rows: {bim_keys:?}"
    );
}
