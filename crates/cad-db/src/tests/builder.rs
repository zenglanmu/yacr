//! Import builder validation, scene identity, and render attributes.

use super::*;

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
            linetype: EntityLineType::default(),
            geometry_source: GeometrySource::Analytic,
            ..Default::default()
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
            linetype: EntityLineType::default(),
            geometry_source: GeometrySource::Analytic,
            ..Default::default()
        },
    )
    .unwrap();
    let db = b.finish().unwrap();
    let attributes = db.entity_render_attributes(EntityId(1));
    assert_eq!(attributes.color, EntityColor::ByBlock);
    assert_eq!(attributes.lineweight, EntityLineWeight::ByBlock);
}
