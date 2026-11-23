use super::*;
use cad_db::EntityTransparency;
use cad_domain::{BodyDisplay, BodyExact, BodyKind, DatabaseId, DocumentId, Point3, Precision};

fn request() -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: std::sync::Arc::from(Vec::new().into_boxed_slice()),
        limits: crate::ImportLimits::default(),
        generation: 0,
        xref: None,
    }
}

fn triangle() -> Mesh {
    Mesh {
        vertices: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ],
        triangles: vec![[0, 1, 2]],
        ..Mesh::default()
    }
}

#[test]
fn push_mesh_keeps_the_documented_bylayer_defaults() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let object = import.reserve_object().unwrap();
    let id = import
        .push_mesh(object, triangle(), EntityColor::Explicit([1, 2, 3]))
        .unwrap();
    let outcome = import.finish().unwrap();
    let attributes = outcome.database.entity_render_attributes(id);
    assert_eq!(attributes.color, EntityColor::Explicit([1, 2, 3]));
    assert_eq!(attributes.transparency, EntityTransparency::ByLayer);
}

#[test]
fn push_mesh_with_attributes_stores_an_explicit_opacity() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let object = import.reserve_object().unwrap();
    let id = import
        .push_mesh_with_attributes(
            object,
            triangle(),
            EntityRenderAttributes {
                clip: None,
                color: EntityColor::Explicit([10, 20, 30]),
                transparency: EntityTransparency::Explicit(0.5),
                ..Default::default()
            },
        )
        .unwrap();
    let outcome = import.finish().unwrap();
    let attributes = outcome.database.entity_render_attributes(id);
    assert_eq!(attributes.color, EntityColor::Explicit([10, 20, 30]));
    assert_eq!(attributes.transparency, EntityTransparency::Explicit(0.5));
    // Nothing else is invented: the layer keeps its opaque, ACI 7 defaults.
    assert_eq!(outcome.database.entity_count(), 1);
}

// ------------------------------------------------------------ BIM ---

fn bim(connector: &str, id: &str) -> cad_db::BimElement {
    cad_db::BimElement {
        category: Some("IfcWall".into()),
        name: None,
        source: cad_db::BimSource {
            connector: connector.into(),
            document_id: Some("doc".into()),
            id: Some(id.into()),
            native_id: None,
        },
        level: None,
        element_type: None,
        material: None,
        psets: Vec::new(),
        evidence: cad_db::BimEvidence::Declared,
    }
}

#[test]
fn set_entity_bim_reaches_the_finished_database() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let object = import.reserve_object().unwrap();
    let id = import
        .push_mesh(object, triangle(), EntityColor::ByLayer)
        .unwrap();
    import.set_entity_bim(id, bim("ifc", "guid")).unwrap();
    assert_eq!(import.bim_element_count(), 1);
    let outcome = import.finish().unwrap();
    let stored = outcome.database.bim_element(id).expect("BIM block stored");
    assert_eq!(stored.source.id.as_deref(), Some("guid"));
}

#[test]
fn set_entity_bim_for_an_unknown_entity_reports_and_fails() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let error = import
        .set_entity_bim(EntityId(99), bim("ifc", "guid"))
        .expect_err("no entity row owns id 99");
    assert!(error.to_string().contains("ymodel.bim.invalid"), "{error}");
    assert_eq!(import.bim_element_count(), 0, "nothing was stored");
    let outcome = import.finish().unwrap();
    assert!(outcome.database.bim_element(EntityId(99)).is_none());
    assert!(matches!(
        outcome.completeness,
        Completeness::Partial(ref reasons)
            if reasons.iter().any(|reason| reason.contains("BIM block was refused"))
    ));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_BIM_ENTITY_REFUSED));
}

// ------------------------------------------------------------ textures ---

fn png(tail: u8) -> Vec<u8> {
    let mut bytes = PNG_MAGIC.to_vec();
    bytes.extend_from_slice(&[tail, 0x1a, 0x2b, 0x3c]);
    bytes
}

fn jpeg() -> Vec<u8> {
    let mut bytes = JPEG_MAGIC.to_vec();
    bytes.extend_from_slice(&[0xe0, 0x00, 0x10, 0x4a]);
    bytes
}

fn request_with(limits: ImportLimits) -> ImportRequest {
    ImportRequest {
        limits,
        ..request()
    }
}

fn is_texture_key(key: &str, extension: &str) -> bool {
    let Some(stem) = key.strip_suffix(extension) else {
        return false;
    };
    stem.len() == 32
        && stem
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

#[test]
fn a_png_and_a_jpeg_get_deterministic_content_addressed_keys() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let png_key = import.register_texture(&png(0x01)).expect("png");
    let jpeg_key = import.register_texture(&jpeg()).expect("jpeg");
    assert!(is_texture_key(&png_key, ".png"), "{png_key}");
    assert!(is_texture_key(&jpeg_key, ".jpg"), "{jpeg_key}");
    assert_ne!(png_key, jpeg_key, "different bytes, different keys");

    // Same bytes, fresh import: the key is a function of the bytes alone, so a
    // saved document stays addressable across sessions.
    let mut second = MeshImport::new(&request(), "TestMesh").unwrap();
    assert_eq!(second.register_texture(&png(0x01)).expect("png"), png_key);
    assert_ne!(
        second.register_texture(&png(0x02)).expect("other png"),
        png_key
    );
}

#[test]
fn a_non_image_is_refused_by_name() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    for bytes in [
        Vec::new(),
        b"not an image".to_vec(),
        // A truncated PNG signature must not pass the sniff.
        PNG_MAGIC[..7].to_vec(),
        // A two-byte JPEG prefix is not a JPEG either.
        JPEG_MAGIC[..2].to_vec(),
    ] {
        let error = import.register_texture(&bytes).expect_err("not a PNG/JPEG");
        assert!(
            error
                .to_string()
                .contains("import.texture.unsupported_format"),
            "{error}"
        );
    }
    assert_eq!(import.textures().count(), 0, "a refusal changes no state");
}

#[test]
fn identical_bytes_are_stored_once() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let first = import.register_texture(&png(0x07)).expect("first");
    let second = import.register_texture(&png(0x07)).expect("second");
    assert_eq!(first, second);
    assert_eq!(import.textures().count(), 1, "one copy, one key");
    // The second registration must not consume budget a second time: the whole
    // document budget is already spent by the one copy.
    let mut tight = MeshImport::new(
        &request_with(ImportLimits {
            max_texture_bytes: png(0x07).len(),
            max_total_texture_bytes: png(0x07).len(),
            max_textures: 1,
            ..ImportLimits::default()
        }),
        "TestMesh",
    )
    .unwrap();
    assert!(tight.register_texture(&png(0x07)).is_ok());
    assert!(
        tight.register_texture(&png(0x07)).is_ok(),
        "a duplicate is not charged twice"
    );
}

#[test]
fn each_texture_budget_is_a_named_refusal_that_keeps_state() {
    // Per-image budget.
    let mut import = MeshImport::new(
        &request_with(ImportLimits {
            max_texture_bytes: 4,
            ..ImportLimits::default()
        }),
        "TestMesh",
    )
    .unwrap();
    let error = import.register_texture(&png(0x01)).expect_err("too large");
    assert!(
        error.to_string().contains("import.texture.too_large"),
        "{error}"
    );
    assert_eq!(import.textures().count(), 0);

    // Per-document total budget: each image fits, the sum does not.
    let one = png(0x01).len();
    let mut import = MeshImport::new(
        &request_with(ImportLimits {
            max_total_texture_bytes: one + 1,
            ..ImportLimits::default()
        }),
        "TestMesh",
    )
    .unwrap();
    import.register_texture(&png(0x01)).expect("first fits");
    let error = import.register_texture(&png(0x02)).expect_err("budget");
    assert!(
        error.to_string().contains("import.texture.budget_exceeded"),
        "{error}"
    );
    assert_eq!(import.textures().count(), 1, "the first texture stays");

    // Texture count budget.
    let mut import = MeshImport::new(
        &request_with(ImportLimits {
            max_textures: 1,
            ..ImportLimits::default()
        }),
        "TestMesh",
    )
    .unwrap();
    import.register_texture(&png(0x01)).expect("first");
    let error = import.register_texture(&png(0x02)).expect_err("too many");
    assert!(
        error.to_string().contains("import.texture.too_many"),
        "{error}"
    );
    assert_eq!(import.textures().count(), 1);
}

#[test]
fn finish_carries_the_bytes_into_the_database() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let key = import.register_texture(&png(0x01)).expect("png");
    // Registered but referenced by no mesh: the importer decided the asset
    // belongs to the document, so it is still carried.
    let outcome = import.finish().expect("finish");
    assert_eq!(
        outcome.database.texture(&key).map(|bytes| &bytes[..]),
        Some(&png(0x01)[..])
    );
    assert_eq!(outcome.database.textures().count(), 1);
}

#[test]
fn a_textured_mesh_reaches_the_finished_database() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let key = import.register_texture(&png(0x01)).expect("png");
    let object = import.reserve_object().unwrap();
    let mut mesh = triangle();
    mesh.uvs = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    mesh.texture = Some(key.clone());
    import
        .push_mesh(object, mesh, EntityColor::Explicit([1, 2, 3]))
        .expect("a textured mesh validates");
    let outcome = import.finish().expect("finish");
    let entity = outcome
        .database
        .entities()
        .find(|entity| entity.id == EntityId(1))
        .expect("the mesh row");
    let SemanticGeometry::Mesh(mesh) = &entity.geometry else {
        panic!("expected a mesh");
    };
    assert_eq!(mesh.texture.as_deref(), Some(key.as_str()));
    assert_eq!(mesh.uvs.len(), 3);
    assert!(outcome.database.texture(&key).is_some());
}

// ------------------------------------------------------------- bodies ---

fn edge_polyline() -> Vec<Point3> {
    vec![
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    ]
}

fn body(kind: BodyKind, mesh: Mesh, edges: Vec<Vec<Point3>>) -> Body3d {
    Body3d {
        kind,
        exact: BodyExact::Unavailable {
            reason: "the source supplied display geometry only".into(),
        },
        display: BodyDisplay {
            mesh,
            edges,
            tolerance: None,
            precision: Precision::Unknown,
            completeness: Completeness::Complete,
        },
    }
}

#[test]
fn push_body_with_attributes_keeps_the_display_edges() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let object = import.reserve_object().unwrap();
    let id = import
        .push_body_with_attributes(
            object,
            body(BodyKind::Sheet, triangle(), vec![edge_polyline()]),
            "TestBody",
            EntityRenderAttributes::default(),
        )
        .unwrap();
    let outcome = import.finish().unwrap();
    let entity = outcome
        .database
        .entities()
        .find(|entity| entity.id == id)
        .expect("the body row");
    let SemanticGeometry::Body3d(stored) = &entity.geometry else {
        panic!("expected a body");
    };
    assert_eq!(stored.display.edges.len(), 1, "display edges survive");
    assert_eq!(stored.display.edges[0].len(), 2);
    assert_eq!(stored.display.mesh.triangles.len(), 1);
    assert_eq!(entity.object.type_key, "TestBody");
}

#[test]
fn a_wire_body_without_triangles_inserts_from_edges_alone() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let object = import.reserve_object().unwrap();
    let id = import
        .push_body(
            object,
            body(BodyKind::Wire, Mesh::default(), vec![edge_polyline()]),
            "TestWire",
        )
        .unwrap();
    let outcome = import.finish().unwrap();
    let entity = outcome
        .database
        .entities()
        .find(|entity| entity.id == id)
        .expect("the wire row");
    let SemanticGeometry::Body3d(stored) = &entity.geometry else {
        panic!("expected a body");
    };
    assert!(stored.display.mesh.triangles.is_empty());
    assert_eq!(stored.display.edges, vec![edge_polyline()]);
}

#[test]
fn a_body_with_no_drawable_geometry_is_refused_by_name() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let object = import.reserve_object().unwrap();
    let error = import
        .push_body(
            object,
            body(BodyKind::Unknown, Mesh::default(), Vec::new()),
            "TestEmpty",
        )
        .expect_err("nothing is drawable");
    match error {
        CadError::InvalidInput(message) => {
            assert!(message.contains("TestEmpty"), "{message}");
            assert!(message.contains("no drawable geometry"), "{message}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    assert_eq!(import.entity_count(), 0, "a refusal inserts no row");
}

#[test]
fn each_inserted_type_key_gets_its_own_capability_row() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();

    let mesh_object = import.reserve_object().unwrap();
    import
        .push_mesh(mesh_object, triangle(), EntityColor::ByLayer)
        .unwrap();

    let body_object = import.reserve_object().unwrap();
    import
        .push_body(
            body_object,
            body(BodyKind::Wire, Mesh::default(), vec![edge_polyline()]),
            "TestWire",
        )
        .unwrap();

    let outcome = import.finish().unwrap();
    assert_eq!(outcome.capabilities.len(), 2);
    let keys: Vec<&str> = outcome
        .capabilities
        .iter()
        .map(|capability| capability.type_key.as_str())
        .collect();
    assert_eq!(keys, vec!["TestMesh", "TestWire"], "sorted by key");
    for capability in &outcome.capabilities {
        assert_eq!(capability.read, SupportStatus::Verified);
        assert_eq!(capability.semantic, SupportStatus::Verified);
        assert_eq!(capability.render, SupportStatus::Verified);
        assert_eq!(capability.pick, SupportStatus::Verified);
        assert_eq!(capability.measure, SupportStatus::Unsupported);
    }
}

#[test]
fn a_single_key_import_still_reports_exactly_one_capability_row() {
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let object = import.reserve_object().unwrap();
    import
        .push_mesh(object, triangle(), EntityColor::ByLayer)
        .unwrap();
    let outcome = import.finish().unwrap();
    assert_eq!(outcome.capabilities.len(), 1);
    assert_eq!(outcome.capabilities[0].type_key, "TestMesh");
}

#[test]
fn an_import_that_inserted_nothing_reports_no_capability_row() {
    // The primary key is not seeded: a capability row must name a carrier
    // this import actually stored (the SKP edge-only case used to report a
    // `SkpPreviewMesh` row without ever inserting one).
    let import = MeshImport::new(&request(), "TestMesh").unwrap();
    let outcome = import.finish().unwrap();
    assert!(
        outcome.capabilities.is_empty(),
        "no inserted geometry, no capability row: {:?}",
        outcome
            .capabilities
            .iter()
            .map(|capability| &capability.type_key)
            .collect::<Vec<_>>()
    );
    assert_eq!(outcome.database.entity_count(), 0);
}

#[test]
fn only_the_body_key_of_an_edge_only_insert_is_reported() {
    // The edge-only shape: no mesh row, one body row for the carrier used.
    let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
    let object = import.reserve_object().unwrap();
    import
        .push_body(
            object,
            body(BodyKind::Wire, Mesh::default(), vec![edge_polyline()]),
            "TestWire",
        )
        .unwrap();
    let outcome = import.finish().unwrap();
    let keys: Vec<&str> = outcome
        .capabilities
        .iter()
        .map(|capability| capability.type_key.as_str())
        .collect();
    assert_eq!(keys, vec!["TestWire"], "the unused mesh key is absent");
}

// ------------------------------------------------ edge-snap measure claim ---

fn measure_of(outcome: &MeshImportOutcome, type_key: &str) -> SupportStatus {
    outcome
        .capabilities
        .iter()
        .find(|capability| capability.type_key == type_key)
        .unwrap_or_else(|| panic!("no capability row for {type_key}"))
        .measure
}

#[test]
fn stored_display_edges_stay_unsupported_without_an_explicit_declaration() {
    // Default false: a body with real display edges is still not a MEASURE
    // source unless the importer declares the claim.
    let mut import = MeshImport::new(&request(), "TestBody").unwrap();
    let object = import.reserve_object().unwrap();
    import
        .push_body(
            object,
            body(BodyKind::Wire, Mesh::default(), vec![edge_polyline()]),
            "TestBody",
        )
        .unwrap();
    let outcome = import.finish().unwrap();
    assert_eq!(measure_of(&outcome, "TestBody"), SupportStatus::Unsupported);
}

#[test]
fn declared_edge_snap_measure_is_partial_only_for_keys_that_stored_edges() {
    // A triangle-only carrier: drawable and pickable, still not measurable.
    let mut triangles = MeshImport::new(&request(), "SkpPreviewMesh").unwrap();
    triangles.declare_edge_snap_measure(true);
    let mesh_object = triangles.reserve_object().unwrap();
    triangles
        .push_mesh(mesh_object, triangle(), EntityColor::ByLayer)
        .unwrap();
    let triangles = triangles.finish().unwrap();
    assert_eq!(
        measure_of(&triangles, "SkpPreviewMesh"),
        SupportStatus::Unsupported,
        "triangles alone never claim measurement"
    );

    // An edge-bearing face-group body under the same key (the SKP face-group
    // body shape) does claim the narrow endpoint/midpoint source.
    let mut mesh_body = MeshImport::new(&request(), "SkpPreviewMesh").unwrap();
    mesh_body.declare_edge_snap_measure(true);
    let mesh_body_object = mesh_body.reserve_object().unwrap();
    mesh_body
        .push_body(
            mesh_body_object,
            body(BodyKind::Unknown, triangle(), vec![edge_polyline()]),
            "SkpPreviewMesh",
        )
        .unwrap();
    let mesh_body = mesh_body.finish().unwrap();
    assert_eq!(
        measure_of(&mesh_body, "SkpPreviewMesh"),
        SupportStatus::Partial,
        "a body that stored display edges claims endpoint/midpoint snap"
    );

    // A wire-only carrier (the SKP edge-only shape).
    let mut wire = MeshImport::new(&request(), "SkpPreviewEdges").unwrap();
    wire.declare_edge_snap_measure(true);
    let wire_object = wire.reserve_object().unwrap();
    wire.push_body(
        wire_object,
        body(BodyKind::Wire, Mesh::default(), vec![edge_polyline()]),
        "SkpPreviewEdges",
    )
    .unwrap();
    let wire = wire.finish().unwrap();
    assert_eq!(
        measure_of(&wire, "SkpPreviewEdges"),
        SupportStatus::Partial,
        "wire bodies store display edges too"
    );
}
