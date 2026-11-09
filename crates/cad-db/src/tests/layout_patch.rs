//! Contract tests for the combined layout-table write core
//! (`drawing::layout_edit` + `drawing::block_edit`'s shared implementation).
//!
//! A layout row is [`Layout`] plus an optional [`PlotSettingsRecord`]; the
//! combined apply path is what lets an undo record restore a layout and the
//! entities that sit in it as one revision, and what lets LAYOUTMERGE delete a
//! layout in the same batch that moves its last entity to model space. Every
//! test pins the single revision bump, the plot-settings cascade and the
//! all-or-nothing delete safety against the **post-batch** entity view.

use super::*;

/// A deterministic plot settings row for one layout.
fn plot_record(layout: u128, paper: &str) -> PlotSettingsRecord {
    PlotSettingsRecord {
        layout: LayoutId(layout),
        paper_size_name: paper.into(),
        paper_width: 210.0,
        paper_height: 297.0,
        margins: PlotMargins::uniform(5.0),
        rotation: PlotRotation::None,
        scale_numerator: 1.0,
        scale_denominator: 1.0,
        plot_type: PlotType::Layout,
        paper_units: PlotPaperUnits::Millimeters,
        provenance: PlotProvenance::Imported,
    }
}

/// Layout 1 (`Sheet`) with plot settings and one paper-space line; layer 0.
fn layout_patch_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.set_plot_settings(plot_record(1, "ISO_A4")).unwrap();
    b.insert_entity(raw_entity(
        1,
        SpaceId::Paper(LayoutId(1)),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(4.0, 0.0),
        },
    ))
    .unwrap();
    b.finish().unwrap()
}

/// The current row of one layout, including its stored plot settings.
fn row_of(db: &DrawingDatabase, id: LayoutId) -> LayoutRow {
    LayoutRow {
        layout: db.layout(id).expect("layout exists").clone(),
        plot: db.plot_settings(id).cloned(),
    }
}

#[test]
fn deleting_a_layout_in_the_same_batch_that_moves_its_entity_is_accepted() {
    let mut db = layout_patch_db();
    let start = db.revision();
    let mut moved = db.entity(EntityId(1)).unwrap().clone();
    moved.space = SpaceId::Model;

    let changes = db
        .apply_drawing_changes_with_blocks_display_layers_and_layouts(
            "delete a layout and move its entity to model",
            TransactionId(20),
            &[],
            vec![(EntityId(1), Some(moved))],
            &[],
            &[],
            &[(LayoutId(1), None)],
        )
        .expect("the post-batch entity view leaves no paper reference");

    assert_eq!(changes.before, start);
    assert_eq!(changes.after, Revision(start.0 + 1));
    assert_eq!(db.revision(), Revision(start.0 + 1), "exactly one revision");
    assert_eq!(changes.changes.len(), 1, "one moved entity, one change");
    assert!(
        matches!(
            &changes.changes[0],
            ObjectChange::Update(object, mask)
                if *object == ObjectId(1) && mask.contains(ChangeMask::STYLE)
        ),
        "the space move is an Update with the style mask, got {:?}",
        changes.changes
    );
    assert_eq!(
        db.entity(EntityId(1)).map(|entity| entity.space.clone()),
        Some(SpaceId::Model)
    );
    assert!(db.layout(LayoutId(1)).is_none(), "the layout is gone");
    assert!(
        db.plot_settings(LayoutId(1)).is_none(),
        "the plot settings row never outlives its layout"
    );
}

#[test]
fn a_layout_and_its_paper_entity_are_restored_in_one_batch() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let mut db = b.finish().unwrap();
    let start = db.revision();

    let restore = LayoutRow {
        layout: Layout {
            id: LayoutId(3),
            name: "Restored".into(),
            viewports: Vec::new(),
        },
        plot: Some(plot_record(3, "ISO_A3")),
    };
    let entity = raw_entity(
        1,
        SpaceId::Paper(LayoutId(3)),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(2.0, 0.0),
        },
    );

    let changes = db
        .apply_drawing_changes_with_blocks_display_layers_and_layouts(
            "restore a layout and its entity",
            TransactionId(21),
            &[],
            vec![(EntityId(1), Some(entity))],
            &[],
            &[],
            &[(LayoutId(3), Some(restore.clone()))],
        )
        .expect("the staged layout is a valid paper-space reference");

    assert_eq!(changes.after, Revision(start.0 + 1));
    assert_eq!(db.revision(), Revision(start.0 + 1), "exactly one revision");
    assert_eq!(
        db.entity(EntityId(1)).map(|entity| entity.space.clone()),
        Some(SpaceId::Paper(LayoutId(3)))
    );
    assert_eq!(
        row_of(&db, LayoutId(3)),
        restore,
        "the layout and its plot settings round-trip verbatim"
    );
    assert_eq!(
        db.next_layout_id(),
        LayoutId(4),
        "the allocator stays ahead of the restored row"
    );
}

#[test]
fn deleting_a_still_referenced_layout_is_refused_without_changing_anything() {
    let mut db = layout_patch_db();
    let start = db.revision();
    let snapshot = format!("{db:?}");

    let error = db
        .apply_drawing_changes_with_blocks_display_layers_and_layouts(
            "delete a layout that still owns an entity",
            TransactionId(22),
            &[],
            Vec::new(),
            &[],
            &[],
            &[(LayoutId(1), None)],
        )
        .expect_err("the surviving paper entity is still in SpaceId::Paper(1)");
    assert!(
        error.to_string().contains("layout.in_use"),
        "expected the stable in-use refusal, got {error}"
    );
    assert_eq!(db.revision(), start, "a refusal bumps no revision");
    assert!(db.layout(LayoutId(1)).is_some(), "the layout still exists");
    assert_eq!(format!("{db:?}"), snapshot, "nothing was mutated");
}
