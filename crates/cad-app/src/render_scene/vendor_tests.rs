//! Derived vendor representation merged into the normal scene assembly.

use super::*;
use crate::render_scene::build_scene_stats_with_vendor;
use crate::vendor::{VendorEntityPrimitives, VendorPrimitive, VendorRepresentationStore};
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    DatabaseId, EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId,
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
