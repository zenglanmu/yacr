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
    // ByLayer, not a baked `Explicit(1.0)`: a hand-built entity inherits its
    // layer's opacity, and the layer table's `Default` row is opaque.
    assert_eq!(attributes.transparency, EntityTransparency::ByLayer);
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

#[test]
fn code_page_is_absent_by_default_and_survives_the_builder() {
    // A hand-built drawing (or a source with no code page) is `None`: unknown,
    // never a guessed page.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    assert_eq!(drawing.code_page(), None);

    // A declared code page survives verbatim; the db does not interpret it.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.set_code_page(Some("ANSI_936"));
    let drawing = b.finish().unwrap();
    assert_eq!(drawing.code_page(), Some("ANSI_936"));

    // Empty/whitespace means "not declared", not a page named "".
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.set_code_page(Some("   "));
    let drawing = b.finish().unwrap();
    assert_eq!(drawing.code_page(), None);
}

#[test]
fn the_point_display_style_defaults_to_the_dwg_defaults_and_survives_the_builder() {
    // A hand-built database — or a source that declared neither `$PDMODE` nor
    // `$PDSIZE` — gets the DWG defaults: mode 0 (dot), size 0 (viewport-relative).
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    assert_eq!(drawing.point_display_mode(), 0);
    assert_eq!(drawing.point_display_size(), 0.0);

    // Declared values survive verbatim, sign included, so the display layer is
    // the single interpreter of the mode's bits and the size's sign.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.set_point_display_mode(35); // cross + a requested circle (bits 32|3)
    b.set_point_display_size(-20.0); // 20% of the viewport height
    let drawing = b.finish().unwrap();
    assert_eq!(drawing.point_display_mode(), 35);
    assert_eq!(drawing.point_display_size(), -20.0);

    // The db stores the value it was given even when it is not usable as a
    // display size: sanitisation is the **consume point's** rule
    // (`cad_app::render_scene::sanitize_point_display_size`), so a corrupt header
    // stays observable here instead of being silently rewritten at import.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.set_point_display_size(f64::NAN);
    assert!(b.finish().unwrap().point_display_size().is_nan());
}

/// One model-space point entity on layer 0, with `attributes` stored beside it.
fn db_with_attributes(attributes: EntityRenderAttributes) -> DrawingDatabase {
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
    b.set_entity_render_attributes(EntityId(1), attributes)
        .unwrap();
    b.finish().unwrap()
}

#[test]
fn the_entity_linetype_scale_defaults_to_the_identity() {
    // The factor is stored for every linetype provenance, explicit and symbolic,
    // so it lives beside the variant rather than inside it. A database built
    // without an importer must keep the identity, or every hand-built entity
    // would render with invented dashes.
    assert_eq!(EntityRenderAttributes::default().linetype_scale, 1.0);

    let db = db_with_attributes(EntityRenderAttributes::default());
    assert_eq!(db.entity_render_attributes(EntityId(1)).linetype_scale, 1.0);
}

#[test]
fn a_stored_entity_linetype_scale_round_trips_with_either_provenance() {
    // `ByLayer` is the case that used to lose the factor: the variant is
    // symbolic and carries no scale of its own.
    let by_layer = db_with_attributes(EntityRenderAttributes {
        linetype: EntityLineType::ByLayer,
        linetype_scale: 2.5,
        ..Default::default()
    });
    let stored = by_layer.entity_render_attributes(EntityId(1));
    assert_eq!(stored.linetype, EntityLineType::ByLayer);
    assert_eq!(stored.linetype_scale, 2.5);

    // An explicit provenance still keeps its own copy in the variant, and the
    // attribute field carries the same factor (the importer writes both).
    let explicit = EntityLineType::Explicit {
        name: "DASHED".into(),
        pattern: LinetypePattern::from_elements([0.5, -0.25]),
        scale: 2.5,
    };
    let explicit_db = db_with_attributes(EntityRenderAttributes {
        linetype: explicit.clone(),
        linetype_scale: 2.5,
        ..Default::default()
    });
    let stored = explicit_db.entity_render_attributes(EntityId(1));
    assert_eq!(stored.linetype, explicit);
    assert_eq!(stored.linetype_scale, 2.5);
}

#[test]
fn a_non_finite_or_non_positive_linetype_scale_sanitises_to_the_identity() {
    // The rule is the existing one (a corrupt factor must not erase every dash),
    // now shared by the importer, the attribute field and the representation
    // instead of being re-implemented per caller.
    for bad in [
        0.0,
        -0.0,
        -1.0,
        -2.5,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        assert_eq!(
            sanitize_linetype_scale(bad),
            1.0,
            "a {bad} linetype scale must degrade to the identity"
        );
    }
    for good in [1.0, 0.5, 2.5, 1e-6, 1e6] {
        assert_eq!(sanitize_linetype_scale(good), good);
    }

    // Storage is verbatim (the fact is preserved as read); sanitising happens
    // where the factor is consumed.
    let db = db_with_attributes(EntityRenderAttributes {
        linetype_scale: f64::NAN,
        ..Default::default()
    });
    let stored = db.entity_render_attributes(EntityId(1)).linetype_scale;
    assert!(stored.is_nan(), "the stored factor is kept as read");
    assert_eq!(sanitize_linetype_scale(stored), 1.0);
}
