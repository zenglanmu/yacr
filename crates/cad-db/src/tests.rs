//! Unit tests.

use super::*;
use cad_domain::*;

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
        dynamic_visibility: None,
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
        dynamic_visibility: None,
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

// ---- Dynamic-block visibility (spec §3.2) ----

/// A block definition + members for the dynamic-visibility tests.
///
/// Block 0 owns three lines (entities 10, 11, 12) governed by a two-state
/// visibility parameter: "A" shows 10 and 11, "B" shows 11 and 12.
fn dynamic_block_db(active: Option<&str>) -> DrawingDatabase {
    use crate::tables::{DynamicBlockState, DynamicBlockVisibility};
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(10), EntityId(11), EntityId(12)],
        dynamic_visibility: None,
    })
    .unwrap();
    for id in [10u128, 11, 12] {
        b.insert_entity(raw_entity(
            id,
            SpaceId::Block(BlockId(0)),
            SemanticGeometry::Line {
                start: point(id as f64, 0.0),
                end: point(id as f64 + 1.0, 0.0),
            },
        ))
        .unwrap();
    }
    b.set_block_dynamic_visibility(
        BlockId(0),
        DynamicBlockVisibility {
            member_entities: vec![EntityId(10), EntityId(11), EntityId(12)],
            states: vec![
                DynamicBlockState {
                    name: "A".into(),
                    entities: vec![EntityId(10), EntityId(11)],
                },
                DynamicBlockState {
                    name: "B".into(),
                    entities: vec![EntityId(11), EntityId(12)],
                },
            ],
            active_state: active.map(str::to_string),
        },
    )
    .unwrap();
    b.finish().unwrap()
}

#[test]
fn dynamic_block_emits_only_the_active_state_entities() {
    let db = dynamic_block_db(Some("A"));
    let visible: Vec<EntityId> = db.block_entities(BlockId(0)).iter().map(|e| e.id).collect();
    assert_eq!(visible, vec![EntityId(10), EntityId(11)]);
    assert_eq!(
        db.block_visible_entities(BlockId(0)),
        Some(vec![EntityId(10), EntityId(11)])
    );

    let db = dynamic_block_db(Some("B"));
    let visible: Vec<EntityId> = db.block_entities(BlockId(0)).iter().map(|e| e.id).collect();
    assert_eq!(visible, vec![EntityId(11), EntityId(12)]);
}

#[test]
fn unknown_active_state_draws_every_member_never_a_guess() {
    let db = dynamic_block_db(None);
    assert_eq!(db.block_entities(BlockId(0)).len(), 3);
    // Entities outside the governed set are always visible.
    assert!(db
        .block_dynamic_visibility(BlockId(0))
        .unwrap()
        .is_visible(EntityId(999)));
}

#[test]
fn visibility_switch_emits_the_expected_delta_and_raises_revision() {
    let mut db = dynamic_block_db(Some("A"));
    let before_revision = db.revision();
    let changeset = db
        .set_block_visibility_state(BlockId(0), "B", TransactionId(7), "switch to B")
        .unwrap();
    assert_eq!(changeset.before, before_revision);
    assert_eq!(changeset.after, Revision(before_revision.0 + 1));
    assert_eq!(db.revision(), changeset.after);
    // Only the entities that entered/left the visible set are reported:
    // 10 leaves, 12 enters, 11 is common to both states.
    let mut reported: Vec<u128> = changeset
        .changes
        .iter()
        .map(|c| match c {
            ObjectChange::Update(id, mask) => {
                assert!(mask.contains(ChangeMask::GEOMETRY));
                id.0
            }
            other => panic!("unexpected change {other:?}"),
        })
        .collect();
    reported.sort();
    assert_eq!(reported, vec![10, 12]);
    assert_eq!(
        db.block_visible_entities(BlockId(0)),
        Some(vec![EntityId(11), EntityId(12)])
    );
}

#[test]
fn switching_to_the_already_active_state_is_a_no_op() {
    let mut db = dynamic_block_db(Some("A"));
    let before = db.revision();
    let changeset = db
        .set_block_visibility_state(BlockId(0), "A", TransactionId(1), "same state")
        .unwrap();
    assert!(changeset.is_empty());
    assert_eq!(changeset.after, before);
    assert_eq!(db.revision(), before);
}

#[test]
fn unknown_visibility_state_is_rejected() {
    let mut db = dynamic_block_db(Some("A"));
    let before = db.revision();
    let result = db.set_block_visibility_state(BlockId(0), "NOPE", TransactionId(1), "bad state");
    assert!(matches!(result, Err(CadError::InvalidInput(_))));
    assert_eq!(db.revision(), before, "a rejected switch changes nothing");
    assert_eq!(
        db.block_visible_entities(BlockId(0)),
        Some(vec![EntityId(10), EntityId(11)])
    );
}

#[test]
fn visibility_for_a_block_without_a_parameter_is_rejected() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    let mut db = b.finish().unwrap();
    assert!(db.block_visible_entities(BlockId(0)).is_none());
    assert!(matches!(
        db.set_block_visibility_state(BlockId(0), "A", TransactionId(1), "no parameter"),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn builder_rejects_a_visibility_state_for_a_non_member_entity() {
    use crate::tables::{DynamicBlockState, DynamicBlockVisibility};
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(1)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(raw_entity(
        1,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Point(point(0.0, 0.0)),
    ))
    .unwrap();
    // Entity 2 is not a member of the block.
    b.set_block_dynamic_visibility(
        BlockId(0),
        DynamicBlockVisibility {
            member_entities: vec![EntityId(1)],
            states: vec![DynamicBlockState {
                name: "A".into(),
                entities: vec![EntityId(2)],
            }],
            active_state: Some("A".into()),
        },
    )
    .unwrap();
    assert!(b.finish().is_err(), "a non-member entity must be rejected");
}

#[test]
fn builder_rejects_duplicate_state_names_and_an_undefined_active_state() {
    use crate::tables::{DynamicBlockState, DynamicBlockVisibility};
    let make = |states: Vec<DynamicBlockState>, active: Option<&str>| {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(1)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_entity(raw_entity(
            1,
            SpaceId::Block(BlockId(0)),
            SemanticGeometry::Point(point(0.0, 0.0)),
        ))
        .unwrap();
        b.set_block_dynamic_visibility(
            BlockId(0),
            DynamicBlockVisibility {
                member_entities: vec![EntityId(1)],
                states,
                active_state: active.map(str::to_string),
            },
        )
        .unwrap();
        b.finish()
    };
    let state = |name: &str| DynamicBlockState {
        name: name.into(),
        entities: vec![EntityId(1)],
    };
    assert!(make(vec![state("A"), state("A")], Some("A")).is_err());
    assert!(make(vec![state("A")], Some("B")).is_err());
    assert!(make(vec![state("A")], Some("A")).is_ok());
}

// ---------------------------------------------------------------------------
// Annotation scales
// ---------------------------------------------------------------------------

fn scale(id: u128, name: &str, paper: f64, drawing: f64) -> crate::tables::Scale {
    crate::tables::Scale {
        id: ScaleId(id),
        name: name.to_string(),
        paper_units: paper,
        drawing_units: drawing,
    }
}

#[test]
fn scale_factor_matches_the_paper_drawing_ratio() {
    let unit = scale(1, "1:1", 1.0, 1.0);
    assert_eq!(unit.factor(), 1.0);
    assert!(unit.is_unit_scale());
    assert!(!unit.is_reduction());
    assert!(!unit.is_enlargement());

    let reduction = scale(2, "1:100", 1.0, 100.0);
    assert!((reduction.factor() - 0.01).abs() < 1e-12);
    assert!((reduction.inverse_factor() - 100.0).abs() < 1e-9);
    assert!(reduction.is_reduction());

    let enlargement = scale(3, "2:1", 2.0, 1.0);
    assert_eq!(enlargement.factor(), 2.0);
    assert!(enlargement.is_enlargement());

    // A degenerate drawing-units value falls back to 1.0 rather than infinity.
    let degenerate = scale(4, "bad", 1.0, 0.0);
    assert_eq!(degenerate.factor(), 1.0);
    assert!(degenerate.is_well_formed());

    let non_finite = scale(5, "nan", f64::NAN, 1.0);
    assert!(!non_finite.is_well_formed());
}

#[test]
fn scales_and_active_scale_are_resolvable_by_name() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_scale(scale(1, "1:1", 1.0, 1.0)).unwrap();
    b.insert_scale(scale(2, "1:100", 1.0, 100.0)).unwrap();
    b.set_active_annotation_scale("1:100", 0.01).unwrap();
    let db = b.finish().unwrap();

    assert_eq!(db.scales().count(), 2);
    assert_eq!(db.scale_by_name("1:100").unwrap().factor(), 0.01);
    assert!(db.scale_by_name("1:999").is_none());

    let active = db.active_annotation_scale().expect("active scale set");
    assert_eq!(active.name, "1:100");
    assert!((active.value - 0.01).abs() < 1e-12);
    assert!(active.named, "1:100 is present in the table");
    assert_eq!(db.annotation_scale_name(), Some("1:100"));
    assert_eq!(db.annotation_scale(), (Some("1:100".into()), 0.01, true));
}

#[test]
fn active_scale_not_in_the_table_is_reported_named_false() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.set_active_annotation_scale("1:50", 0.02).unwrap();
    let db = b.finish().unwrap();
    let active = db.active_annotation_scale().expect("active scale set");
    assert!(!active.named, "1:50 was never inserted");
    assert!(!db.annotation_scale().2);
}

#[test]
fn builder_rejects_non_finite_scale_and_non_finite_active_value() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    let err = b
        .insert_scale(scale(1, "bad", f64::INFINITY, 1.0))
        .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    let err = b.set_active_annotation_scale("1:1", f64::NAN).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
}

// ---------------------------------------------------------------------------
// Drawing write transaction (docs/drawing-edit.md §1)
// ---------------------------------------------------------------------------

fn line_entity(id: u128, start: Point3, end: Point3) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line { start, end },
        draw_order: id as i64,
    }
}

/// A database with layer 0, layout 1 and block 0 (no members yet).
fn writable_db() -> DrawingDatabase {
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
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    b.finish().unwrap()
}

#[test]
fn drawing_insert_commits_in_order_and_bumps_revision() {
    let mut db = writable_db();
    let mut tx = db
        .begin_drawing_transaction("create line", TransactionId(7))
        .unwrap();
    tx.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    tx.insert_entity(line_entity(2, point(0.0, 0.0), point(0.0, 1.0)))
        .unwrap();
    assert_eq!(tx.staged_len(), 2);
    let changes = tx.commit().unwrap();
    assert_eq!(changes.before, Revision(0));
    assert_eq!(changes.after, Revision(1));
    assert_eq!(db.revision(), Revision(1));
    // Caller staging order is preserved, even though ids are not sorted.
    assert_eq!(
        changes.changes,
        vec![
            ObjectChange::Insert(ObjectId(1)),
            ObjectChange::Insert(ObjectId(2)),
        ]
    );
    assert!(changes.follows(DatabaseId(1), Revision(0)));
}

#[test]
fn drawing_empty_transaction_is_a_noop() {
    let mut db = writable_db();
    let before = db.revision();
    let tx = db
        .begin_drawing_transaction("nothing", TransactionId(1))
        .unwrap();
    let changes = tx.commit().unwrap();
    assert!(changes.is_empty());
    assert_eq!(changes.before, changes.after);
    assert_eq!(changes.before, before);
    assert_eq!(db.revision(), before);
    assert_eq!(db.entity_count(), 0);
}

#[test]
fn drawing_rollback_leaves_no_state() {
    let mut db = writable_db();
    {
        let mut tx = db
            .begin_drawing_transaction("edit", TransactionId(1))
            .unwrap();
        tx.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
            .unwrap();
        assert_eq!(tx.staged_len(), 1);
        tx.rollback();
    }
    assert_eq!(db.entity_count(), 0);
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn drawing_invalid_change_rejects_the_whole_transaction() {
    let mut db = writable_db();
    let before_identity = db.scene_identity();
    // Entity 99 has a zero-length line, which validation rejects.
    let mut tx = db
        .begin_drawing_transaction("mixed", TransactionId(1))
        .unwrap();
    tx.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    tx.insert_entity(line_entity(99, point(2.0, 2.0), point(2.0, 2.0)))
        .unwrap();
    let err = tx.commit().unwrap_err();
    assert!(matches!(err, CadError::Invariant(_)), "got {err:?}");
    // Nothing was written: no entity 1, no revision bump, same scene identity.
    assert_eq!(db.entity_count(), 0);
    assert_eq!(db.revision(), Revision(0));
    assert_eq!(db.scene_identity(), before_identity);
}

#[test]
fn drawing_key_mismatch_and_object_mismatch_are_rejected() {
    let mut db = writable_db();
    // Change key 5 but the entity carries id 6.
    let mut wrong = line_entity(6, point(0.0, 0.0), point(1.0, 0.0));
    wrong.object.id = ObjectId(6);
    let err = db
        .apply_drawing_changes("bad", TransactionId(1), vec![(EntityId(5), Some(wrong))])
        .unwrap_err();
    assert!(matches!(err, CadError::Invariant(_)));

    // Key/id agree but the object id does not share the value.
    let mut wrong = line_entity(5, point(0.0, 0.0), point(1.0, 0.0));
    wrong.object.id = ObjectId(1234);
    let err = db
        .apply_drawing_changes("bad", TransactionId(1), vec![(EntityId(5), Some(wrong))])
        .unwrap_err();
    assert!(matches!(err, CadError::Invariant(_)));
    assert_eq!(db.entity_count(), 0);
}

#[test]
fn drawing_missing_layer_and_missing_space_are_rejected() {
    let mut db = writable_db();
    let mut bad_layer = line_entity(1, point(0.0, 0.0), point(1.0, 0.0));
    bad_layer.layer = LayerId(42);
    assert!(db
        .apply_drawing_changes("x", TransactionId(1), vec![(EntityId(1), Some(bad_layer))])
        .is_err());

    let mut bad_layout = line_entity(2, point(0.0, 0.0), point(1.0, 0.0));
    bad_layout.space = SpaceId::Paper(LayoutId(9));
    assert!(db
        .apply_drawing_changes("x", TransactionId(1), vec![(EntityId(2), Some(bad_layout))])
        .is_err());

    let mut bad_block = line_entity(3, point(0.0, 0.0), point(1.0, 0.0));
    bad_block.space = SpaceId::Block(BlockId(9));
    assert!(db
        .apply_drawing_changes("x", TransactionId(1), vec![(EntityId(3), Some(bad_block))])
        .is_err());
    assert_eq!(db.entity_count(), 0);
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn drawing_double_modification_is_rejected() {
    let mut db = writable_db();
    let mut tx = db.begin_drawing_transaction("x", TransactionId(1)).unwrap();
    tx.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    // Staging the same id again is rejected immediately, before commit.
    assert!(tx
        .insert_entity(line_entity(1, point(0.0, 0.0), point(2.0, 0.0)))
        .is_err());

    // The shared apply path also rejects a duplicated id in one batch.
    let err = db
        .apply_drawing_changes(
            "dup",
            TransactionId(1),
            vec![
                (
                    EntityId(1),
                    Some(line_entity(1, point(0.0, 0.0), point(1.0, 0.0))),
                ),
                (EntityId(1), None),
            ],
        )
        .unwrap_err();
    assert!(matches!(err, CadError::Invariant(_)));
    assert_eq!(db.entity_count(), 0);
}

#[test]
fn drawing_delete_missing_entity_is_rejected() {
    let mut db = writable_db();
    let mut tx = db.begin_drawing_transaction("x", TransactionId(1)).unwrap();
    assert!(tx.delete_entity(EntityId(99)).is_err());
    // Nothing valid was staged; commit is a no-op rather than a bump.
    let changes = tx.commit().unwrap();
    assert!(changes.is_empty());
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn drawing_update_masks_distinguish_geometry_and_style() {
    let mut db = writable_db();
    db.apply_drawing_changes(
        "seed",
        TransactionId(1),
        vec![(
            EntityId(1),
            Some(line_entity(1, point(0.0, 0.0), point(1.0, 0.0))),
        )],
    )
    .unwrap();

    // Geometry-only update.
    let changeset = db
        .apply_drawing_changes(
            "geometry",
            TransactionId(2),
            vec![(
                EntityId(1),
                Some(line_entity(1, point(0.0, 0.0), point(5.0, 0.0))),
            )],
        )
        .unwrap();
    assert_eq!(
        changeset.changes,
        vec![ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY)]
    );

    // Style-only update (draw order changes, geometry identical).
    let mut styled = line_entity(1, point(0.0, 0.0), point(5.0, 0.0));
    styled.draw_order = 99;
    let changeset = db
        .apply_drawing_changes("style", TransactionId(3), vec![(EntityId(1), Some(styled))])
        .unwrap();
    assert_eq!(
        changeset.changes,
        vec![ObjectChange::Update(ObjectId(1), ChangeMask::STYLE)]
    );

    // Geometry and style together report both bits.
    let mut both = line_entity(1, point(0.0, 0.0), point(8.0, 0.0));
    both.draw_order = 100;
    let changeset = db
        .apply_drawing_changes("both", TransactionId(4), vec![(EntityId(1), Some(both))])
        .unwrap();
    assert_eq!(
        changeset.changes,
        vec![ObjectChange::Update(
            ObjectId(1),
            ChangeMask::GEOMETRY.union(ChangeMask::STYLE)
        )]
    );

    // An identical replacement is still an Update, never an empty mask.
    let same = line_entity(1, point(0.0, 0.0), point(8.0, 0.0));
    let mut same = same;
    same.draw_order = 100;
    let changeset = db
        .apply_drawing_changes("same", TransactionId(5), vec![(EntityId(1), Some(same))])
        .unwrap();
    match &changeset.changes[0] {
        ObjectChange::Update(_, mask) => assert_ne!(mask.0, 0),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn drawing_transform_emits_transform_and_geometry_and_bakes_the_move() {
    let mut db = writable_db();
    db.apply_drawing_changes(
        "seed",
        TransactionId(1),
        vec![(
            EntityId(1),
            Some(line_entity(1, point(0.0, 0.0), point(1.0, 0.0))),
        )],
    )
    .unwrap();

    let mut tx = db
        .begin_drawing_transaction("move", TransactionId(2))
        .unwrap();
    tx.transform_entity(EntityId(1), &Transform3::translation(point(10.0, 0.0)))
        .unwrap();
    let changeset = tx.commit().unwrap();
    assert_eq!(
        changeset.changes,
        vec![ObjectChange::Update(
            ObjectId(1),
            ChangeMask::TRANSFORM.union(ChangeMask::GEOMETRY)
        )]
    );
    match &db.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, point(10.0, 0.0));
            assert_eq!(*end, point(11.0, 0.0));
        }
        other => panic!("unexpected geometry {other:?}"),
    }
}

#[test]
fn drawing_transform_unsupported_kind_is_rejected_and_stops_nothing() {
    let mut db = writable_db();
    // Text is not baked by this build.
    let text = DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbText".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: "hi".into(),
            position: point(0.0, 0.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
        },
        draw_order: 1,
    };
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(1), Some(text))])
        .unwrap();
    let mut tx = db
        .begin_drawing_transaction("move", TransactionId(2))
        .unwrap();
    let err = tx
        .transform_entity(EntityId(1), &Transform3::translation(point(1.0, 0.0)))
        .unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "got {err:?}");
    // The failed stage left nothing; commit is a no-op.
    assert_eq!(tx.staged_len(), 0);
    let changes = tx.commit().unwrap();
    assert!(changes.is_empty());
    assert_eq!(db.revision(), Revision(1), "seed revision unchanged");
}

#[test]
fn drawing_transform_circle_requires_similarity() {
    let mut db = writable_db();
    let circle = DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbCircle".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Circle {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 2.0,
        },
        draw_order: 1,
    };
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(1), Some(circle))])
        .unwrap();

    // A rotation + uniform scale keeps it a circle.
    let mut tx = db
        .begin_drawing_transaction("scale", TransactionId(2))
        .unwrap();
    tx.transform_entity(EntityId(1), &Transform3::scale(3.0))
        .unwrap();
    tx.commit().unwrap();
    match &db.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Circle { radius, center, .. } => {
            assert_eq!(*center, point(0.0, 0.0));
            assert!((radius - 6.0).abs() < 1e-9);
        }
        other => panic!("unexpected {other:?}"),
    }

    // A non-uniform scale would make it an ellipse: refused, not mis-stored.
    let mut shear = Transform3::identity();
    shear.matrix[0][0] = 2.0;
    shear.matrix[1][1] = 1.0;
    let mut tx = db
        .begin_drawing_transaction("squash", TransactionId(3))
        .unwrap();
    let err = tx.transform_entity(EntityId(1), &shear).unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "got {err:?}");
    assert_eq!(tx.staged_len(), 0);
    let _ = tx.commit().unwrap();
    // Circle radius unchanged by the rejected transform.
    match &db.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Circle { radius, .. } => assert!((radius - 6.0).abs() < 1e-9),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn drawing_transform_mirror_is_rejected_for_arcs_not_dropped() {
    let mut db = writable_db();
    let arc = DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbArc".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 2.0,
            start: 0.0,
            sweep: std::f64::consts::FRAC_PI_2,
        },
        draw_order: 1,
    };
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(1), Some(arc))])
        .unwrap();
    // Mirror about the Y axis (negative determinant).
    let mut mirror = Transform3::identity();
    mirror.matrix[0][0] = -1.0;
    let mut tx = db
        .begin_drawing_transaction("mirror", TransactionId(2))
        .unwrap();
    let err = tx.transform_entity(EntityId(1), &mirror).unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "got {err:?}");
    assert_eq!(tx.staged_len(), 0);
}

#[test]
fn drawing_transform_insert_composes_onto_the_instance() {
    let mut db = writable_db();
    let insert = DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbBlockReference".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(point(1.0, 0.0)),
        },
        draw_order: 1,
    };
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(1), Some(insert))])
        .unwrap();
    let mut tx = db
        .begin_drawing_transaction("move", TransactionId(2))
        .unwrap();
    tx.transform_entity(EntityId(1), &Transform3::translation(point(5.0, 0.0)))
        .unwrap();
    tx.commit().unwrap();
    match &db.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Insert { transform, .. } => {
            let p = transform.apply_point(point(0.0, 0.0));
            assert_eq!(p, point(6.0, 0.0));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn drawing_delete_cleans_block_membership_and_render_attributes() {
    let mut db = writable_db();
    // A block-member entity plus a model insert referencing the block.
    let mut member = line_entity(10, point(0.0, 0.0), point(1.0, 0.0));
    member.space = SpaceId::Block(BlockId(0));
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(10), Some(member))])
        .unwrap();
    // Insert path auto-registered membership.
    assert!(db
        .block(BlockId(0))
        .unwrap()
        .entities
        .contains(&EntityId(10)));

    db.apply_drawing_changes(
        "attrs",
        TransactionId(2),
        vec![(
            EntityId(10),
            Some({
                let mut m = db.entity(EntityId(10)).unwrap().clone();
                m.draw_order = 5;
                m
            }),
        )],
    )
    .unwrap();
    assert!(db.entity_render_attributes(EntityId(10)).color == EntityColor::ByLayer);

    let mut tx = db
        .begin_drawing_transaction("delete", TransactionId(3))
        .unwrap();
    tx.delete_entity(EntityId(10)).unwrap();
    let changes = tx.commit().unwrap();
    assert_eq!(changes.changes, vec![ObjectChange::Delete(ObjectId(10))]);
    assert!(db.entity(EntityId(10)).is_none());
    assert!(!db
        .block(BlockId(0))
        .unwrap()
        .entities
        .contains(&EntityId(10)));
}

#[test]
fn drawing_delete_prunes_dynamic_visibility_lists() {
    use crate::tables::{DynamicBlockState, DynamicBlockVisibility};
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(10), EntityId(11)],
        dynamic_visibility: None,
    })
    .unwrap();
    for id in [10u128, 11] {
        let mut e = raw_entity(
            id,
            SpaceId::Block(BlockId(0)),
            SemanticGeometry::Point(point(id as f64, 0.0)),
        );
        e.layer = LayerId(0);
        b.insert_entity(e).unwrap();
    }
    b.set_block_dynamic_visibility(
        BlockId(0),
        DynamicBlockVisibility {
            member_entities: vec![EntityId(10), EntityId(11)],
            states: vec![DynamicBlockState {
                name: "A".into(),
                entities: vec![EntityId(10)],
            }],
            active_state: None,
        },
    )
    .unwrap();
    let mut db = b.finish().unwrap();

    let mut tx = db
        .begin_drawing_transaction("delete", TransactionId(1))
        .unwrap();
    tx.delete_entity(EntityId(10)).unwrap();
    tx.commit().unwrap();
    let vis = db.block_dynamic_visibility(BlockId(0)).unwrap();
    assert!(!vis.member_entities.contains(&EntityId(10)));
    assert!(!vis.states[0].entities.contains(&EntityId(10)));
    assert!(vis.member_entities.contains(&EntityId(11)));
}

#[test]
fn drawing_id_allocation_is_monotonic_and_never_reuses_a_deleted_id() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(line_entity(7, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    let mut db = b.finish().unwrap();
    // Initialised from the largest existing id.
    assert_eq!(db.next_entity_id(), EntityId(8));

    let first = db.allocate_entity_id();
    let second = db.allocate_entity_id();
    assert_eq!(first, EntityId(8));
    assert_eq!(second, EntityId(9));
    assert_eq!(db.next_entity_id(), EntityId(10));

    // Insert id 8 then delete it; the allocator must not hand out 8 again.
    let mut e = line_entity(8, point(0.0, 0.0), point(1.0, 0.0));
    e.layer = LayerId(0);
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(8), Some(e))])
        .unwrap();
    db.apply_drawing_changes("del", TransactionId(2), vec![(EntityId(8), None)])
        .unwrap();
    assert!(db.entity(EntityId(8)).is_none());
    assert_eq!(db.allocate_entity_id(), EntityId(10));
}

#[test]
fn drawing_builder_initialises_allocator_from_block_members_too() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let mut member = line_entity(50, point(0.0, 0.0), point(1.0, 0.0));
    member.space = SpaceId::Block(BlockId(0));
    member.layer = LayerId(0);
    b.insert_entity(member).unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(50)],
        dynamic_visibility: None,
    })
    .unwrap();
    let db = b.finish().unwrap();
    assert_eq!(db.next_entity_id(), EntityId(51));
}

#[test]
fn drawing_scene_identity_changes_on_every_write() {
    let mut db = writable_db();
    let identity0 = db.scene_identity();
    db.apply_drawing_changes(
        "insert",
        TransactionId(1),
        vec![(
            EntityId(1),
            Some(line_entity(1, point(0.0, 0.0), point(1.0, 0.0))),
        )],
    )
    .unwrap();
    let identity1 = db.scene_identity();
    assert_ne!(identity0, identity1);

    let changeset = db
        .apply_drawing_changes(
            "update",
            TransactionId(2),
            vec![(
                EntityId(1),
                Some(line_entity(1, point(0.0, 0.0), point(9.0, 0.0))),
            )],
        )
        .unwrap();
    assert_eq!(changeset.after, Revision(2));
    let identity2 = db.scene_identity();
    assert_ne!(identity1, identity2);

    db.apply_drawing_changes("delete", TransactionId(3), vec![(EntityId(1), None)])
        .unwrap();
    assert_ne!(db.scene_identity(), identity2);
}

#[test]
fn drawing_insert_into_block_auto_registers_membership() {
    let mut db = writable_db();
    let mut member = line_entity(20, point(0.0, 0.0), point(1.0, 0.0));
    member.space = SpaceId::Block(BlockId(0));
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(20), Some(member))])
        .unwrap();
    assert!(db
        .block(BlockId(0))
        .unwrap()
        .entities
        .contains(&EntityId(20)));
}

#[test]
fn drawing_reason_is_required() {
    let mut db = writable_db();
    match db.begin_drawing_transaction("   ", TransactionId(1)) {
        Err(CadError::InvalidInput(_)) => {}
        Err(other) => panic!("unexpected error {other:?}"),
        Ok(_) => panic!("an empty reason must be rejected"),
    }
}

#[test]
fn drawing_validate_geometry_covers_supported_kinds() {
    // A representative valid geometry of each supported kind passes.
    let valid = vec![
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        SemanticGeometry::Polyline {
            points: vec![point(0.0, 0.0), point(1.0, 0.0)],
            bulges: vec![0.0, 0.0],
            closed: false,
        },
        SemanticGeometry::Circle {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 1.0,
        },
        SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 1.0,
            start: 0.0,
            sweep: 1.0,
        },
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            major_axis: point(2.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
        SemanticGeometry::Spline {
            degree: 2,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control_points: vec![point(0.0, 0.0), point(1.0, 1.0), point(2.0, 0.0)],
            weights: vec![1.0, 1.0, 1.0],
        },
        SemanticGeometry::Point(point(3.0, 4.0)),
        SemanticGeometry::Text {
            text: "x".into(),
            position: point(0.0, 0.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
        },
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::identity(),
        },
        SemanticGeometry::Mesh(Mesh {
            vertices: vec![point(0.0, 0.0), point(1.0, 0.0), point(0.0, 1.0)],
            triangles: vec![[0, 1, 2]],
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
        }),
        SemanticGeometry::Image {
            origin: point(0.0, 0.0),
            u: point(0.5, 0.0),
            v: point(0.0, 0.5),
            pixels: [640.0, 480.0],
            file: Some("logo.png".into()),
            clip: None,
            visible: true,
        },
        SemanticGeometry::Mask {
            boundary: vec![point(0.0, 0.0), point(2.0, 0.0), point(2.0, 2.0)],
            inverted: false,
        },
    ];
    for geometry in valid {
        crate::validate_geometry(&geometry)
            .unwrap_or_else(|e| panic!("expected valid, got {e:?} for {geometry:?}"));
    }

    // The same kinds with degenerate values are rejected.
    assert!(crate::validate_geometry(&SemanticGeometry::Mesh(Mesh {
        vertices: vec![point(0.0, 0.0), point(1.0, 0.0), point(0.0, 1.0)],
        triangles: vec![[0, 1, 99]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    }))
    .is_err());
    assert!(crate::validate_geometry(&SemanticGeometry::Spline {
        degree: 2,
        knots: vec![0.0],
        control_points: vec![point(0.0, 0.0)],
        weights: Vec::new(),
    })
    .is_err());
    assert!(crate::validate_geometry(&SemanticGeometry::Ellipse {
        center: point(0.0, 0.0),
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0
        },
        major_axis: point(0.0, 0.0),
        ratio: 0.5,
        start: 0.0,
        sweep: 1.0,
    })
    .is_err());
}

#[test]
fn drawing_transform_bakes_image_placement_and_mask_boundary() {
    let image = SemanticGeometry::Image {
        origin: point(1.0, 2.0),
        u: point(0.5, 0.0),
        v: point(0.0, 0.25),
        pixels: [640.0, 480.0],
        file: Some("logo.png".into()),
        clip: Some(ImageClip {
            vertices: vec![[0.0, 0.0], [1.0, 1.0]],
            inside: true,
        }),
        visible: true,
    };
    // Translate + uniform scale: `origin` moves as a point while `u`/`v` scale
    // as direction vectors (no translation applied to them).
    let mut t = Transform3::scale(2.0);
    t.matrix[0][3] = 10.0;
    t.matrix[1][3] = 20.0;
    match crate::transform_geometry(&image, &t).unwrap() {
        SemanticGeometry::Image {
            origin,
            u,
            v,
            pixels,
            file,
            clip,
            visible,
        } => {
            assert_eq!(origin, point(12.0, 24.0));
            assert_eq!(u, point(1.0, 0.0));
            assert_eq!(v, point(0.0, 0.5));
            assert_eq!(pixels, [640.0, 480.0]);
            assert_eq!(file.as_deref(), Some("logo.png"));
            assert!(clip.expect("clip").inside);
            assert!(visible);
        }
        other => panic!("expected Image, got {other:?}"),
    }

    let mask = SemanticGeometry::Mask {
        boundary: vec![point(0.0, 0.0), point(1.0, 0.0), point(1.0, 1.0)],
        inverted: true,
    };
    match crate::transform_geometry(&mask, &t).unwrap() {
        SemanticGeometry::Mask { boundary, inverted } => {
            assert_eq!(
                boundary,
                vec![point(10.0, 20.0), point(12.0, 20.0), point(12.0, 22.0)]
            );
            assert!(inverted);
        }
        other => panic!("expected Mask, got {other:?}"),
    }
}
