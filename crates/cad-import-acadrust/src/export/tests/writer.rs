//! M1-M7 writer regression coverage: limits, styles, text, dynamic blocks and INSERT.

use super::*;

// ---------------------------------------------------------------------------
// M1/M2/M3/M4/M5/M6/M7 regression coverage.
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

#[test]
fn oversized_polyface_mesh_is_dropped_not_corrupted() {
    // An out-of-range face index must not wrap into a negative (invisible edge)
    // or 0 (unused face) index.
    let bad_index = Mesh {
        vertices: vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
        triangles: vec![[0, 1, 40000]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    let (bytes, report) = export_one(SemanticGeometry::Mesh(bad_index));
    assert!(read_entities(&bytes).is_empty());
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.dropped.polyface_index_range"));

    // Too many vertices is dropped too.
    let too_many = Mesh {
        vertices: (0..32768).map(|i| p(i as f64, 0.0, 0.0)).collect(),
        triangles: Vec::new(),
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    let (bytes, report) = export_one(SemanticGeometry::Mesh(too_many));
    assert!(read_entities(&bytes).is_empty());
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.dropped.polyface_index_range"));
}

#[test]
fn dangling_style_id_is_substituted_and_reported_as_a_conversion() {
    let (bytes, report) = export_one(SemanticGeometry::Text {
        text: "X".into(),
        position: p(0.0, 0.0, 0.0),
        style: StyleId(99),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
    });
    assert_eq!(report.counts.exact, 0, "a substituted style is not exact");
    assert_eq!(report.counts.converted, 1);
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.converted.text" && e.message.contains("Standard")));
    match &read_entities(&bytes)[0] {
        EntityType::Text(text) => assert_eq!(text.style, "Standard"),
        other => panic!("expected TEXT, got {other:?}"),
    }
}

#[test]
fn transparency_and_linetype_scale_are_written() {
    let mut builder = base_builder();
    builder
        .insert_linetype(DbLineType {
            id: LinetypeId(1),
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.5, -0.25]),
            complex: false,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    builder
        .set_entity_render_attributes(
            EntityId(1),
            EntityRenderAttributes {
                transparency: EntityTransparency::Explicit(0.5),
                color: cad_db::EntityColor::ByLayer,
                lineweight: cad_db::EntityLineWeight::ByLayer,
                linetype: EntityLineType::Explicit {
                    name: "DASHED".into(),
                    pattern: LinetypePattern::from_elements([0.5, -0.25]),
                    scale: 2.5,
                },
                geometry_source: GeometrySource::Analytic,
                annotative: Default::default(),
            },
        )
        .unwrap();
    let db = builder.finish().unwrap();
    let (bytes, _) = export_db(&db);
    match &read_entities(&bytes)[0] {
        EntityType::Line(line) => {
            assert_eq!(line.common.transparency, Transparency::Explicit(128));
            assert_eq!(line.common.linetype, "DASHED");
            assert!((line.common.linetype_scale - 2.5).abs() < 1e-12);
        }
        other => panic!("expected LINE, got {other:?}"),
    }
}

#[test]
fn aligned_text_writes_the_alignment_point_and_round_trips() {
    let (bytes, report) = export_one(SemanticGeometry::Text {
        text: "X".into(),
        position: p(3.0, 4.0, 0.0),
        style: StyleId(0),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Center,
        v_align: TextAlignV::Middle,
    });
    assert_eq!(report.counts.exact, 1);
    match &read_entities(&bytes)[0] {
        EntityType::Text(text) => {
            assert_eq!(
                text.alignment_point,
                Some(Vector3::new(3.0, 4.0, 0.0)),
                "non-default alignment must anchor at the alignment point"
            );
        }
        other => panic!("expected TEXT, got {other:?}"),
    }
    // Re-import picks the alignment point as the position (inverse of import).
    let reimported = import_bytes(&bytes);
    let position = reimported
        .entities()
        .find_map(|e| match &e.geometry {
            SemanticGeometry::Text { position, .. } => Some(*position),
            _ => None,
        })
        .expect("text reimports");
    assert_point(position, p(3.0, 4.0, 0.0));
}

#[test]
fn mtext_writes_the_attachment_point() {
    let (bytes, report) = export_one(SemanticGeometry::Text {
        text: "A\nB".into(),
        position: p(1.0, 2.0, 0.0),
        style: StyleId(0),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Right,
        v_align: TextAlignV::Top,
    });
    match &read_entities(&bytes)[0] {
        EntityType::MText(mtext) => {
            assert_eq!(
                mtext.attachment_point,
                acadrust::entities::AttachmentPoint::TopRight
            );
        }
        other => panic!("expected MTEXT, got {other:?}"),
    }
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.converted.text_mtext"));
}

#[test]
fn dynamic_block_visibility_exports_only_visible_members() {
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
    let mut transform = Transform3::identity();
    transform.matrix[0][3] = 10.0;
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform,
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let db = builder.finish().unwrap();
    let (bytes, report) = export_db(&db);
    let entities = read_entities(&bytes);
    assert_eq!(entities.len(), 1, "only the visible member is exported");
    match &entities[0] {
        EntityType::Line(line) => assert_eq!(line.start, Vector3::new(10.0, 0.0, 0.0)),
        other => panic!("expected LINE, got {other:?}"),
    }
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.converted.insert" && e.message.contains("visibility")));
}

#[test]
fn insert_cycle_is_dropped_with_an_explicit_reason() {
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
                transform: Transform3::identity(),
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let db = builder.finish().unwrap();
    let (bytes, report) = export_db(&db);
    assert!(read_entities(&bytes).is_empty());
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.dropped.insert_cycle"));
}

#[test]
fn insert_depth_beyond_the_limit_is_an_explicit_error() {
    let mut builder = base_builder();
    // A -> B -> C (each block's member is an INSERT of the next).
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(11)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(2),
            entities: vec![EntityId(12)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(3),
            entities: vec![EntityId(13)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            11,
            SemanticGeometry::Insert {
                block: BlockId(2),
                transform: Transform3::identity(),
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            12,
            SemanticGeometry::Insert {
                block: BlockId(3),
                transform: Transform3::identity(),
            },
            SpaceId::Block(BlockId(2)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            13,
            SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            SpaceId::Block(BlockId(3)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: Transform3::identity(),
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let db = builder.finish().unwrap();
    let result = export(&ExportRequest {
        database: &db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits {
            max_output_entities: 1_000_000,
            max_explode_depth: 1,
        },
        units: None,
    });
    assert!(matches!(result, Err(CadError::InvalidInput(_))));
}

#[test]
fn insert_applies_rotation_and_scale_in_the_right_order() {
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
    // transform = translate(5,0) * rotate(90deg) * scale(2): (1,0) -> (5,2).
    let scale = Transform3 {
        matrix: [
            [2.0, 0.0, 0.0, 0.0],
            [0.0, 2.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let rotate = Transform3 {
        matrix: [
            [0.0, -1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let transform = Transform3::translation(p(5.0, 0.0, 0.0))
        .matrix_mul(&rotate)
        .matrix_mul(&scale);
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform,
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let db = builder.finish().unwrap();
    let (bytes, _) = export_db(&db);
    match &read_entities(&bytes)[0] {
        EntityType::Line(line) => {
            assert!((line.start - Vector3::new(5.0, 0.0, 0.0)).length() < 1e-9);
            assert!((line.end - Vector3::new(5.0, 2.0, 0.0)).length() < 1e-9);
        }
        other => panic!("expected LINE, got {other:?}"),
    }
}

#[test]
fn unit_context_maps_to_insunits() {
    let db = db_with(vec![SemanticGeometry::Point(p(0.0, 0.0, 0.0))]);
    let (bytes, report) = export(&ExportRequest {
        database: &db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: Some(UnitContext {
            source: Unit::Millimeter,
            display: Unit::Millimeter,
            display_per_source: Some(1.0),
            decimal_places: 3,
        }),
    })
    .unwrap();
    let doc = DxfReader::from_reader(Cursor::new(bytes))
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(doc.header.insertion_units, 4);
    assert!(!report
        .notes
        .iter()
        .any(|note| note.code.starts_with("export.note.units")));
}

#[test]
fn entities_are_emitted_in_draw_order() {
    let mut builder = base_builder();
    for (id, x, order) in [(1u128, 20.0, 2i64), (2u128, 0.0, 0i64), (3u128, 10.0, 1i64)] {
        builder
            .insert_entity(entity(
                id,
                SemanticGeometry::Line {
                    start: p(x, 0.0, 0.0),
                    end: p(x + 1.0, 0.0, 0.0),
                },
                SpaceId::Model,
                order,
            ))
            .unwrap();
    }
    let db = builder.finish().unwrap();
    let (bytes, _) = export_db(&db);
    let starts: Vec<f64> = read_entities(&bytes)
        .iter()
        .filter_map(|e| match e {
            EntityType::Line(line) => Some(line.start.x),
            _ => None,
        })
        .collect();
    assert_eq!(starts, vec![0.0, 10.0, 20.0]);
}
