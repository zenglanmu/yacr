//! Mesh base-colour textures through the host `.ymodel` funnel (ADR-0038 D3/D4/D10).
//!
//! `cad-store-zip` is carrier-owned (`scripts/check-architecture.py` keeps it out
//! of every shared `cad-*` layer), so this contract drives a **capturing** fake
//! [`DocumentArchive`] exactly like [`super::ymodel`] does. What is under test is
//! the host funnel: the snapshot the host hands the archive must carry the
//! document's texture bytes and the mesh's `uvs`/`texture`, and reopening that
//! archive must install them back on the drawing while the image pass decodes the
//! texture from the database (no file, no placeholder).
//!
//! The `textures/<key>` **entry naming** itself is `cad-store-zip`'s contract
//! (ADR-0038 D4, pinned by that crate's own tests); the snapshot is the only
//! authority this layer can see, so it is what these contracts assert.
//!
//! The image side uses the desktop strategy (`DesktopResources`) rather than the
//! `UnsupportedResources` stub the other host tests use, so `install_images` runs
//! the real document-aware loader and publishes the texture key.

use super::*;

use cad_db::{
    DbEntity, DbObject, DocumentSnapshot, DrawingDatabase, DrawingDatabaseBuilder, FormatVersion,
    Layer, ManifestSummary,
};
use cad_domain::ymodel::FORMAT_MINOR;
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, Mesh, ObjectId, Point3, Revision,
    SemanticGeometry, SpaceId, UnitContext,
};
use cad_platform::{
    ArchiveBytes, ArchiveError, ArchiveWrite, DocumentArchive, FileEvent, FileTarget,
    LoadedArchive, PlatformFiles, SavePurpose,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// A canonical content-addressed key: 32 lowercase hex + `.png`.
const TEXTURE_KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";
const YMODEL_PATH: &str = "/tmp/textured.ymodel";

/// A 2x2 RGBA PNG (red, green, blue, dark), encoded once and committed inline so
/// the test needs no binary fixture and no encoder dependency in `cad-host`.
const TEXTURE_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xb6, 0x0d,
    0x24, 0x00, 0x00, 0x00, 0x15, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x0c, 0x81, 0xf4, 0x7f, 0x2e, 0x11, 0xb9, 0xff, 0x00, 0x41, 0x71, 0x07, 0x36, 0x40, 0xa9,
    0x20, 0x1a, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

const YMODEL_BYTES: &[u8] = b"CAPTURED-TEXTURED-YMODEL";

/// A fake carrier archive: one canned open, one captured save.
///
/// A successful `write` retains its snapshot, so a later `read` serves it back:
/// that is what makes save → reopen a real round trip here (a real store would
/// encode and decode the same content).
struct TextureArchive {
    opened: LoadedArchive,
    saved: Mutex<Option<DocumentSnapshot>>,
}

impl TextureArchive {
    fn new(opened: LoadedArchive) -> Self {
        Self {
            opened,
            saved: Mutex::new(None),
        }
    }
}

impl DocumentArchive for TextureArchive {
    fn read(&self, _bytes: &[u8]) -> Result<LoadedArchive, ArchiveError> {
        if let Some(snapshot) = self.saved.lock().unwrap().clone() {
            return Ok(LoadedArchive {
                manifest: manifest(snapshot.revision),
                snapshot,
            });
        }
        Ok(self.opened.clone())
    }

    fn write(&self, request: &ArchiveWrite<'_>) -> Result<ArchiveBytes, ArchiveError> {
        *self.saved.lock().unwrap() = Some(request.snapshot.clone());
        Ok(ArchiveBytes {
            bytes: YMODEL_BYTES.to_vec(),
            saved_revision: request.snapshot.revision,
        })
    }
}

/// A synchronous platform whose "disk" is a map, so an in-place `.ymodel` save
/// can really re-read the target it was opened from.
#[derive(Default)]
struct DiskFiles {
    events: RefCell<Vec<FileEvent>>,
    disk: RefCell<BTreeMap<String, Vec<u8>>>,
    written: RefCell<Vec<(String, Vec<u8>)>>,
    source: RefCell<Option<FileTarget>>,
}

impl PlatformFiles for DiskFiles {
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

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn target() -> FileTarget {
    FileTarget {
        display_name: "textured.ymodel".to_string(),
        opaque: YMODEL_PATH.to_string(),
    }
}

fn manifest(saved_revision: Revision) -> ManifestSummary {
    ManifestSummary {
        format: FormatVersion {
            read: 1,
            write: 1,
            minor: FORMAT_MINOR,
        },
        document_uuid: "doc-uuid-textured".to_string(),
        open_revision: Revision(0),
        saved_revision,
        foreign_entries: Vec::new(),
    }
}

/// A core with the **desktop** resource strategy, so `install_images` runs the
/// real loader instead of the `UnsupportedResources` stub the other host tests
/// use (they do not exercise images).
fn textured_core(files: Rc<DiskFiles>) -> HostCore {
    let platform = HostPlatform {
        files,
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(DesktopResources),
        capabilities: capabilities(false),
    };
    HostCore::new(options(), platform, RenderSessionHandle::default()).expect("host core")
}

fn textured_mesh() -> Mesh {
    Mesh {
        vertices: vec![p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        texture: Some(TEXTURE_KEY.to_string()),
    }
}

/// A real database with one textured mesh plus the texture that mesh names.
fn textured_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .expect("layer");
    builder
        .insert_texture(TEXTURE_KEY, Arc::from(TEXTURE_PNG))
        .expect("the document owns the texture bytes");
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbMesh".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Mesh(textured_mesh()),
            draw_order: 0,
        })
        .expect("textured mesh");
    builder.finish().expect("database")
}

fn textured_snapshot() -> DocumentSnapshot {
    textured_database()
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(0),
        )
        .expect("snapshot")
}

fn mesh_of(drawing: &DrawingDatabase) -> Mesh {
    let entity = drawing
        .entity(EntityId(1))
        .expect("the textured mesh entity");
    let SemanticGeometry::Mesh(mesh) = &entity.geometry else {
        panic!("expected a mesh, got {:?}", entity.geometry);
    };
    mesh.clone()
}

#[test]
fn a_textured_mesh_survives_a_ymodel_save_and_reopen() {
    let snapshot = textured_snapshot();
    let archive = Arc::new(TextureArchive::new(LoadedArchive {
        manifest: manifest(snapshot.revision),
        snapshot,
    }));
    let files = Rc::new(DiskFiles::default());
    files
        .disk
        .borrow_mut()
        .insert(YMODEL_PATH.to_string(), YMODEL_BYTES.to_vec());

    let core = textured_core(files.clone());
    core.set_document_archive(Some(archive.clone()));
    core.deliver_external_open(target(), Arc::from(YMODEL_BYTES.to_vec()))
        .expect("the textured archive opens");

    // The open funnel decoded the document texture through the desktop strategy:
    // a drawing whose ONLY raster asset is a mesh texture must not look
    // image-free (ADR-0038 D10).
    {
        let report = core
            .last_image_report
            .borrow()
            .clone()
            .expect("install_images ran for a texture-only drawing");
        assert_eq!(report.requested, vec![TEXTURE_KEY.to_string()]);
        assert_eq!(report.loaded, vec![TEXTURE_KEY.to_string()]);
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert_eq!(report.bytes, 16, "2x2 RGBA decoded from the DB bytes");
    }
    // `set_images` only reports a change when the handle really held a cache, so
    // this is the "the cache was attached" half of the statement above.
    assert!(
        core.render.set_images(None),
        "a decoded texture cache must have been installed on the session"
    );

    {
        let drawing = core.controller.borrow().drawing().expect("open drawing");
        assert_eq!(
            drawing.texture(TEXTURE_KEY).map(|bytes| &bytes[..]),
            Some(TEXTURE_PNG),
            "the texture bytes are document state"
        );
        let mesh = mesh_of(drawing.as_ref());
        assert_eq!(mesh.texture.as_deref(), Some(TEXTURE_KEY));
        assert_eq!(mesh.uvs, textured_mesh().uvs);
    }

    // One real edit so the save carries a non-empty scope and a new revision.
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 5.0), p(5.0, 5.0))
        .expect("edit");
    core.request_save().expect("the native save runs");

    let written = files.written.borrow();
    assert_eq!(written.len(), 1, "exactly one `.ymodel` is written");
    assert_eq!(written[0].0, "textured.ymodel");
    assert_eq!(written[0].1, YMODEL_BYTES);
    drop(written);

    // What the host hands the archive: the texture table and the mesh fields.
    let saved = archive
        .saved
        .lock()
        .unwrap()
        .clone()
        .expect("the save captured a snapshot");
    assert_eq!(
        saved.textures.get(TEXTURE_KEY).map(|bytes| &bytes[..]),
        Some(TEXTURE_PNG),
        "the snapshot carries textures/<key> under the mesh's own key"
    );
    assert_eq!(saved.textures.len(), 1, "one content-addressed texture");
    let saved_drawing = DrawingDatabase::from_snapshot(&saved).expect("hydrate the saved snapshot");
    let saved_mesh = mesh_of(&saved_drawing);
    assert_eq!(saved_mesh.texture.as_deref(), Some(TEXTURE_KEY));
    assert_eq!(
        saved_mesh.uvs,
        textured_mesh().uvs,
        "UV0 is part of the saved mesh"
    );

    // Reopen through the host loader: the same bytes, UVs and texture come back.
    core.deliver_external_open(target(), Arc::from(YMODEL_BYTES.to_vec()))
        .expect("the saved archive reopens");
    {
        let drawing = core.controller.borrow().drawing().expect("reopened");
        assert_eq!(
            drawing.texture(TEXTURE_KEY).map(|bytes| &bytes[..]),
            Some(TEXTURE_PNG),
            "save → reopen keeps the texture byte-for-byte"
        );
        let mesh = mesh_of(drawing.as_ref());
        assert_eq!(mesh.texture.as_deref(), Some(TEXTURE_KEY));
        assert_eq!(mesh.uvs, textured_mesh().uvs, "UV0 survives the round trip");
    }
}

/// A mesh naming a texture the document does not carry is refused by name on the
/// existing validation path. This asserts what is already there: `from_snapshot`
/// does **not** re-run referential checks (unchanged behaviour), the entity write
/// path does, with the stable `cad:Mesh.texture:` prefix.
#[test]
fn a_texture_key_without_bytes_is_refused_by_name_on_validation() {
    let db = textured_database();
    let entity = db.entity(EntityId(1)).expect("mesh entity").clone();
    // The archive path: strip the bytes the mesh's key names.
    let mut snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(0),
        )
        .expect("snapshot");
    snapshot.textures.clear();

    // Existing behaviour, pinned rather than changed: hydration accepts the
    // snapshot (it validates the texture *table*, not each mesh reference).
    let hydrated = DrawingDatabase::from_snapshot(&snapshot).expect("hydration succeeds");
    assert!(
        hydrated.texture(TEXTURE_KEY).is_none(),
        "the bytes really are absent"
    );

    let error = cad_db::validate_entity(&hydrated, EntityId(1), &entity)
        .expect_err("a mesh naming an absent texture must not be writable");
    assert!(
        matches!(&error, cad_domain::CadError::InvalidInput(_)),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("cad:Mesh.texture:"),
        "the refusal uses the stable prefix, got {message}"
    );
    assert!(
        message.contains(TEXTURE_KEY),
        "the refusal names the missing key, got {message}"
    );
}
