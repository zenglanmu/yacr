//! Mesh base-colour textures through a **real `.ymodel` file on disk**.
//!
//! The shared layers can only see `DocumentSnapshot`s, and `cad-store-zip`'s own
//! tests assert its entry plan rather than a file. This test closes the gap the
//! carrier actually ships:
//!
//! ```text
//! DrawingDatabase -> snapshot -> ZipDocumentStore::write -> bytes -> <uuid>.ymodel
//!   -> ZipDocumentStore::read -> LoadedDocument -> DrawingDatabase::from_snapshot
//! ```
//!
//! and it inspects the intermediate artifact as a zip, because `textures/<key>`
//! is an archive-level fact (ADR-0038 D4) that the parsed snapshot alone cannot
//! prove. A second-generation save is checked too: the writer has no raw-copy
//! path for textures, so the bytes must be re-encoded from the snapshot rather
//! than inherited from the previous archive.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use cad_db::store::{MemorySink, MemorySource};
use cad_db::{
    ChangeScope, DbEntity, DbObject, DocumentSnapshot, DocumentStore, DrawingDatabase,
    DrawingDatabaseBuilder, Layer, LoadedDocument,
};
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, Mesh, ObjectId, Point3, Revision,
    SemanticGeometry, SpaceId, UnitContext,
};
use cad_store_zip::ZipDocumentStore;

/// A canonical content-addressed key: 32 lowercase hex + `.png`.
const TEXTURE_KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";

/// A real 2x2 RGBA PNG (red, green, blue, dark), committed inline so the test
/// needs no binary fixture and no encoder dependency.
const TEXTURE_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xb6, 0x0d,
    0x24, 0x00, 0x00, 0x00, 0x15, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x0c, 0x81, 0xf4, 0x7f, 0x2e, 0x11, 0xb9, 0xff, 0x00, 0x41, 0x71, 0x07, 0x36, 0x40, 0xa9,
    0x20, 0x1a, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// A fresh directory under the system temp dir, removed again on drop (no
/// `tempfile` dependency in this crate).
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> TempDir {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "yacr-ymodel-texture-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        TempDir(path)
    }

    /// The archive path for one generation: a UUID-shaped name under the temp
    /// directory, as a carrier would save it.
    fn archive(&self, uuid: &str) -> PathBuf {
        self.0.join(format!("{uuid}.ymodel"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
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
        pbr: None,
    }
}

/// A real database with one textured mesh plus the texture the mesh names.
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

fn snapshot_of(db: &DrawingDatabase, revision: u64) -> DocumentSnapshot {
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(1),
        Vec::new(),
        Revision(revision),
    )
    .expect("snapshot")
}

fn mesh_of(db: &DrawingDatabase) -> Mesh {
    let entity = db.entity(EntityId(1)).expect("the textured mesh entity");
    match &entity.geometry {
        SemanticGeometry::Mesh(mesh) => mesh.clone(),
        other => panic!("expected a mesh, got {other:?}"),
    }
}

/// Write one generation through the store and put the committed bytes in a real
/// file. Returns the bytes that reached the disk.
fn save_archive(path: &Path, snapshot: &DocumentSnapshot, previous: Option<Vec<u8>>) -> Vec<u8> {
    let mut store = ZipDocumentStore::new()
        .with_previous(previous)
        .with_document_uuid(Some("11111111-2222-8333-8444-555555555555".to_string()))
        .with_open_revision(Some(0))
        .with_saved_at(Some("2026-11-20T00:00:00Z".to_string()));
    let mut sink = MemorySink::new();
    store
        .write(snapshot, &ChangeScope::default(), &mut sink)
        .expect("the store writes an archive");
    let bytes = sink
        .committed_bytes()
        .expect("the sink committed the archive")
        .to_vec();
    std::fs::write(path, &bytes).expect("the archive reaches the disk");
    bytes
}

/// Read one generation back from the file, exactly like an open would.
fn load_archive(path: &Path) -> LoadedDocument {
    let bytes = std::fs::read(path).expect("the archive is readable");
    let mut store = ZipDocumentStore::new();
    let mut source = MemorySource::new(bytes);
    store.read(&mut source).expect("the archive parses")
}

/// The central directory names of the file, read as a zip (not as a snapshot).
fn archive_names(path: &Path) -> Vec<String> {
    let file = std::fs::File::open(path).expect("the archive is a readable file");
    let archive = zip::ZipArchive::new(file).expect("the file is a valid zip archive");
    archive
        .file_names()
        .map(|name| name.expect("a UTF-8/CP437 entry name").into_owned())
        .collect()
}

fn texture_entry(names: &[String]) -> String {
    names
        .iter()
        .find(|name| name.starts_with("textures/"))
        .unwrap_or_else(|| panic!("no textures/ entry in {names:?}"))
        .clone()
}

#[test]
fn a_textured_mesh_round_trips_through_a_real_ymodel_file() {
    // The inline PNG must be a real decodable image, or "the bytes survived"
    // would only prove a blob was copied.
    let decoded =
        cad_resources::decode_image(TEXTURE_PNG, &cad_resources::ResourceLimits::default())
            .expect("the fixture decodes as PNG");
    assert_eq!((decoded.width, decoded.height), (2, 2));
    assert_eq!(decoded.byte_len(), 16, "2x2 RGBA");

    let database = textured_database();
    let snapshot = snapshot_of(&database, 0);
    let temp = TempDir::new("round-trip");
    let path = temp.archive("6f1c6a2e-3b9d-4d1f-8f0e-2a5b7c9d1e3f");

    // 1. A real file with real, non-zero archive bytes.
    let written = save_archive(&path, &snapshot, None);
    assert!(!written.is_empty(), "the archive is not empty");
    assert!(
        written.starts_with(b"PK\x03\x04"),
        "the file starts with a zip local-file header"
    );
    let on_disk = std::fs::read(&path).expect("the file read back");
    assert_eq!(
        on_disk, written,
        "the bytes on disk are the committed bytes"
    );

    // 2. The archive, as a zip, really carries the texture entry.
    let names = archive_names(&path);
    let observed = texture_entry(&names);
    assert_eq!(
        observed,
        format!("textures/{TEXTURE_KEY}"),
        "the entry names the content-addressed key"
    );

    // 3. The parsed document has the bytes, the UVs and the key back.
    let loaded = load_archive(&path);
    assert_eq!(loaded.textures.len(), 1, "one content-addressed texture");
    assert_eq!(
        loaded.textures[TEXTURE_KEY].as_ref(),
        TEXTURE_PNG,
        "the texture bytes are identical after the file round trip"
    );
    assert_eq!(
        loaded.snapshot.textures[TEXTURE_KEY].as_ref(),
        TEXTURE_PNG,
        "the snapshot is the authority and carries the same bytes"
    );

    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    assert_eq!(
        reopened.entity_count(),
        database.entity_count(),
        "entity count is unchanged"
    );
    let mesh = mesh_of(&reopened);
    assert_eq!(mesh.texture.as_deref(), Some(TEXTURE_KEY));
    assert_eq!(
        mesh.uvs,
        textured_mesh().uvs,
        "UV0 is part of the saved mesh"
    );
    assert_eq!(
        reopened.texture(TEXTURE_KEY).map(|bytes| &bytes[..]),
        Some(TEXTURE_PNG),
        "the reopened database owns the texture"
    );
}

#[test]
fn a_second_generation_archive_re_encodes_the_texture() {
    let temp = TempDir::new("second-generation");
    let first_path = temp.archive("0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d");
    let second_path = temp.archive("1b2c3d4e-5f6a-4b7c-8d9e-0f1a2b3c4d5e");

    let first_bytes = save_archive(&first_path, &snapshot_of(&textured_database(), 0), None);
    let loaded = load_archive(&first_path);

    // Re-save the *loaded* document, handing the store the previous archive so
    // any raw-copy path it has is available. Textures have none: the writer
    // encodes them from the snapshot every time.
    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    let second_snapshot = snapshot_of(&reopened, 1);
    let second_bytes = save_archive(&second_path, &second_snapshot, Some(first_bytes));

    assert!(!second_bytes.is_empty(), "the second archive is not empty");
    assert_eq!(
        texture_entry(&archive_names(&second_path)),
        format!("textures/{TEXTURE_KEY}"),
        "the second-generation archive still names the texture entry"
    );

    let reloaded = load_archive(&second_path);
    assert_eq!(
        reloaded.textures.len(),
        1,
        "the second generation did not duplicate or drop the texture"
    );
    assert_eq!(
        reloaded.textures[TEXTURE_KEY].as_ref(),
        TEXTURE_PNG,
        "a second-generation save keeps the texture byte-for-byte"
    );

    let reopened_again = DrawingDatabase::from_snapshot(&reloaded.snapshot).expect("hydrate again");
    assert_eq!(
        reopened_again.entity_count(),
        reopened.entity_count(),
        "entity count is unchanged across generations"
    );
    let mesh = mesh_of(&reopened_again);
    assert_eq!(mesh.texture.as_deref(), Some(TEXTURE_KEY));
    assert_eq!(mesh.uvs, textured_mesh().uvs);
}

/// The negative case stays on the existing validation path: a mesh naming a key
/// the document does not carry is refused with the stable `cad:Mesh.texture:`
/// prefix. This test asserts behaviour that already exists; it invents none.
#[test]
fn a_texture_key_without_bytes_is_refused_by_name() {
    let database = textured_database();
    let entity = database
        .entity(EntityId(1))
        .expect("the textured mesh entity")
        .clone();
    let mut snapshot = snapshot_of(&database, 0);
    snapshot.textures.clear();

    let hydrated = DrawingDatabase::from_snapshot(&snapshot).expect("hydration succeeds");
    assert!(
        hydrated.texture(TEXTURE_KEY).is_none(),
        "the bytes really are absent from the archive"
    );

    let error = cad_db::validate_entity(&hydrated, EntityId(1), &entity)
        .expect_err("a mesh naming an absent texture must not be writable");
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
