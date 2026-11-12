//! Derived vendor representation merged into the normal scene assembly.

use super::*;
use crate::render_scene::build_scene_stats_with_vendor;
use crate::vendor::{
    VendorEntityPrimitives, VendorPrimitive, VendorRepresentationStore, VendorText,
};
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    DatabaseId, EntityId, LayerId, LayoutId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId,
};

fn db_with_vendor_entity() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(9));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "TCH_WALL".into(),
                revision: Revision(0),
                source_handle: Some("2C".into()),
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Opaque {
                type_key: "TCH_WALL".into(),
                version: 0,
                payload: Vec::new(),
            },
            draw_order: 7,
        })
        .unwrap();
    builder.finish().unwrap()
}

#[test]
fn vendor_primitives_merge_under_the_source_entity_id() {
    let db = db_with_vendor_entity();
    let store = VendorRepresentationStore::new(
        9,
        vec![VendorEntityPrimitives {
            handle: 0x2C,
            primitives: vec![VendorPrimitive::Lines(vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 5.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 5.0,
                    y: 2.0,
                    z: 0.0,
                },
            ])],
        }],
    );
    let (delta, stats) = build_scene_stats_with_vendor(
        &db,
        TaskStamp::new(DocumentId(73), 0),
        None,
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
        Some(&store),
    )
    .unwrap();
    assert_eq!(stats.fragments_in, 1, "one derived fragment");
    let batch = delta
        .added
        .iter()
        .find(|batch| batch.draw_order == 7)
        .expect("the derived batch");
    assert_eq!(batch.sources.len(), 1);
    assert_eq!(batch.sources[0].entity, EntityId(1));
    assert!(batch.vertices.len() >= 3);
    assert_eq!(batch.topology, cad_scene::RenderTopology::Lines);
}

#[test]
fn a_store_for_another_handle_contributes_nothing() {
    let db = db_with_vendor_entity();
    let miss = VendorRepresentationStore::new(
        10,
        vec![VendorEntityPrimitives {
            handle: 0x2D,
            primitives: vec![VendorPrimitive::Lines(vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 1.0,
                    z: 0.0,
                },
            ])],
        }],
    );
    let (_delta, stats) = build_scene_stats_with_vendor(
        &db,
        TaskStamp::new(DocumentId(73), 0),
        None,
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
        Some(&miss),
    )
    .unwrap();
    assert_eq!(stats.fragments_in, 0);
}

#[test]
#[ignore]
fn a_real_tangent_handle_maps_to_its_source_entity() {
    use cad_import_acadrust::Importer as _;

    let Ok(path) = std::env::var("YACR_TEST_DWG") else {
        return;
    };
    let bytes = std::fs::read(path).expect("read fixture");
    let request = cad_import_acadrust::ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: std::sync::Arc::from(bytes),
        limits: cad_import_acadrust::ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let drawing = cad_import_acadrust::AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("import");
    let overrides = LayerOverrideSet::new();
    let model = drawing.database.model_space();
    let target = model
        .iter()
        .find(|entity| entity.object.type_key == "TCH_WALL")
        .expect("a model-space TCH wall exists");
    let handle = u64::from_str_radix(
        target
            .object
            .source_handle
            .as_deref()
            .expect("the wall has a source handle"),
        16,
    )
    .expect("hex handle");
    // The real fixture imports the TCH 2D symbol blocks by name; the plugin
    // references them by source handle. Both ends must meet here.
    for handle_value in [0x3DD82u64, 0x3E6F4] {
        let block = drawing
            .database
            .block_by_source_handle(handle_value)
            .unwrap_or_else(|| panic!("real block {handle_value:X} is imported"));
        assert!(
            drawing
                .database
                .block(block)
                .is_some_and(|definition| !definition.entities.is_empty()),
            "block {handle_value:X} has member geometry"
        );
    }
    let store = VendorRepresentationStore::new(
        1,
        vec![VendorEntityPrimitives {
            handle,
            primitives: vec![
                VendorPrimitive::Lines(vec![
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
                ]),
                VendorPrimitive::Block {
                    block: 0x3DD82,
                    origin: Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    scale: Point3 {
                        x: 1.0,
                        y: 1.0,
                        z: 1.0,
                    },
                    rotation: 0.0,
                },
            ],
        }],
    );
    let (delta, stats) = build_scene_stats_with_vendor(
        &drawing.database,
        TaskStamp::new(DocumentId(1), 0),
        None,
        None,
        &overrides,
        SpaceSelection::Model,
        Some(&store),
    )
    .expect("scene");
    eprintln!(
        "real handle {:X} entity={} fragments={} batches={}",
        handle,
        target.id.0,
        stats.fragments_in,
        delta.added.len()
    );
    assert!(
        stats.fragments_in >= 2,
        "the real symbol block expanded into fragments"
    );
    assert!(delta.added.iter().any(|batch| batch
        .sources
        .iter()
        .any(|source| source.entity == target.id)));
}

#[test]
fn vendor_text_is_shaped_with_the_host_font_engine() {
    use crate::vendor::VendorText;
    use cad_representation::FontEngine;

    let db = db_with_vendor_entity();
    let store = VendorRepresentationStore::new(
        11,
        vec![VendorEntityPrimitives {
            handle: 0x2C,
            primitives: vec![VendorPrimitive::Text(VendorText {
                origin: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                height: 100.0,
                rotation: 0.0,
                wrap_width: None,
                attachment: 7,
                text: "A".to_string(),
            })],
        }],
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fonts")
        .join("osifont.ttf");
    let bytes = std::fs::read(&path).expect("read osifont.ttf");
    let mut fonts = FontEngine::new();
    fonts
        .register("osifont", std::sync::Arc::from(bytes))
        .expect("register font");
    fonts.set_fallback(vec!["osifont".to_string()]);
    let shaped = fonts
        .shape_unstyled(
            "A",
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            100.0,
            0.0,
            cad_domain::TextAlignH::Left,
            cad_domain::TextAlignV::Baseline,
            None,
            1.0,
        )
        .expect("shape A");
    eprintln!(
        "direct shaping: polylines={} meshes={} renderable={}",
        shaped.polylines.len(),
        shaped.meshes.len(),
        shaped.has_renderable_content
    );
    let (delta, stats) = build_scene_stats_with_vendor(
        &db,
        TaskStamp::new(DocumentId(73), 0),
        Some(std::sync::Arc::new(fonts)),
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
        Some(&store),
    )
    .expect("scene");
    assert!(
        stats.fragments_in > 0,
        "the text produced drawable fragments"
    );
    assert!(delta.added.iter().any(|batch| batch
        .sources
        .iter()
        .any(|source| source.entity == EntityId(1))));
}

/// A model-space source entity (handle `2C`) plus one imported block whose
/// source handle is `0xABCD` and whose single member is a line `(0,0)->(1,0)`.
fn db_with_symbol_block() -> cad_db::DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(10));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_block(cad_db::BlockDefinition {
            id: cad_domain::BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(
            cad_domain::BlockId(1),
            "SYM",
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        )
        .unwrap();
    builder.set_block_source_handle(cad_domain::BlockId(1), 0xABCD);
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(2),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(2),
            layer: LayerId(0),
            space: SpaceId::Block(cad_domain::BlockId(1)),
            geometry: SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            draw_order: 0,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "TCH_OPENING".into(),
                revision: Revision(0),
                source_handle: Some("2C".into()),
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Opaque {
                type_key: "TCH_OPENING".into(),
                version: 0,
                payload: Vec::new(),
            },
            draw_order: 3,
        })
        .unwrap();
    builder.finish().unwrap()
}

#[test]
fn a_vendor_block_reference_expands_the_imported_block() {
    let db = db_with_symbol_block();
    let store = VendorRepresentationStore::new(
        12,
        vec![VendorEntityPrimitives {
            handle: 0x2C,
            primitives: vec![VendorPrimitive::Block {
                block: 0xABCD,
                origin: Point3 {
                    x: 10.0,
                    y: 20.0,
                    z: 0.0,
                },
                scale: Point3 {
                    x: 2.0,
                    y: 2.0,
                    z: 1.0,
                },
                rotation: 0.0,
            }],
        }],
    );
    let (delta, stats) = build_scene_stats_with_vendor(
        &db,
        TaskStamp::new(DocumentId(73), 0),
        None,
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
        Some(&store),
    )
    .expect("scene");
    assert!(
        stats.fragments_in >= 1,
        "the block member became a fragment"
    );
    let batch = delta
        .added
        .iter()
        .find(|batch| {
            batch
                .sources
                .iter()
                .any(|source| source.entity == EntityId(1))
        })
        .expect("the block geometry is sourced to the TCH entity");
    let absolute: Vec<[f64; 2]> = batch
        .vertices
        .iter()
        .map(|vertex| {
            [
                batch.local_origin.x + vertex[0] as f64,
                batch.local_origin.y + vertex[1] as f64,
            ]
        })
        .collect();
    assert!(
        absolute
            .iter()
            .any(|point| { (point[0] - 10.0).abs() < 1e-6 && (point[1] - 20.0).abs() < 1e-6 }),
        "the insert origin is applied: {absolute:?}"
    );
    assert!(
        absolute
            .iter()
            .any(|point| { (point[0] - 12.0).abs() < 1e-6 && (point[1] - 20.0).abs() < 1e-6 }),
        "the x scale is applied: {absolute:?}"
    );
}

#[test]
fn a_vendor_block_reference_with_an_unknown_handle_draws_nothing() {
    let db = db_with_symbol_block();
    let store = VendorRepresentationStore::new(
        13,
        vec![VendorEntityPrimitives {
            handle: 0x2C,
            primitives: vec![VendorPrimitive::Block {
                block: 0xDEAD,
                origin: Point3 {
                    x: 10.0,
                    y: 20.0,
                    z: 0.0,
                },
                scale: Point3 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                },
                rotation: 0.0,
            }],
        }],
    );
    let (_delta, stats) = build_scene_stats_with_vendor(
        &db,
        TaskStamp::new(DocumentId(73), 0),
        None,
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
        Some(&store),
    )
    .expect("scene");
    assert_eq!(stats.fragments_in, 0, "no invented geometry");
}

#[test]
fn a_vendor_entity_inside_a_block_is_expanded_under_its_insert() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(11));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_block(cad_db::BlockDefinition {
            id: cad_domain::BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder.set_block_source_handle(cad_domain::BlockId(1), 0xABCD);
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(2),
                type_key: "TCH_WALL".into(),
                revision: Revision(0),
                source_handle: Some("ABC".into()),
            },
            id: EntityId(2),
            layer: LayerId(0),
            space: SpaceId::Block(cad_domain::BlockId(1)),
            geometry: SemanticGeometry::Opaque {
                type_key: "TCH_WALL".into(),
                version: 0,
                payload: Vec::new(),
            },
            draw_order: 0,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "INSERT".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Insert {
                block: cad_domain::BlockId(1),
                transform: cad_domain::Transform3::translation(Point3 {
                    x: 10.0,
                    y: 0.0,
                    z: 0.0,
                }),
            },
            draw_order: 0,
        })
        .unwrap();
    let db = builder.finish().unwrap();
    let store = VendorRepresentationStore::new(
        14,
        vec![VendorEntityPrimitives {
            handle: 0xABC,
            primitives: vec![VendorPrimitive::Lines(vec![
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
            ])],
        }],
    );
    let (delta, _stats) = build_scene_stats_with_vendor(
        &db,
        TaskStamp::new(DocumentId(74), 0),
        None,
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
        Some(&store),
    )
    .expect("scene");
    let batch = delta
        .added
        .iter()
        .find(|batch| {
            batch.sources.iter().any(|source| {
                source.entity == EntityId(2) && source.instance.0 == vec![EntityId(1)]
            })
        })
        .expect("the block member's derived geometry carries its instance path");
    let absolute: Vec<[f64; 2]> = batch
        .vertices
        .iter()
        .map(|vertex| {
            [
                batch.local_origin.x + vertex[0] as f64,
                batch.local_origin.y + vertex[1] as f64,
            ]
        })
        .collect();
    assert!(absolute
        .iter()
        .any(|point| { (point[0] - 10.0).abs() < 1e-6 && (point[1] - 0.0).abs() < 1e-6 }));
    assert!(absolute
        .iter()
        .any(|point| { (point[0] - 11.0).abs() < 1e-6 && (point[1] - 0.0).abs() < 1e-6 }));
}

#[test]
#[ignore]
fn a_real_block_containing_tch_entities_expands_them() {
    use cad_import_acadrust::Importer as _;

    let Ok(path) = std::env::var("YACR_TEST_DWG") else {
        return;
    };
    let bytes = std::fs::read(path).expect("read fixture");
    let request = cad_import_acadrust::ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: std::sync::Arc::from(bytes),
        limits: cad_import_acadrust::ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let drawing = cad_import_acadrust::AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("import");
    // A model-space INSERT whose block owns a TCH member that the vendor pass
    // also extracted. This is the real shape the walker must reach.
    let vendor_handles: std::collections::HashSet<u64> = drawing
        .vendor_records
        .iter()
        .map(|record| record.handle)
        .collect();
    let mut found: Option<(EntityId, EntityId, u64)> = None;
    for insert in drawing.database.model_space() {
        let SemanticGeometry::Insert { block, .. } = &insert.geometry else {
            continue;
        };
        let Some(definition) = drawing.database.block(*block) else {
            continue;
        };
        for member_id in &definition.entities {
            let Some(member) = drawing.database.entity(*member_id) else {
                continue;
            };
            let Some(handle) = member.object.source_handle.as_deref() else {
                continue;
            };
            let Ok(value) = u64::from_str_radix(handle, 16) else {
                continue;
            };
            if vendor_handles.contains(&value) {
                found = Some((insert.id, member.id, value));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let (insert_id, member_id, handle) = found.expect("a real INSERT owns a TCH member");
    let store = VendorRepresentationStore::new(
        15,
        vec![VendorEntityPrimitives {
            handle,
            primitives: vec![VendorPrimitive::Lines(vec![
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
            ])],
        }],
    );
    let (delta, stats) = build_scene_stats_with_vendor(
        &drawing.database,
        TaskStamp::new(DocumentId(1), 0),
        None,
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
        Some(&store),
    )
    .expect("scene");
    assert!(stats.fragments_in > 0);
    eprintln!(
        "real block insert {:?} member {:?} handle {:X} fragments={}",
        insert_id, member_id, handle, stats.fragments_in
    );
    assert!(delta.added.iter().any(|batch| batch
        .sources
        .iter()
        .any(|source| { source.entity == member_id && source.instance.0 == vec![insert_id] })));
}

// ---- Source-baked `invisible` entities are not assembled ----

fn line(id: u128, space: SpaceId, y: f64, draw_order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry: SemanticGeometry::Line {
            start: Point3 { x: 0.0, y, z: 0.0 },
            end: Point3 { x: 10.0, y, z: 0.0 },
        },
        draw_order,
    }
}

/// Layer 0 only, with model lines 1 (visible) and 2 (source-baked hidden).
fn db_with_baked_hidden_model_entity() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(9));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(line(1, SpaceId::Model, 0.0, 1))
        .unwrap();
    builder
        .insert_entity(line(2, SpaceId::Model, 5.0, 2))
        .unwrap();
    builder.set_hidden_entities([EntityId(2)]).unwrap();
    builder.finish().unwrap()
}

#[test]
fn baked_hidden_model_entity_emits_no_batch() {
    let db = db_with_baked_hidden_model_entity();
    let (delta, stats) = build_scene_stats_with_vendor(
        &db,
        TaskStamp::new(DocumentId(73), 0),
        None,
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
        None,
    )
    .unwrap();
    assert_eq!(stats.fragments_in, 1, "only the visible line is assembled");
    assert!(delta.added.iter().all(|batch| batch
        .sources
        .iter()
        .all(|source| source.entity != EntityId(2))));
}

#[test]
fn baked_hidden_paper_entity_emits_no_batch() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(9));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(cad_db::Layout {
            id: LayoutId(1),
            name: "Sheet".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder
        .insert_entity(line(1, SpaceId::Paper(LayoutId(1)), 0.0, 1))
        .unwrap();
    builder
        .insert_entity(line(2, SpaceId::Paper(LayoutId(1)), 5.0, 2))
        .unwrap();
    builder.set_hidden_entities([EntityId(2)]).unwrap();
    let db = builder.finish().unwrap();

    let (delta, stats) = build_scene_stats_with_vendor(
        &db,
        TaskStamp::new(DocumentId(73), 0),
        None,
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Paper(LayoutId(1)),
        None,
    )
    .unwrap();
    assert_eq!(
        stats.fragments_in, 1,
        "only the visible sheet line is assembled"
    );
    assert!(delta.added.iter().all(|batch| batch
        .sources
        .iter()
        .all(|source| source.entity != EntityId(2))));
}

// ---- TCH `wrapWidth` is decoded but not applied as an auto-wrap width ----

/// The real font the text tests register (`fonts/osifont.ttf`), with the same
/// fallback chain the host uses.
fn vendor_text_engine() -> cad_representation::FontEngine {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fonts")
        .join("osifont.ttf");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("read {} for vendor text tests: {error}", path.display()));
    let mut fonts = cad_representation::FontEngine::new();
    fonts
        .register("osifont", std::sync::Arc::from(bytes))
        .unwrap_or_else(|error| panic!("register osifont.ttf: {error:?}"));
    fonts.set_fallback(vec!["osifont".to_string()]);
    fonts
}

/// Assemble one vendor `Text` primitive onto the shared opaque source entity.
fn assemble_vendor_text(text: &str, height: f64, wrap_width: Option<f64>) -> SceneDelta {
    let db = db_with_vendor_entity();
    let store = VendorRepresentationStore::new(
        11,
        vec![VendorEntityPrimitives {
            handle: 0x2C,
            primitives: vec![VendorPrimitive::Text(VendorText {
                origin: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                height,
                rotation: 0.0,
                wrap_width,
                attachment: 7,
                text: text.to_string(),
            })],
        }],
    );
    let (delta, _stats) = build_scene_stats_with_vendor(
        &db,
        TaskStamp::new(DocumentId(73), 0),
        Some(std::sync::Arc::new(vendor_text_engine())),
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
        Some(&store),
    )
    .expect("assemble vendor text");
    delta
}

/// World-space y extent of the batches owned by source entity 1.
fn vendor_text_y_span(delta: &SceneDelta) -> f64 {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut vertices = 0usize;
    for batch in &delta.added {
        if !batch
            .sources
            .iter()
            .any(|source| source.entity == EntityId(1))
        {
            continue;
        }
        for vertex in &batch.vertices {
            let y = batch.local_origin.y + vertex[1] as f64;
            min = min.min(y);
            max = max.max(y);
            vertices += 1;
        }
    }
    assert!(
        vertices > 0,
        "the vendor text produced no vertices ({} batch(es) in the delta)",
        delta.added.len()
    );
    max - min
}

#[test]
fn tch_wrap_width_does_not_split_a_single_line() {
    // `16.500` at height 100 is wider than the 120 wrap width, so honouring the
    // hint would break it into several lines. Tianzheng's `wrapWidth` is an
    // unverified layout hint; the label must stay one line.
    let delta = assemble_vendor_text("16.500", 100.0, Some(120.0));
    let span = vendor_text_y_span(&delta);
    assert!(
        span < 1.5 * 100.0,
        "wrapWidth must not wrap a single-line label: y span {span} for height 100"
    );
}

#[test]
fn tch_explicit_newline_still_splits_lines() {
    // Explicit `\n` is the real layout mechanism: two lines stay two lines even
    // when a wrap width is present.
    let delta = assemble_vendor_text("A\nB", 100.0, Some(120.0));
    let span = vendor_text_y_span(&delta);
    assert!(
        span >= 1.5 * 100.0,
        "an explicit newline must advance to a second line: y span {span} for height 100"
    );
}
