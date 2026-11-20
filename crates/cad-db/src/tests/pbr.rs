//! Mesh PBR channel contracts (ADR-0039 P1).
//!
//! Four things must hold for the data model to be honest:
//!
//! 1. a declared [`MeshPbr`] is document state: it survives the snapshot and its
//!    texture keys resolve against the document's texture table;
//! 2. every scalar is finite and in `[0, 1]` — refused, never clamped;
//! 3. a PBR texture needs UV0 and a canonical document texture key;
//! 4. `pbr: None` costs nothing: no new requirement, and a transform carries the
//!    material unchanged.

use std::sync::Arc;

use cad_domain::*;

use crate::{
    transform_geometry, validate_geometry, validate_texture_references, DbEntity, DocumentSnapshot,
    DrawingDatabase, DrawingDatabaseBuilder, Layer,
};

/// The base-colour key (ADR-0038) and one key per PBR slot.
const KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";
const MR_KEY: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f0.png";
const NORMAL_KEY: &str = "1f2e3d4c5b6a79880917263544352617.png";
const EMISSIVE_KEY: &str = "2f3e4d5c6b7a89890716253443526170.jpg";
const OCCLUSION_KEY: &str = "3f4e5d6c7b8a9999081726354453627f.png";

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn unit_uvs() -> Vec<[f64; 2]> {
    vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]
}

/// Every scalar and every slot set, so a test that mutates one field is visibly
/// about that field alone.
fn pbr_sample() -> MeshPbr {
    MeshPbr {
        metallic: 0.75,
        roughness: 0.25,
        emissive: [0.0, 0.125, 0.5],
        occlusion_strength: 0.5,
        metallic_roughness_texture: Some(MR_KEY.to_string()),
        normal_texture: Some(NORMAL_KEY.to_string()),
        emissive_texture: Some(EMISSIVE_KEY.to_string()),
        occlusion_texture: Some(OCCLUSION_KEY.to_string()),
    }
}

fn triangle(uvs: Vec<[f64; 2]>, texture: Option<&str>, pbr: Option<MeshPbr>) -> Mesh {
    Mesh {
        vertices: vec![p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs,
        texture: texture.map(str::to_string),
        pbr,
    }
}

fn mesh_with(pbr: MeshPbr) -> SemanticGeometry {
    SemanticGeometry::Mesh(triangle(unit_uvs(), None, Some(pbr)))
}

fn base_builder() -> DrawingDatabaseBuilder {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .expect("layer");
    builder
}

/// A database that holds the base texture **and** all four PBR slots.
fn database_with_pbr_textures() -> DrawingDatabase {
    let mut builder = base_builder();
    for key in [KEY, MR_KEY, NORMAL_KEY, EMISSIVE_KEY, OCCLUSION_KEY] {
        builder
            .insert_texture(key, Arc::from(vec![0x89u8, 0x50, 0x4e, 0x47]))
            .expect("texture");
    }
    builder.finish().expect("database")
}

fn raw_entity(id: u128, geometry: SemanticGeometry) -> DbEntity {
    super::raw_entity(id, SpaceId::Model, geometry)
}

fn snapshot_of(db: &DrawingDatabase) -> DocumentSnapshot {
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(1),
        Vec::new(),
        Revision(1),
    )
    .expect("snapshot")
}

// ---------------------------------------------------------------------------
// Persistence.
// ---------------------------------------------------------------------------

#[test]
fn pbr_material_and_its_textures_round_trip_through_the_snapshot() {
    let mut builder = base_builder();
    for key in [KEY, MR_KEY, NORMAL_KEY, EMISSIVE_KEY, OCCLUSION_KEY] {
        builder
            .insert_texture(key, Arc::from(vec![0x89u8, 0x50, 0x4e, 0x47]))
            .expect("texture");
    }
    builder
        .insert_entity(raw_entity(
            1,
            SemanticGeometry::Mesh(triangle(unit_uvs(), Some(KEY), Some(pbr_sample()))),
        ))
        .expect("entity");
    let db = builder.finish().expect("database");

    let hydrated = DrawingDatabase::from_snapshot(&snapshot_of(&db)).expect("hydrate");
    let entity = hydrated.entity(EntityId(1)).expect("the mesh row");
    let SemanticGeometry::Mesh(mesh) = &entity.geometry else {
        panic!("expected a mesh");
    };
    assert_eq!(mesh.pbr, Some(pbr_sample()), "PBR is document state");
    assert_eq!(mesh.texture.as_deref(), Some(KEY));
    assert_eq!(mesh.uvs, unit_uvs());
    // The slots' bytes are document state too, so the reloaded mesh is sampleable.
    for key in [MR_KEY, NORMAL_KEY, EMISSIVE_KEY, OCCLUSION_KEY] {
        assert!(
            hydrated.texture(key).is_some(),
            "{key} must survive the snapshot"
        );
    }
}

#[test]
fn a_mesh_without_pbr_is_unchanged_by_the_new_rules() {
    let db = base_builder().finish().expect("database");
    let geometry = SemanticGeometry::Mesh(triangle(unit_uvs(), None, None));
    validate_geometry(&geometry).expect("no PBR, no new requirements");
    validate_texture_references(&db, &geometry).expect("nothing to resolve");

    let moved = transform_geometry(&geometry, &Transform3::scale(2.0)).expect("transform");
    let SemanticGeometry::Mesh(mesh) = moved else {
        panic!("expected a mesh");
    };
    assert_eq!(mesh.pbr, None, "a transform never invents a material");
}

// ---------------------------------------------------------------------------
// Validation: scalars.
// ---------------------------------------------------------------------------

#[test]
fn out_of_range_and_non_finite_pbr_scalars_are_refused() {
    for (name, mutate) in [
        (
            "metallic",
            (|pbr: &mut MeshPbr| pbr.metallic = 1.5) as fn(&mut MeshPbr),
        ),
        ("roughness", |pbr: &mut MeshPbr| pbr.roughness = -0.5),
        ("occlusion_strength", |pbr: &mut MeshPbr| {
            pbr.occlusion_strength = 2.0
        }),
    ] {
        let mut pbr = pbr_sample();
        mutate(&mut pbr);
        let error = validate_geometry(&mesh_with(pbr)).expect_err(name);
        assert!(matches!(&error, CadError::InvalidInput(_)), "{error:?}");
        assert!(
            error.to_string().contains("cad:Mesh.pbr:"),
            "{name}: {error}"
        );
        assert!(error.to_string().contains(name), "{name}: {error}");
        assert!(error.to_string().contains("[0, 1]"), "{name}: {error}");
    }

    // Non-finite values are refused as such: the database must not store a value
    // no shader could evaluate.
    for (name, mutate) in [
        (
            "metallic",
            (|pbr: &mut MeshPbr| pbr.metallic = f64::NAN) as fn(&mut MeshPbr),
        ),
        ("roughness", |pbr: &mut MeshPbr| {
            pbr.roughness = f64::INFINITY
        }),
        ("occlusion_strength", |pbr: &mut MeshPbr| {
            pbr.occlusion_strength = f64::NEG_INFINITY
        }),
    ] {
        let mut pbr = pbr_sample();
        mutate(&mut pbr);
        let error = validate_geometry(&mesh_with(pbr)).expect_err(name);
        assert!(
            error.to_string().contains("cad:Mesh.pbr:") && error.to_string().contains("non-finite"),
            "{name}: {error}"
        );
    }
}

#[test]
fn every_emissive_component_is_range_checked() {
    for index in 0..3 {
        let mut pbr = pbr_sample();
        pbr.emissive[index] = 1.25;
        let error = validate_geometry(&mesh_with(pbr)).expect_err("out of range");
        assert!(
            error
                .to_string()
                .contains(&format!("cad:Mesh.pbr: emissive[{index}]")),
            "component {index}: {error}"
        );

        let mut pbr = pbr_sample();
        pbr.emissive[index] = f64::NAN;
        let error = validate_geometry(&mesh_with(pbr)).expect_err("non-finite");
        assert!(
            error.to_string().contains("non-finite"),
            "component {index}: {error}"
        );
    }
}

#[test]
fn the_range_edges_are_accepted() {
    let pbr = MeshPbr {
        metallic: 0.0,
        roughness: 1.0,
        emissive: [0.0, 1.0, 0.0],
        occlusion_strength: 1.0,
        ..MeshPbr::default()
    };
    validate_geometry(&mesh_with(pbr)).expect("0 and 1 are inside the closed range");
}

// ---------------------------------------------------------------------------
// Validation: texture slots.
// ---------------------------------------------------------------------------

#[test]
fn a_pbr_texture_key_must_be_the_canonical_document_form() {
    for key in [
        "",
        "/tmp/normal.png",
        "textures/4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png",
        "4C2F2E0A5F3D9B7C1E8A6D4B2C0F9E71.png",
        "normal.jpeg",
    ] {
        let pbr = MeshPbr {
            normal_texture: Some(key.to_string()),
            ..MeshPbr::default()
        };
        let error = validate_geometry(&mesh_with(pbr)).expect_err(key);
        assert!(
            error.to_string().contains("cad:Mesh.pbr:")
                && error.to_string().contains("not a valid texture key"),
            "{key:?}: {error}"
        );
    }
}

#[test]
fn a_pbr_texture_needs_uv0() {
    // Scalars alone need no parameterisation.
    let scalars_only = MeshPbr {
        metallic: 0.5,
        roughness: 0.5,
        ..MeshPbr::default()
    };
    validate_geometry(&SemanticGeometry::Mesh(triangle(
        Vec::new(),
        None,
        Some(scalars_only),
    )))
    .expect("a scalar-only material has nothing to sample");

    // Every slot is sampled through UV0, so each one requires UVs on its own.
    for slot in [
        (|pbr: &mut MeshPbr| pbr.metallic_roughness_texture = Some(MR_KEY.into()))
            as fn(&mut MeshPbr),
        |pbr: &mut MeshPbr| pbr.normal_texture = Some(NORMAL_KEY.into()),
        |pbr: &mut MeshPbr| pbr.emissive_texture = Some(EMISSIVE_KEY.into()),
        |pbr: &mut MeshPbr| pbr.occlusion_texture = Some(OCCLUSION_KEY.into()),
    ] {
        let mut pbr = MeshPbr::default();
        slot(&mut pbr);
        let error = validate_geometry(&SemanticGeometry::Mesh(triangle(
            Vec::new(),
            None,
            Some(pbr),
        )))
        .expect_err("a texture without UVs cannot be sampled");
        assert!(
            error.to_string().contains("cad:Mesh.pbr:") && error.to_string().contains("needs UV0"),
            "{error}"
        );
    }
}

#[test]
fn a_pbr_texture_the_document_does_not_hold_is_refused() {
    let db = base_builder().finish().expect("database");
    let pbr = MeshPbr {
        normal_texture: Some(NORMAL_KEY.to_string()),
        ..MeshPbr::default()
    };
    let geometry = SemanticGeometry::Mesh(triangle(unit_uvs(), None, Some(pbr)));
    validate_geometry(&geometry).expect("structurally valid");
    let error = validate_texture_references(&db, &geometry).expect_err("no such texture");
    let CadError::InvalidInput(message) = &error else {
        panic!("expected InvalidInput, got {error:?}");
    };
    assert_eq!(
        message,
        &format!("cad:Mesh.pbr: {NORMAL_KEY:?} is not present in the drawing database")
    );

    let db = database_with_pbr_textures();
    validate_texture_references(&db, &geometry).expect("the document holds the slot");
}

#[test]
fn the_reference_check_walks_a_body_display_mesh() {
    let db = base_builder().finish().expect("database");
    let body = SemanticGeometry::Body3d(Box::new(Body3d {
        kind: BodyKind::Sheet,
        exact: BodyExact::Unavailable {
            reason: "no exact geometry".into(),
        },
        display: BodyDisplay {
            mesh: triangle(unit_uvs(), None, Some(pbr_sample())),
            edges: Vec::new(),
            tolerance: None,
            precision: Precision::Analytic,
            completeness: Completeness::Complete,
        },
    }));
    let error = validate_texture_references(&db, &body).expect_err("missing PBR textures");
    assert!(error.to_string().contains("cad:Mesh.pbr:"), "{error}");
}

// ---------------------------------------------------------------------------
// Transform.
// ---------------------------------------------------------------------------

#[test]
fn transform_carries_pbr_unchanged() {
    let source = SemanticGeometry::Mesh(triangle(unit_uvs(), Some(KEY), Some(pbr_sample())));
    let moved = transform_geometry(&source, &Transform3::scale(2.0)).expect("transform");
    let SemanticGeometry::Mesh(mesh) = moved else {
        panic!("expected a mesh");
    };
    assert_eq!(
        mesh.pbr,
        Some(pbr_sample()),
        "PBR is material data, not placement: a similarity must not change it"
    );
    assert_eq!(mesh.texture.as_deref(), Some(KEY));
    assert_eq!(mesh.uvs, unit_uvs());
    assert_eq!(mesh.vertices[1], p(2.0, 0.0));
}
