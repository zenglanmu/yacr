//! Mesh base-colour texture contracts (ADR-0038 A1).
//!
//! Four things must hold for the feature to be honest:
//!
//! 1. a texture key is the canonical `<32 lower hex>.png|.jpg` form or nothing;
//! 2. the bytes are document state — inserted through the builder, carried by
//!    [`DocumentSnapshot`], and readable through `textures()`/`texture()`;
//! 3. `validate_geometry` refuses a mesh whose UV/texture combination could not be
//!    sampled, with the stable `cad:Mesh.uvs:` / `cad:Mesh.texture:` prefixes;
//! 4. `validate_texture_references` refuses a texture the document does not hold,
//!    and `transform_geometry` carries UV/texture through unchanged.

use std::sync::Arc;

use cad_domain::*;

use crate::{
    transform_geometry, validate_entity, validate_geometry, validate_texture_references,
    DocumentSnapshot, DrawingDatabase, DrawingDatabaseBuilder, Layer,
};

/// A canonical key: 32 lowercase hex digits + an extension.
const KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn triangle(uvs: Vec<[f64; 2]>, texture: Option<&str>) -> Mesh {
    Mesh {
        vertices: vec![p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs,
        texture: texture.map(str::to_string),
        // These ADR-0038 contracts are about the base-colour texture only.
        pbr: None,
    }
}

fn unit_uvs() -> Vec<[f64; 2]> {
    vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]
}

fn empty_database(id: u128) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(id));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .expect("layer");
    builder.finish().expect("database")
}

fn database_with_texture() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .expect("layer");
    builder
        .insert_texture(KEY, Arc::from(vec![0x89u8, 0x50, 0x4e, 0x47]))
        .expect("texture");
    builder.finish().expect("database")
}

// ---------------------------------------------------------------------------
// The key form and the builder.
// ---------------------------------------------------------------------------

#[test]
fn insert_texture_refuses_a_key_the_archive_could_not_name() {
    for key in [
        "",
        "/tmp/tex.png",
        "textures/4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png",
        "../4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png",
        "4C2F2E0A5F3D9B7C1E8A6D4B2C0F9E71.png",
        "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.jpeg",
    ] {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        let error = builder
            .insert_texture(key, Arc::from(vec![1u8, 2, 3]))
            .expect_err("an invalid key must be refused");
        assert!(
            matches!(&error, CadError::InvalidInput(_)),
            "{key:?}: {error:?}"
        );
        assert!(
            error.to_string().contains("cad:texture.key:"),
            "{key:?}: {error}"
        );
    }
}

#[test]
fn insert_texture_refuses_an_empty_payload() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    let error = builder
        .insert_texture(KEY, Arc::from(Vec::new()))
        .expect_err("an empty texture cannot decode");
    assert!(error.to_string().contains("cad:texture.empty:"), "{error}");
}

#[test]
fn insert_texture_is_idempotent_for_one_content_addressed_key() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    let bytes: Arc<[u8]> = Arc::from(vec![1u8, 2, 3]);
    builder
        .insert_texture(KEY, Arc::clone(&bytes))
        .expect("first");
    builder
        .insert_texture(KEY, Arc::clone(&bytes))
        .expect("again");
    // Different bytes under one content-addressed key is a caller bug, not a
    // silent replace: the asset a mesh references must not change identity.
    assert!(builder
        .insert_texture(KEY, Arc::from(vec![9u8, 9, 9]))
        .is_err());
    let db = builder.finish().expect("database");
    let stored: Vec<(&str, &Arc<[u8]>)> = db.textures().collect();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].0, KEY);
    assert_eq!(&stored[0].1[..], &bytes[..]);
    assert!(db.texture(KEY).is_some());
    assert!(db.texture("4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e72.png").is_none());
}

#[test]
fn textures_are_part_of_the_snapshot_and_come_back_on_load() {
    let db = database_with_texture();
    let snapshot: DocumentSnapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(3),
        )
        .expect("snapshot");
    assert_eq!(snapshot.textures.len(), 1);
    assert_eq!(&snapshot.textures[KEY][..], &[0x89u8, 0x50, 0x4e, 0x47][..]);
    let hydrated = DrawingDatabase::from_snapshot(&snapshot).expect("hydrate");
    assert_eq!(
        hydrated.texture(KEY).map(|bytes| &bytes[..]),
        Some(&[0x89u8, 0x50, 0x4e, 0x47][..]),
        "the texture bytes are document state and survive a snapshot round trip"
    );
}

#[test]
fn an_invalid_texture_in_a_snapshot_refuses_the_load() {
    // A store-side bug (or a hand-edited archive) must not put an unusable asset in
    // memory: `from_snapshot` re-runs the builder's key/payload checks.
    let db = database_with_texture();
    let mut snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(1),
        )
        .expect("snapshot");
    snapshot.textures.clear();
    snapshot
        .textures
        .insert("../escape.png".to_string(), Arc::from(vec![1u8]));
    let error = DrawingDatabase::from_snapshot(&snapshot).expect_err("refused");
    assert!(
        error.to_string().contains("textures/../escape.png"),
        "{error}"
    );
}

// ---------------------------------------------------------------------------
// Validation.
// ---------------------------------------------------------------------------

#[test]
fn a_correct_textured_mesh_validates() {
    let db = database_with_texture();
    let geometry = SemanticGeometry::Mesh(triangle(unit_uvs(), Some(KEY)));
    validate_geometry(&geometry).expect("UV count matches the vertex count");
    validate_texture_references(&db, &geometry).expect("the document carries the texture");
}

#[test]
fn validate_geometry_rejects_the_uv_violations() {
    // One UV per vertex, or none.
    let error = validate_geometry(&SemanticGeometry::Mesh(triangle(
        vec![[0.0, 0.0], [1.0, 0.0]],
        None,
    )))
    .expect_err("a short UV list cannot be sampled");
    assert!(matches!(&error, CadError::InvalidInput(_)), "{error:?}");
    assert!(
        error
            .to_string()
            .contains("cad:Mesh.uvs: expected one UV per vertex or none"),
        "{error}"
    );

    // Every UV must be finite.
    let error = validate_geometry(&SemanticGeometry::Mesh(triangle(
        vec![[0.0, 0.0], [f64::NAN, 0.0], [0.0, 1.0]],
        None,
    )))
    .expect_err("a non-finite UV is a NaN sample coordinate");
    assert!(error.to_string().contains("cad:Mesh.uvs:"), "{error}");
}

#[test]
fn validate_geometry_rejects_the_texture_violations() {
    // A texture with no UV cannot be sampled.
    let error = validate_geometry(&SemanticGeometry::Mesh(triangle(Vec::new(), Some(KEY))))
        .expect_err("texture without UV0");
    assert!(matches!(&error, CadError::InvalidInput(_)), "{error:?}");
    assert!(error.to_string().contains("cad:Mesh.texture:"), "{error}");

    // The key form is a domain rule, so it is refused without a database too.
    for key in [
        "/tmp/tex.png",
        "../tex.png",
        "4C2F2E0A5F3D9B7C1E8A6D4B2C0F9E71.png",
        "tex.png",
        "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.jpeg",
    ] {
        let error = validate_geometry(&SemanticGeometry::Mesh(triangle(unit_uvs(), Some(key))))
            .expect_err(key);
        assert!(
            error.to_string().contains("cad:Mesh.texture:"),
            "{key}: {error}"
        );
    }
}

#[test]
fn a_texture_the_document_does_not_hold_is_refused() {
    let db = empty_database(1);
    let other = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e72.png";
    let error = validate_texture_references(
        &db,
        &SemanticGeometry::Mesh(triangle(unit_uvs(), Some(other))),
    )
    .expect_err("the texture is not part of this document");
    assert!(matches!(&error, CadError::InvalidInput(_)), "{error:?}");
    let CadError::InvalidInput(message) = &error else {
        unreachable!("checked just above");
    };
    assert_eq!(
        message,
        &format!("cad:Mesh.texture: {other:?} is not present in the drawing database")
    );
}

#[test]
fn the_texture_reference_check_reaches_compound_and_body_display() {
    let db = database_with_texture();
    let textured = SemanticGeometry::Mesh(triangle(unit_uvs(), Some(KEY)));
    // Inside a compound.
    let compound = SemanticGeometry::Compound(vec![
        SemanticGeometry::Line {
            start: p(0.0, 0.0),
            end: p(1.0, 0.0),
        },
        textured.clone(),
    ]);
    validate_texture_references(&db, &compound).expect("the compound member resolves");

    // Inside a body's display cache.
    let body = SemanticGeometry::Body3d(Box::new(Body3d {
        kind: BodyKind::Sheet,
        exact: BodyExact::Unavailable {
            reason: "no exact geometry".into(),
        },
        display: BodyDisplay {
            mesh: triangle(unit_uvs(), Some(KEY)),
            edges: Vec::new(),
            tolerance: None,
            precision: Precision::Analytic,
            completeness: Completeness::Complete,
        },
    }));
    validate_texture_references(&db, &body).expect("the display mesh resolves");

    // The empty database refuses both, so the walk really descends.
    let empty = empty_database(2);
    assert!(validate_texture_references(&empty, &compound).is_err());
    assert!(validate_texture_references(&empty, &body).is_err());
}

#[test]
fn an_entity_write_refuses_a_dangling_texture() {
    // The whole point of the reference check: a mesh naming an image the document
    // does not hold must not become a stored row.
    let entity = super::raw_entity(
        1,
        SpaceId::Model,
        SemanticGeometry::Mesh(triangle(unit_uvs(), Some(KEY))),
    );
    let empty = empty_database(1);
    let error = validate_entity(&empty, EntityId(1), &entity).expect_err("the texture is missing");
    assert!(error.to_string().contains("cad:Mesh.texture:"), "{error}");

    // With the texture present the same row is accepted.
    let db = database_with_texture();
    validate_entity(&db, EntityId(1), &entity).expect("a textured mesh is writable");
}

// ---------------------------------------------------------------------------
// Transform.
// ---------------------------------------------------------------------------

#[test]
fn transform_geometry_carries_uv_and_texture_unchanged() {
    let source = SemanticGeometry::Mesh(triangle(unit_uvs(), Some(KEY)));
    let moved = transform_geometry(&source, &Transform3::scale(2.0)).expect("transform");
    let SemanticGeometry::Mesh(mesh) = moved else {
        panic!("expected a mesh");
    };
    assert_eq!(mesh.texture.as_deref(), Some(KEY));
    assert_eq!(
        mesh.uvs,
        unit_uvs(),
        "UV belongs to the surface, so a similarity must not change it"
    );
    assert_eq!(mesh.vertices[1], p(2.0, 0.0));
}
