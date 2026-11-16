//! BLOCKS + INSERT emission and the `XCLIP` clip chain.
//!
//! The v1 exporter exploded every block reference into its members, so a SAVE
//! lost the block's identity, name and base point and could not carry an XCLIP
//! at all. These tests pin the replacement contract: real `BLOCK_RECORD`s, real
//! `INSERT` records naming them, the clip object chain, and a declared reason for
//! every reference that genuinely cannot be one record.
//!
//! Writer + our own reader agreement is **internal consistency**, not a
//! third-party compatibility claim.

use acadrust::entities::Insert as DxfInsert;

use super::*;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn entity(id: u128, geometry: SemanticGeometry, space: SpaceId, draw_order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "test".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order,
    }
}

fn base_builder() -> DrawingDatabaseBuilder {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_style(Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    builder
}

/// `T(translation) · R(rotation radians) · S(scale)` — the placement convention
/// the importer bakes the block base point into.
fn placement(translation: Point3, rotation: f64, scale: [f64; 3]) -> Transform3 {
    let (sin, cos) = rotation.sin_cos();
    let mut linear = Transform3::identity();
    linear.matrix[0][0] = cos * scale[0];
    linear.matrix[1][0] = sin * scale[0];
    linear.matrix[0][1] = -sin * scale[1];
    linear.matrix[1][1] = cos * scale[1];
    linear.matrix[2][2] = scale[2];
    Transform3::translation(translation).matrix_mul(&linear)
}

fn mirror_at(translation: Point3) -> Transform3 {
    let mut mirror = Transform3::identity();
    mirror.matrix[0][0] = -1.0;
    Transform3::translation(translation).matrix_mul(&mirror)
}

fn assert_transform_close(actual: &Transform3, expected: &Transform3, what: &str) {
    for (row, (a, b)) in actual.matrix.iter().zip(expected.matrix.iter()).enumerate() {
        for (column, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            assert!(
                (x - y).abs() < 1e-9,
                "{what}: matrix[{row}][{column}] is {x}, expected {y}"
            );
        }
    }
}

fn assert_vertices_close(actual: &[[f64; 2]], expected: &[[f64; 2]]) {
    assert_eq!(actual.len(), expected.len(), "vertex count: {actual:?}");
    for (a, b) in actual.iter().zip(expected.iter()) {
        assert!(
            (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9,
            "vertex {a:?} != {b:?}"
        );
    }
}

/// Entities owned by one written block definition, in output order.
fn block_entities(document: &acadrust::CadDocument, name: &str) -> Vec<EntityType> {
    let record = document
        .block_records
        .get(name)
        .unwrap_or_else(|| panic!("no BLOCK_RECORD named {name}"));
    record
        .entity_handles
        .iter()
        .filter_map(|handle| document.get_entity(*handle).cloned())
        .collect()
}

/// How many BLOCK_RECORDs carry `name` (a definition must be written once).
fn block_record_count(document: &acadrust::CadDocument, name: &str) -> usize {
    document
        .block_records
        .iter()
        .filter(|record| record.name.eq_ignore_ascii_case(name))
        .count()
}

/// Model-space INSERT records of a written document.
fn model_inserts(document: &acadrust::CadDocument) -> Vec<DxfInsert> {
    let model = document
        .block_records
        .get("*Model_Space")
        .expect("the writer always emits the model-space block record");
    model
        .entity_handles
        .iter()
        .filter_map(|handle| match document.get_entity(*handle) {
            Some(EntityType::Insert(insert)) => Some(insert.clone()),
            _ => None,
        })
        .collect()
}

fn count_records(bytes: &[u8], record: &str) -> usize {
    dxf_pairs(bytes)
        .iter()
        .filter(|(code, value)| *code == 0 && value == record)
        .count()
}

fn read_document(bytes: &[u8]) -> acadrust::CadDocument {
    DxfReader::from_reader(Cursor::new(bytes.to_vec()))
        .unwrap()
        .read()
        .unwrap()
}

fn model_insert_geometries(database: &DrawingDatabase) -> Vec<Transform3> {
    let mut insertions: Vec<(i64, Transform3)> = database
        .entities()
        .filter(|entity| entity.space == SpaceId::Model)
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Insert { transform, .. } => Some((entity.draw_order, *transform)),
            _ => None,
        })
        .collect();
    insertions.sort_by_key(|(order, _)| *order);
    insertions.into_iter().map(|(_, t)| t).collect()
}

// ---------------------------------------------------------------------------
// BLOCKS + INSERT
// ---------------------------------------------------------------------------

#[test]
fn a_named_block_with_two_references_exports_blocks_and_inserts() {
    let mut builder = base_builder();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(1), "GEAR", p(0.0, 0.0, 0.0))
        .unwrap();
    builder
        .insert_entity(entity(
            2,
            SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    // One reference rotated 90°, one scaled 2× and moved.
    let rotated = placement(p(5.0, 0.0, 0.0), std::f64::consts::FRAC_PI_2, [1.0; 3]);
    let scaled = placement(p(20.0, 0.0, 0.0), 0.0, [2.0, 2.0, 1.0]);
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: rotated,
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            3,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: scaled,
            },
            SpaceId::Model,
            1,
        ))
        .unwrap();
    let database = builder.finish().unwrap();

    let (bytes, report) = export_db(&database);
    // No reference converted: block identity is no longer a loss.
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.code != "export.converted.insert"),
        "a representable reference is not a conversion: {:?}",
        report.entries
    );
    let text = std::str::from_utf8(&bytes).expect("ASCII DXF is valid UTF-8");
    assert!(text.contains("BLOCKS"), "the BLOCKS section is written");
    assert_eq!(count_records(&bytes, "INSERT"), 2, "two INSERT records");

    // The definition is written once, with its member in block-local coordinates.
    let document = read_document(&bytes);
    assert_eq!(block_record_count(&document, "GEAR"), 1);
    let members = block_entities(&document, "GEAR");
    assert_eq!(members.len(), 1, "the member is written in block space");
    match &members[0] {
        EntityType::Line(line) => {
            assert_eq!(line.start, Vector3::new(0.0, 0.0, 0.0));
            assert_eq!(line.end, Vector3::new(1.0, 0.0, 0.0));
        }
        other => panic!("expected the block-local LINE, got {other:?}"),
    }
    // Both records name the block and carry their own placement.
    let inserts = model_inserts(&document);
    assert_eq!(inserts.len(), 2);
    assert!(inserts.iter().all(|insert| insert.block_name == "GEAR"));
    assert!((inserts[0].rotation - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    assert!((inserts[0].insert_point - Vector3::new(5.0, 0.0, 0.0)).length() < 1e-9);
    assert!((inserts[1].x_scale() - 2.0).abs() < 1e-9);
    assert!((inserts[1].insert_point - Vector3::new(20.0, 0.0, 0.0)).length() < 1e-9);

    // Re-import puts both instances back where they were, and keeps the name.
    let reimported = import_bytes(&bytes);
    let transforms = model_insert_geometries(&reimported);
    assert_eq!(transforms.len(), 2);
    assert_transform_close(&transforms[0], &rotated, "rotated reference");
    assert_transform_close(&transforms[1], &scaled, "scaled reference");
    let block = reimported
        .blocks()
        .find(|block| reimported.block_name(block.id) == Some("GEAR"))
        .expect("the block definition survives the round trip");
    assert!(reimported
        .block(block.id)
        .is_some_and(|definition| definition.entities.len() == 1));
}

#[test]
fn a_nested_block_reference_round_trips_as_a_nested_insert() {
    let mut builder = base_builder();
    // INNER: one line. OUTER: an INSERT of INNER, rotated inside the definition.
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(2),
            entities: vec![EntityId(3)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(1), "INNER", p(0.0, 0.0, 0.0))
        .unwrap();
    builder
        .set_block_metadata(BlockId(2), "OUTER", p(0.0, 0.0, 0.0))
        .unwrap();
    builder
        .insert_entity(entity(
            2,
            SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    let nested = placement(p(10.0, 0.0, 0.0), std::f64::consts::FRAC_PI_2, [1.0; 3]);
    builder
        .insert_entity(entity(
            3,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: nested,
            },
            SpaceId::Block(BlockId(2)),
            0,
        ))
        .unwrap();
    let outer = placement(p(100.0, 50.0, 0.0), 0.0, [1.0; 3]);
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(2),
                transform: outer,
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let database = builder.finish().unwrap();

    let (bytes, report) = export_db(&database);
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.code != "export.converted.insert"),
        "{:?}",
        report.entries
    );

    let document = read_document(&bytes);
    // Each definition exactly once, even though OUTER's member references INNER.
    assert_eq!(block_record_count(&document, "INNER"), 1);
    assert_eq!(block_record_count(&document, "OUTER"), 1);
    let outer_members = block_entities(&document, "OUTER");
    assert_eq!(outer_members.len(), 1);
    match &outer_members[0] {
        EntityType::Insert(insert) => {
            assert_eq!(insert.block_name, "INNER");
            assert!((insert.rotation - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
            assert!((insert.insert_point - Vector3::new(10.0, 0.0, 0.0)).length() < 1e-9);
        }
        other => panic!("expected the nested INSERT, got {other:?}"),
    }

    // The nested placement survives: the outer instance is in model space and the
    // inner one keeps its block-local placement inside OUTER.
    let reimported = import_bytes(&bytes);
    let transforms = model_insert_geometries(&reimported);
    assert_eq!(transforms.len(), 1, "only OUTER is a model-space reference");
    assert_transform_close(&transforms[0], &outer, "outer reference");
    let nested_transforms: Vec<Transform3> = reimported
        .entities()
        .filter(|entity| matches!(entity.space, SpaceId::Block(_)))
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Insert { transform, .. } => Some(*transform),
            _ => None,
        })
        .collect();
    assert_eq!(nested_transforms.len(), 1);
    assert_transform_close(&nested_transforms[0], &nested, "nested reference");
}

#[test]
fn a_mirrored_reference_still_explodes_with_its_declared_reason() {
    let mut builder = base_builder();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(1), "MIRRORED", p(0.0, 0.0, 0.0))
        .unwrap();
    builder
        .insert_entity(entity(
            2,
            SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: mirror_at(p(4.0, 0.0, 0.0)),
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let database = builder.finish().unwrap();

    let (bytes, report) = export_db(&database);
    let conversions: Vec<&crate::export::ExportDiagnostic> = report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.converted.insert")
        .collect();
    assert_eq!(
        conversions.len(),
        1,
        "exactly one declared conversion: {:?}",
        report.entries
    );
    assert!(
        conversions[0].message.contains("mirrored"),
        "the reason says what cannot be expressed: {}",
        conversions[0].message
    );

    // The members are still exported (exploded), mirrored, as a real LINE.
    let document = read_document(&bytes);
    assert_eq!(count_records(&bytes, "INSERT"), 0);
    assert!(
        document.block_records.get("MIRRORED").is_none(),
        "a reference that cannot be a record must not leave a dead definition behind"
    );
    let lines: Vec<&acadrust::entities::Line> = document
        .entities()
        .filter_map(|entity| match entity {
            EntityType::Line(line) => Some(line),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 1);
    assert!((lines[0].start - Vector3::new(4.0, 0.0, 0.0)).length() < 1e-9);
    assert!((lines[0].end - Vector3::new(3.0, 0.0, 0.0)).length() < 1e-9);
    // The reference was reached, so the definition is not also reported as an
    // unrelated unreferenced block.
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.code != "export.dropped.unreferenced_block"),
        "{:?}",
        report.entries
    );
}

#[test]
fn a_definition_without_a_table_name_still_explodes_with_its_reason() {
    let mut builder = base_builder();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            2,
            SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: placement(p(3.0, 0.0, 0.0), 0.0, [1.0; 3]),
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let database = builder.finish().unwrap();

    let (bytes, report) = export_db(&database);
    assert_eq!(count_records(&bytes, "INSERT"), 0);
    let conversions: Vec<&crate::export::ExportDiagnostic> = report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.converted.insert")
        .collect();
    assert_eq!(conversions.len(), 1, "{:?}", report.entries);
    assert!(
        conversions[0].message.contains("no registered table name"),
        "{}",
        conversions[0].message
    );
    match &read_entities(&bytes)[0] {
        EntityType::Line(line) => {
            assert!((line.start - Vector3::new(3.0, 0.0, 0.0)).length() < 1e-9);
        }
        other => panic!("expected the exploded LINE, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// XCLIP
// ---------------------------------------------------------------------------

/// A database with a named block and one clipped model-space reference.
fn clipped_database(clip: cad_domain::BlockClip) -> DrawingDatabase {
    let mut builder = base_builder();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(1), "WINDOW", p(0.0, 0.0, 0.0))
        .unwrap();
    builder
        .insert_entity(entity(
            2,
            SemanticGeometry::Line {
                start: p(-5.0, -5.0, 0.0),
                end: p(5.0, 5.0, 0.0),
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: placement(p(10.0, 20.0, 0.0), std::f64::consts::FRAC_PI_6, [1.0; 3]),
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    builder
        .set_entity_render_attributes(
            EntityId(1),
            EntityRenderAttributes {
                clip: Some(clip),
                ..Default::default()
            },
        )
        .unwrap();
    builder.finish().unwrap()
}

fn model_clip(database: &DrawingDatabase) -> Option<BlockClip> {
    database
        .entities()
        .filter(|entity| entity.space == SpaceId::Model)
        .find(|entity| matches!(entity.geometry, SemanticGeometry::Insert { .. }))
        .and_then(|entity| database.entity_render_attributes(entity.id).clip)
}

#[test]
fn a_rotated_clipped_reference_round_trips_its_block_local_boundary() {
    let stored = BlockClip {
        vertices: vec![[-2.0, -1.0], [3.0, 4.0]],
        inside: true,
        visible: true,
    };
    let database = clipped_database(stored.clone());
    let (bytes, report) = export_db(&database);
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.code != "export.xclip_clip_dropped"),
        "the clip is written: {:?}",
        report.entries
    );
    assert_eq!(count_records(&bytes, "SPATIAL_FILTER"), 1);
    assert_eq!(count_records(&bytes, "INSERT"), 1);

    let reimported = import_bytes(&bytes);
    let restored = model_clip(&reimported).expect("the boundary comes back");
    assert_vertices_close(&restored.vertices, &stored.vertices);
    assert!(restored.inside);
    assert!(restored.visible);
    assert!(restored.is_active());
}

#[test]
fn an_inverted_clip_is_declared_and_not_written() {
    // The format has no clip-mode field, so an inverted region cannot be stored;
    // the reference is still a real INSERT, and the boundary is refused by name.
    let database = clipped_database(BlockClip {
        vertices: vec![[-2.0, -1.0], [3.0, 4.0]],
        inside: false,
        visible: true,
    });
    let (bytes, report) = export_db(&database);
    let dropped: Vec<&crate::export::ExportDiagnostic> = report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.xclip_clip_dropped")
        .collect();
    assert_eq!(dropped.len(), 1, "{:?}", report.entries);
    assert!(
        dropped[0].message.contains("inverted"),
        "the reason names the missing clip mode: {}",
        dropped[0].message
    );
    assert_eq!(
        count_records(&bytes, "INSERT"),
        1,
        "the reference itself is still written"
    );
    assert_eq!(count_records(&bytes, "SPATIAL_FILTER"), 0);
    let reimported = import_bytes(&bytes);
    assert!(
        model_clip(&reimported).is_none(),
        "an inverted boundary is not silently written as an inside one"
    );
}

#[test]
fn a_clip_on_an_exploded_reference_is_reported_with_the_explosion_reason() {
    let mut builder = base_builder();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(1), "MIRRORED", p(0.0, 0.0, 0.0))
        .unwrap();
    builder
        .insert_entity(entity(
            2,
            SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: mirror_at(p(4.0, 0.0, 0.0)),
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    builder
        .set_entity_render_attributes(
            EntityId(1),
            EntityRenderAttributes {
                clip: Some(BlockClip {
                    vertices: vec![[-1.0, -1.0], [1.0, 1.0]],
                    inside: true,
                    visible: true,
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let database = builder.finish().unwrap();

    let (_, report) = export_db(&database);
    let dropped: Vec<&crate::export::ExportDiagnostic> = report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.xclip_clip_dropped")
        .collect();
    assert_eq!(dropped.len(), 1, "{:?}", report.entries);
    assert!(
        dropped[0].message.contains("exploded") && dropped[0].message.contains("mirrored"),
        "the reason says the reference itself could not be written: {}",
        dropped[0].message
    );
}

// ---------------------------------------------------------------------------
// real fixture
// ---------------------------------------------------------------------------

fn fixture_bytes(path: &str) -> Vec<u8> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read(root.join(path)).expect("fixture readable")
}

#[test]
fn a_real_fixture_keeps_its_block_instances_across_a_save() {
    // `entities.dxf` (QCAD sample) references two named model-space blocks, so
    // this is a real source file rather than a hand-built database.
    let imported = import_bytes(&fixture_bytes("fixtures/dxf/qcad-examples/entities.dxf"));
    let before = model_insert_geometries(&imported);
    assert!(
        !before.is_empty(),
        "precondition: the fixture has model-space block references"
    );
    // The definitions the references name (anonymous, INSERT-unreferenced
    // definitions such as a dimension's `*D` picture block are regenerated by the
    // dimension writer, not preserved by name).
    let names: Vec<String> = imported
        .entities()
        .filter(|entity| entity.space == SpaceId::Model)
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Insert { block, .. } => imported.block_name(*block),
            _ => None,
        })
        .map(str::to_string)
        .collect();
    assert!(
        !names.is_empty(),
        "precondition: the fixture names its blocks"
    );

    let (bytes, report) = export_db(&imported);
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.code != "export.converted.insert"),
        "every reference in the fixture is representable: {:?}",
        report.entries
    );

    let reimported = import_bytes(&bytes);
    let after = model_insert_geometries(&reimported);
    assert_eq!(
        before.len(),
        after.len(),
        "instance count survives the save"
    );
    for (index, (a, b)) in before.iter().zip(after.iter()).enumerate() {
        assert_transform_close(b, a, &format!("fixture instance {index}"));
    }
    for name in names {
        assert!(
            reimported
                .blocks()
                .any(|block| reimported.block_name(block.id) == Some(name.as_str())),
            "block definition '{name}' survives the save"
        );
    }
}

#[test]
fn a_non_uniform_reference_is_written_as_one_insert_not_exploded() {
    // DXF INSERT carries three independent scale factors, so a non-uniform
    // reference is representable; only a mirror/tilt/shear is not.
    let mut builder = base_builder();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(1), "SQUASHED", p(0.0, 0.0, 0.0))
        .unwrap();
    builder
        .insert_entity(entity(
            2,
            SemanticGeometry::Circle {
                center: p(0.0, 0.0, 0.0),
                normal: p(0.0, 0.0, 1.0),
                radius: 1.0,
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    let squashed = placement(p(7.0, -2.0, 0.0), 0.0, [2.0, 3.0, 1.0]);
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: squashed,
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let database = builder.finish().unwrap();

    let (bytes, report) = export_db(&database);
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.code != "export.converted.insert"),
        "{:?}",
        report.entries
    );
    let document = read_document(&bytes);
    let inserts = model_inserts(&document);
    assert_eq!(inserts.len(), 1);
    assert!((inserts[0].x_scale() - 2.0).abs() < 1e-9);
    assert!((inserts[0].y_scale() - 3.0).abs() < 1e-9);
    assert!((inserts[0].z_scale() - 1.0).abs() < 1e-9);

    let reimported = import_bytes(&bytes);
    let transforms = model_insert_geometries(&reimported);
    assert_eq!(transforms.len(), 1);
    assert_transform_close(&transforms[0], &squashed, "non-uniform reference");
}

#[test]
fn a_named_dynamic_block_writes_only_the_active_state_and_declares_it() {
    let mut builder = base_builder();
    for (id, y) in [(2u128, 0.0), (3u128, 1.0)] {
        builder
            .insert_entity(entity(
                id,
                SemanticGeometry::Line {
                    start: p(0.0, y, 0.0),
                    end: p(1.0, y, 0.0),
                },
                SpaceId::Block(BlockId(1)),
                0,
            ))
            .unwrap();
    }
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2), EntityId(3)],
            dynamic_visibility: Some(DynamicBlockVisibility {
                member_entities: vec![EntityId(2), EntityId(3)],
                states: vec![DynamicBlockState {
                    name: "A".into(),
                    entities: vec![EntityId(2)],
                }],
                active_state: Some("A".into()),
            }),
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(1), "DOOR", p(0.0, 0.0, 0.0))
        .unwrap();
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: placement(p(10.0, 0.0, 0.0), 0.0, [1.0; 3]),
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let database = builder.finish().unwrap();

    let (bytes, report) = export_db(&database);
    // One real INSERT, and the reduced definition: only the active state's member.
    assert_eq!(count_records(&bytes, "INSERT"), 1);
    let document = read_document(&bytes);
    assert_eq!(block_entities(&document, "DOOR").len(), 1);
    assert!(
        report.entries.iter().any(|entry| {
            entry.code == "export.converted.insert" && entry.message.contains("visibility")
        }),
        "the reduced definition is declared on the reference: {:?}",
        report.entries
    );
}

#[test]
fn a_named_block_that_contains_itself_does_not_write_a_recursive_definition() {
    let mut builder = base_builder();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(1), "LOOP", p(0.0, 0.0, 0.0))
        .unwrap();
    builder
        .insert_entity(entity(
            2,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: Transform3::identity(),
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: placement(p(5.0, 0.0, 0.0), 0.0, [1.0; 3]),
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let database = builder.finish().unwrap();

    let (bytes, report) = export_db(&database);
    // The export terminates and the definition is not self-referential: a DXF
    // block that contains an INSERT of itself would recurse on load.
    assert!(report
        .entries
        .iter()
        .any(|entry| entry.code == "export.dropped.insert_cycle"));
    let document = read_document(&bytes);
    assert_eq!(
        count_records(&bytes, "INSERT"),
        1,
        "only the model reference"
    );
    assert!(block_entities(&document, "LOOP").is_empty());
    let reimported = import_bytes(&bytes);
    assert_eq!(
        model_insert_geometries(&reimported).len(),
        1,
        "the host reference survives"
    );
}
