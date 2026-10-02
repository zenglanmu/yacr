//! Unit tests.

use super::*;
use cad_domain::*;

fn ann(id: u128) -> Annotation {
    Annotation {
        id: AnnotationId(id),
        space: SpaceId::Model,
        geometry: AnnotationGeometry::Text(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }),
        text: "note".to_string(),
        style: AnnotationStyle::default(),
        created_unix_ms: 0,
        modified_unix_ms: 0,
        anchor: None,
        precision: Precision::Analytic,
    }
}

#[test]
fn plot_settings_are_optional_and_default_to_an_explicit_page() {
    use crate::tables::{
        PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation, PlotSettingsRecord, PlotType,
    };

    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(crate::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    let db = b.finish().unwrap();
    // No stored record: the query returns an explicit documented default, and
    // the raw accessor stays honest about the absence.
    assert!(db.plot_settings(LayoutId(1)).is_none());
    let fallback = db.plot_settings_for(LayoutId(1));
    assert!(matches!(
        fallback.provenance,
        PlotProvenance::DefaultPage { .. }
    ));
    assert_eq!(fallback.paper_width, 210.0);
    assert_eq!(fallback.paper_height, 297.0);

    // A stored record round-trips exactly, including rotation and margins.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(crate::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.set_plot_settings(PlotSettingsRecord {
        layout: LayoutId(1),
        paper_size_name: "ISO_A3".into(),
        paper_width: 297.0,
        paper_height: 420.0,
        margins: PlotMargins::uniform(12.0),
        rotation: PlotRotation::Degrees90,
        scale_numerator: 1.0,
        scale_denominator: 100.0,
        plot_type: PlotType::Layout,
        paper_units: PlotPaperUnits::Millimeters,
        provenance: PlotProvenance::Imported,
    })
    .unwrap();
    let db = b.finish().unwrap();
    let stored = db.plot_settings(LayoutId(1)).unwrap();
    assert_eq!(stored.rotation.to_degrees(), 90.0);
    assert_eq!(stored.rotated_size(), (420.0, 297.0));
    assert_eq!(stored.margins.horizontal_total(), 24.0);
    assert_eq!(stored.margins.vertical_total(), 24.0);
}

#[test]
fn plot_settings_for_an_unknown_layout_are_rejected() {
    use crate::tables::{
        PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation, PlotSettingsRecord, PlotType,
    };
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // No layout inserted: attaching settings would be silently unreachable.
    let result = b.set_plot_settings(PlotSettingsRecord {
        layout: LayoutId(7),
        paper_size_name: String::new(),
        paper_width: 210.0,
        paper_height: 297.0,
        margins: PlotMargins::default(),
        rotation: PlotRotation::None,
        scale_numerator: 1.0,
        scale_denominator: 1.0,
        plot_type: PlotType::Layout,
        paper_units: PlotPaperUnits::Millimeters,
        provenance: PlotProvenance::Imported,
    });
    assert!(result.is_err());
}

#[test]
fn transaction_commit_raises_revision_and_is_ordered() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let tx = db.begin("create text", TransactionId(7)).unwrap();
    let mut tx = tx;
    tx.insert_annotation(ann(10)).unwrap();
    let changes = tx.commit().unwrap();
    assert_eq!(changes.before, Revision(0));
    assert_eq!(changes.after, Revision(1));
    assert_eq!(changes.changes, vec![ObjectChange::Insert(ObjectId(10))]);
    assert_eq!(db.revision(), Revision(1));
    assert!(db.is_dirty());
    assert!(changes.follows(DatabaseId(1), Revision(0)));
}

#[test]
fn dropped_transaction_leaves_no_state() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    {
        let mut tx = db.begin("edit", TransactionId(1)).unwrap();
        tx.insert_annotation(ann(10)).unwrap();
        tx.rollback();
    }
    assert_eq!(db.len(), 0);
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn failed_transaction_does_not_change_revision() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let mut tx = db.begin("delete missing", TransactionId(1)).unwrap();
    assert!(tx.delete_annotation(AnnotationId(99)).is_err());
    // Nothing valid was staged, so commit is a no-op rather than a bump.
    let changes = tx.commit().unwrap();
    assert!(changes.is_empty());
    assert_eq!(changes.before, changes.after);
    assert_eq!(db.revision(), Revision(0));
    assert_eq!(db.len(), 0);
}

#[test]
fn double_modification_in_one_transaction_is_rejected() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let mut tx = db.begin("x", TransactionId(1)).unwrap();
    tx.insert_annotation(ann(1)).unwrap();
    // Second staged change for the same id is allowed at stage time but the
    // commit rejects it as an invariant violation.
    tx.staged.insert(AnnotationId(1), None);
    assert!(tx.commit().is_err());
    assert_eq!(db.len(), 0);
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn export_marking_requires_a_real_revision() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    assert!(db.mark_exported(Revision(5)).is_err());
    db.apply_annotation_changes("a", TransactionId(1), vec![(AnnotationId(1), Some(ann(1)))])
        .unwrap();
    assert!(db.is_dirty());
    db.mark_exported(Revision(1)).unwrap();
    assert!(!db.is_dirty());
}

#[test]
fn builder_rejects_dangling_layer_reference() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(42),
        space: SpaceId::Model,
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
    assert!(matches!(b.finish(), Err(CadError::Invariant(_))));
}

#[test]
fn builder_accepts_consistent_database_and_computes_bounds() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: Some("1A".into()),
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 3.0,
                y: 4.0,
                z: 0.0,
            },
        },
        draw_order: 0,
    })
    .unwrap();
    let db = b.finish().unwrap();
    assert_eq!(db.entity_count(), 1);
    let (min, max) = db.bounds().unwrap();
    assert_eq!(min.x, 0.0);
    assert_eq!(max.x, 3.0);
    assert_eq!(max.y, 4.0);
}

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn raw_entity(id: u128, space: SpaceId, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order: id as i64,
    }
}

fn insert_at(block: u128, dx: f64) -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: BlockId(block),
        transform: Transform3::translation(point(dx, 0.0)),
    }
}

#[test]
fn block_entities_are_not_model_space_and_inserts_expand_in_bounds() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // Block 0 owns a unit line; two model inserts place it at x=10 and x=20.
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(2)],
    })
    .unwrap();
    b.insert_entity(raw_entity(
        2,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    ))
    .unwrap();
    b.insert_entity(raw_entity(1, SpaceId::Model, insert_at(0, 10.0)))
        .unwrap();
    b.insert_entity(raw_entity(3, SpaceId::Model, insert_at(0, 20.0)))
        .unwrap();
    let db = b.finish().unwrap();

    // The block definition is not drawn top-level (audit B15).
    assert_eq!(db.model_space().len(), 2);
    assert_eq!(db.block_entities(BlockId(0)).len(), 1);
    // Bounds expand both instances: 10 .. 21.
    let (min, max) = db.bounds().unwrap();
    assert_eq!(min.x, 10.0);
    assert_eq!(max.x, 21.0);
}

#[test]
fn cyclic_block_reference_does_not_loop_bounds() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // Block 0 inserts itself.
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(2)],
    })
    .unwrap();
    b.insert_entity(raw_entity(2, SpaceId::Block(BlockId(0)), insert_at(0, 1.0)))
        .unwrap();
    b.insert_entity(raw_entity(1, SpaceId::Model, insert_at(0, 0.0)))
        .unwrap();
    let db = b.finish().unwrap();
    // Must terminate; a self-referential block contributes no bounds.
    assert!(db.bounds().is_none());
}

#[test]
fn scene_identity_distinguishes_same_id_different_content() {
    // Two databases that reuse DatabaseId(1) but hold different drawings
    // must not compare equal, or a host would keep stale GPU batches on
    // screen after opening the second one (audit B04).
    let make = |end_x: f64| {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: Some("1A".into()),
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: end_x,
                    y: 0.0,
                    z: 0.0,
                },
            },
            draw_order: 0,
        })
        .unwrap();
        b.finish().unwrap()
    };
    let a = make(10.0);
    let b = make(20.0);
    assert_eq!(a.id(), b.id(), "same DatabaseId");
    assert_ne!(a.scene_identity(), b.scene_identity());
    assert_eq!(a.scene_identity(), a.scene_identity());
}

#[test]
fn render_attributes_default_to_by_layer_and_opaque() {
    // A database with no recorded attributes (hand-built fixtures) must be
    // fully opaque and use the documented ByLayer colour/lineweight, never a
    // fabricated explicit value.
    let attributes = EntityRenderAttributes::default();
    assert_eq!(attributes.transparency, EntityTransparency::Explicit(1.0));
    assert_eq!(attributes.color, EntityColor::ByLayer);
    assert_eq!(attributes.lineweight, EntityLineWeight::ByLayer);
    assert_eq!(attributes.geometry_source, GeometrySource::Analytic);
}

#[test]
fn render_attributes_round_trip_through_the_database() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(raw_entity(
        1,
        SpaceId::Model,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    ))
    .unwrap();
    // ByObject colour/lineweight survive verbatim; ByBlock stays symbolic.
    b.set_entity_render_attributes(
        EntityId(1),
        EntityRenderAttributes {
            transparency: EntityTransparency::Explicit(0.5),
            color: EntityColor::Explicit([10, 20, 30]),
            lineweight: EntityLineWeight::Explicit(0.35),
            geometry_source: GeometrySource::Analytic,
        },
    )
    .unwrap();
    // An entity with unknown attributes falls back to the defaults.
    b.insert_entity(raw_entity(
        2,
        SpaceId::Model,
        SemanticGeometry::Point(point(2.0, 2.0)),
    ))
    .unwrap();
    let db = b.finish().unwrap();

    let one = db.entity_render_attributes(EntityId(1));
    assert_eq!(one.color, EntityColor::Explicit([10, 20, 30]));
    assert_eq!(one.lineweight, EntityLineWeight::Explicit(0.35));
    assert_eq!(
        db.entity_render_attributes(EntityId(2)).color,
        EntityColor::ByLayer
    );
}

#[test]
fn byblock_color_and_lineweight_stay_symbolic() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(raw_entity(
        1,
        SpaceId::Model,
        SemanticGeometry::Point(point(0.0, 0.0)),
    ))
    .unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        EntityRenderAttributes {
            transparency: EntityTransparency::ByBlock,
            color: EntityColor::ByBlock,
            lineweight: EntityLineWeight::ByBlock,
            geometry_source: GeometrySource::Analytic,
        },
    )
    .unwrap();
    let db = b.finish().unwrap();
    let attributes = db.entity_render_attributes(EntityId(1));
    assert_eq!(attributes.color, EntityColor::ByBlock);
    assert_eq!(attributes.lineweight, EntityLineWeight::ByBlock);
}
