//! Contract tests for the cad-db layout/paper-viewport write core
//! (`drawing::layout_edit`).
//!
//! A layout row is [`Layout`] plus an optional [`PlotSettingsRecord`]; these
//! tests pin the monotonic id allocation, the one-revision-per-call rule, the
//! name grammar shared with the app, the plot-settings cascade, the in-use
//! delete refusal and the exact `LayoutRow` round-trip through
//! [`DrawingDatabase::apply_layout_changes`].

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

/// Layout 1 (`Layout1`, with plot settings) and layout 2 (`Layout2`, no plot).
fn layout_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for (id, name) in [(LayoutId(1), "Layout1"), (LayoutId(2), "Layout2")] {
        b.insert_layout(Layout {
            id,
            name: name.into(),
            viewports: Vec::new(),
        })
        .unwrap();
    }
    b.set_plot_settings(plot_record(1, "ISO_A4")).unwrap();
    b.finish().unwrap()
}

/// A database whose layout 1 contains one paper-space line.
fn in_use_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "InUse".into(),
        viewports: Vec::new(),
    })
    .unwrap();
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

/// The current row of one layout, including its plot settings.
fn row_of(db: &DrawingDatabase, id: LayoutId) -> LayoutRow {
    LayoutRow {
        layout: db.layout(id).expect("layout exists").clone(),
        plot: db.plot_settings(id).cloned(),
    }
}

fn row(id: u128, name: &str, plot: Option<PlotSettingsRecord>) -> LayoutRow {
    LayoutRow {
        layout: Layout {
            id: LayoutId(id),
            name: name.into(),
            viewports: Vec::new(),
        },
        plot,
    }
}

fn finite_viewport() -> PaperViewport {
    PaperViewport {
        clip: vec![
            point(0.0, 0.0),
            point(1.0, 0.0),
            point(1.0, 1.0),
            point(0.0, 1.0),
        ],
        model_to_paper: Transform3::identity(),
        completeness: Completeness::Complete,
    }
}

#[test]
fn create_layout_appends_a_monotonic_id_and_bumps_once() {
    let mut db = layout_db();
    let start = db.revision();
    assert_eq!(db.next_layout_id(), LayoutId(3));

    let (id, changes) = db
        .create_layout("  Sheet A  ", TransactionId(1), "create layout")
        .expect("a fresh, legal name is accepted");

    assert_eq!(id, LayoutId(3));
    assert!(id.0 >= 1, "LayoutId(0) is never allocated");
    assert_eq!(db.layout(id).unwrap().name, "Sheet A");
    assert!(db.layout(id).unwrap().viewports.is_empty());
    assert_eq!(db.plot_settings(id), None);
    assert_eq!(db.next_layout_id(), LayoutId(4));
    assert_eq!(changes.before, start);
    assert_eq!(changes.after, Revision(start.0 + 1));
    assert_eq!(db.revision(), Revision(start.0 + 1));
    assert!(changes.changes.is_empty());
    assert_eq!(changes.transaction, TransactionId(1));
}

#[test]
fn create_layout_refuses_bad_and_duplicate_names() {
    let mut db = layout_db();
    let start = db.revision();
    let before = db.layouts().count();

    for bad in ["", "   ", "A/B", "Line\nBreak", "pipe|name"] {
        let error = db
            .create_layout(bad, TransactionId(1), "bad name")
            .expect_err("an invalid name is refused");
        assert!(
            error.to_string().contains("layout.invalid_name"),
            "unexpected error for {bad:?}: {error}"
        );
    }
    let long = "a".repeat(256);
    assert!(db
        .create_layout(&long, TransactionId(1), "too long")
        .is_err());

    let error = db
        .create_layout("LAYOUT2", TransactionId(1), "duplicate")
        .expect_err("names are unique case-insensitively");
    assert!(error.to_string().contains("layout.duplicate_name"));

    assert_eq!(db.revision(), start);
    assert_eq!(db.layouts().count(), before);
}

#[test]
fn create_layout_never_reuses_a_deleted_id() {
    let mut db = layout_db();
    let (created, _) = db
        .create_layout("Transient", TransactionId(1), "create")
        .unwrap();
    assert_eq!(created, LayoutId(3));
    db.delete_layout(created, TransactionId(2), "delete")
        .unwrap();

    let (next, _) = db
        .create_layout("AfterDelete", TransactionId(3), "create")
        .unwrap();
    assert_eq!(next, LayoutId(4), "a deleted id is never handed out again");
}

#[test]
fn rename_layout_trims_updates_and_keeps_the_plot_row() {
    let mut db = layout_db();
    let plot_before = db.plot_settings(LayoutId(1)).cloned();
    let start = db.revision();

    let changes = db
        .rename_layout(LayoutId(1), "  Sheet One  ", TransactionId(1), "rename")
        .expect("a legal unique name is accepted");

    assert_eq!(db.layout(LayoutId(1)).unwrap().name, "Sheet One");
    assert_eq!(db.plot_settings(LayoutId(1)).cloned(), plot_before);
    assert_eq!(changes.before, start);
    assert_eq!(changes.after, Revision(start.0 + 1));
    assert!(changes.changes.is_empty());

    // Exactly one bump per accepted call, including a same-spelling write.
    db.rename_layout(LayoutId(1), "Sheet One", TransactionId(2), "repeat")
        .unwrap();
    assert_eq!(db.revision(), Revision(start.0 + 2));
}

#[test]
fn rename_layout_refuses_duplicates_and_unknown_ids() {
    let mut db = layout_db();
    let start = db.revision();

    let error = db
        .rename_layout(LayoutId(1), "LAYOUT2", TransactionId(1), "duplicate")
        .expect_err("another layout owns that name case-insensitively");
    assert!(error.to_string().contains("layout.duplicate_name"));
    assert_eq!(db.layout(LayoutId(1)).unwrap().name, "Layout1");

    let error = db
        .rename_layout(LayoutId(99), "Whatever", TransactionId(1), "unknown")
        .expect_err("an unknown layout is refused");
    assert!(error.to_string().contains("layout.unknown_id"));

    // A pure case change of the row's own name is allowed.
    db.rename_layout(LayoutId(1), "LAYOUT1", TransactionId(1), "case change")
        .unwrap();
    assert_eq!(db.layout(LayoutId(1)).unwrap().name, "LAYOUT1");
    assert_eq!(db.revision(), Revision(start.0 + 1));
}

#[test]
fn delete_layout_cascades_plot_settings_and_bumps_once() {
    let mut db = layout_db();
    let start = db.revision();
    assert!(db.plot_settings(LayoutId(1)).is_some());

    let changes = db
        .delete_layout(LayoutId(1), TransactionId(1), "delete layout")
        .expect("the layout is unused");

    assert!(db.layout(LayoutId(1)).is_none());
    assert_eq!(
        db.plot_settings(LayoutId(1)),
        None,
        "plot settings must never outlive their layout"
    );
    assert_eq!(changes.after, Revision(start.0 + 1));
    assert!(changes.changes.is_empty());

    let error = db
        .delete_layout(LayoutId(99), TransactionId(2), "unknown")
        .expect_err("an unknown layout is refused");
    assert!(error.to_string().contains("layout.unknown_id"));
}

#[test]
fn delete_layout_refuses_in_use_and_the_reserved_model_row() {
    let mut db = in_use_db();
    let start = db.revision();

    let error = db
        .delete_layout(LayoutId(1), TransactionId(1), "in use")
        .expect_err("a layout with an entity in it is refused");
    assert!(error.to_string().contains("layout.in_use"));
    assert!(db.layout(LayoutId(1)).is_some());
    assert_eq!(db.revision(), start);

    let mut with_model = DrawingDatabaseBuilder::new(DatabaseId(2));
    with_model
        .insert_layout(Layout {
            id: LayoutId(0),
            name: "Model Space".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    let mut with_model = with_model.finish().unwrap();
    let model_start = with_model.revision();
    let error = with_model
        .delete_layout(LayoutId(0), TransactionId(1), "model")
        .expect_err("the synthetic model row is not deletable");
    assert!(error.to_string().contains("layout.model_undeletable"));
    assert!(with_model.layout(LayoutId(0)).is_some());
    assert_eq!(with_model.revision(), model_start);
}

#[test]
fn set_layout_viewports_stores_them_and_bumps_once() {
    let mut db = layout_db();
    let start = db.revision();
    let viewport = finite_viewport();

    let changes = db
        .set_layout_viewports(
            LayoutId(1),
            vec![viewport.clone()],
            TransactionId(1),
            "set viewport",
        )
        .expect("a finite viewport is accepted");

    assert_eq!(db.layout(LayoutId(1)).unwrap().viewports, vec![viewport]);
    assert!(
        db.plot_settings(LayoutId(1)).is_some(),
        "page setup untouched"
    );
    assert_eq!(changes.after, Revision(start.0 + 1));

    let error = db
        .set_layout_viewports(LayoutId(99), Vec::new(), TransactionId(2), "unknown layout")
        .expect_err("an unknown layout is refused");
    assert!(error.to_string().contains("layout.unknown_id"));
}

#[test]
fn set_layout_viewports_refuses_non_finite_geometry() {
    let mut db = layout_db();
    let start = db.revision();

    let mut non_finite_clip = finite_viewport();
    non_finite_clip.clip[2].x = f64::NAN;
    let error = db
        .set_layout_viewports(
            LayoutId(1),
            vec![non_finite_clip],
            TransactionId(1),
            "nan clip",
        )
        .expect_err("a non-finite clip point is refused");
    assert!(error.to_string().contains("layout.non_finite_viewport"));

    let mut non_finite_matrix = finite_viewport();
    non_finite_matrix.model_to_paper.matrix[0][3] = f64::INFINITY;
    db.set_layout_viewports(
        LayoutId(1),
        vec![non_finite_matrix],
        TransactionId(1),
        "inf matrix",
    )
    .expect_err("a non-finite transform is refused");

    assert_eq!(db.revision(), start);
    assert!(db.layout(LayoutId(1)).unwrap().viewports.is_empty());
}

#[test]
fn apply_layout_changes_creates_and_deletes_in_one_batch() {
    let mut db = layout_db();
    let start = db.revision();

    let changes = db
        .apply_layout_changes(
            "restore layout batch",
            TransactionId(9),
            &[
                (LayoutId(3), Some(row(3, "New", None))),
                (LayoutId(2), None),
            ],
        )
        .expect("the layout-only batch is valid");

    assert_eq!(changes.before, start);
    assert_eq!(changes.after, Revision(start.0 + 1));
    assert!(changes.changes.is_empty());
    assert!(db.layout(LayoutId(2)).is_none());
    assert_eq!(db.layout(LayoutId(3)).unwrap().name, "New");
    assert_eq!(db.plot_settings(LayoutId(2)), None);
    assert_eq!(
        db.next_layout_id(),
        LayoutId(4),
        "an applied create advances the allocator too"
    );
}

#[test]
fn apply_layout_changes_refuses_duplicate_final_names() {
    let mut db = layout_db();
    let start = db.revision();

    let error = db
        .apply_layout_changes(
            "duplicate",
            TransactionId(1),
            &[(LayoutId(3), Some(row(3, "layout1", None)))],
        )
        .expect_err("the final name set would contain a duplicate");
    assert!(error.to_string().contains("layout.duplicate_name"));
    assert_eq!(db.revision(), start);
    assert!(db.layout(LayoutId(3)).is_none());

    let error = db
        .apply_layout_changes(
            "duplicate inside batch",
            TransactionId(1),
            &[
                (LayoutId(3), Some(row(3, "Shared", None))),
                (LayoutId(4), Some(row(4, "SHARED", None))),
            ],
        )
        .expect_err("two rows staged in one batch cannot share a name");
    assert!(error.to_string().contains("layout.duplicate_name"));
    assert_eq!(db.revision(), start);
}

#[test]
fn apply_layout_changes_refuses_deleting_a_referenced_layout() {
    let mut db = in_use_db();
    let start = db.revision();

    let error = db
        .apply_layout_changes(
            "delete referenced layout",
            TransactionId(1),
            &[(LayoutId(1), None)],
        )
        .expect_err("an entity still lives in the layout");
    assert!(error.to_string().contains("layout.in_use"));
    assert!(db.layout(LayoutId(1)).is_some());
    assert_eq!(db.revision(), start);
}

#[test]
fn apply_layout_changes_round_trips_the_plot_row_exactly() {
    let mut db = layout_db();
    let original = row_of(&db, LayoutId(1));
    assert_eq!(original.plot.as_ref().unwrap().paper_size_name, "ISO_A4");

    // Remove the plot row through a patch that stages no page setup.
    db.apply_layout_changes(
        "drop plot",
        TransactionId(1),
        &[(LayoutId(1), Some(row(1, "Layout1", None)))],
    )
    .unwrap();
    assert_eq!(db.plot_settings(LayoutId(1)), None);

    // Restore the recorded row: the plot settings come back byte-for-byte.
    db.apply_layout_changes(
        "restore plot",
        TransactionId(2),
        &[(LayoutId(1), Some(original.clone()))],
    )
    .unwrap();
    assert_eq!(row_of(&db, LayoutId(1)), original);
    assert_eq!(db.plot_settings(LayoutId(1)).cloned(), original.plot);

    // A mismatched plot key is a caller bug and is refused.
    let mismatch = PlotSettingsRecord {
        layout: LayoutId(2),
        ..plot_record(2, "Other")
    };
    let error = db
        .apply_layout_changes(
            "mismatched plot",
            TransactionId(3),
            &[(LayoutId(1), Some(row(1, "Layout1", Some(mismatch))))],
        )
        .expect_err("plot.layout must equal the change key");
    assert!(error.to_string().contains("layout.plot_mismatch"));
}

#[test]
fn apply_layout_changes_may_update_but_never_creates_layout_zero() {
    let mut db = layout_db();
    let start = db.revision();

    let error = db
        .apply_layout_changes(
            "create model row",
            TransactionId(1),
            &[(LayoutId(0), Some(row(0, "Model Space", None)))],
        )
        .expect_err("a patch cannot create LayoutId(0)");
    assert!(error.to_string().contains("layout.model_undeletable"));
    assert_eq!(db.revision(), start);

    let mut with_model = DrawingDatabaseBuilder::new(DatabaseId(2));
    with_model
        .insert_layout(Layout {
            id: LayoutId(0),
            name: "Model Space".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    let mut with_model = with_model.finish().unwrap();
    let model_start = with_model.revision();

    with_model
        .apply_layout_changes(
            "update model row",
            TransactionId(1),
            &[(LayoutId(0), Some(row(0, "Model Space", None)))],
        )
        .expect("updating the existing synthetic model row is allowed");
    assert_eq!(with_model.revision(), Revision(model_start.0 + 1));
}

#[test]
fn set_plot_settings_upserts_one_revision_and_round_trips_through_layout_changes() {
    let mut db = layout_db();
    let start = db.revision();
    let mut record = plot_record(2, "  ISO_A3  ");
    record.provenance = PlotProvenance::Authored;
    record.rotation = PlotRotation::Degrees90;

    let changes = db
        .set_plot_settings(LayoutId(2), record.clone(), TransactionId(9), "pagesetup")
        .expect("a valid authored page setup is stored");
    assert_eq!(db.revision(), Revision(start.0 + 1), "one revision bump");
    assert_eq!(changes.before, start);
    assert_eq!(changes.after, db.revision());
    assert!(
        changes.changes.is_empty(),
        "a layout edit changes no entities"
    );
    let stored = db
        .plot_settings(LayoutId(2))
        .expect("layout 2 now has a stored page setup")
        .clone();
    assert_eq!(
        stored.paper_size_name, "ISO_A3",
        "the name is stored trimmed"
    );
    assert_eq!(stored.provenance, PlotProvenance::Authored);
    assert_eq!(stored.rotation, PlotRotation::Degrees90);

    // The authored row round-trips through the undo/redo apply entry: removing
    // it restores "no stored page setup", re-adding it restores the same record.
    let with_plot = row_of(&db, LayoutId(2));
    db.apply_layout_changes(
        "undo pagesetup",
        TransactionId(10),
        &[(LayoutId(2), Some(row(2, "Layout2", None)))],
    )
    .expect("removing the plot row is a legal layout batch");
    assert!(db.plot_settings(LayoutId(2)).is_none());
    db.apply_layout_changes(
        "redo pagesetup",
        TransactionId(11),
        &[(LayoutId(2), Some(with_plot))],
    )
    .expect("restoring the authored plot row is a legal layout batch");
    assert_eq!(db.plot_settings(LayoutId(2)), Some(&stored));
}

#[test]
fn set_plot_settings_refuses_unknown_layout_bad_paper_and_bad_scale() {
    let mut db = layout_db();
    let start = db.revision();

    let unknown = db
        .set_plot_settings(
            LayoutId(99),
            plot_record(99, "A4"),
            TransactionId(1),
            "pagesetup",
        )
        .expect_err("an unknown layout is refused");
    assert!(
        unknown.to_string().contains("layout.unknown_id"),
        "{unknown}"
    );

    let mismatched = db
        .set_plot_settings(
            LayoutId(2),
            plot_record(1, "A4"),
            TransactionId(1),
            "pagesetup",
        )
        .expect_err("the record key must match the layout");
    assert!(
        mismatched.to_string().contains("layout.plot_mismatch"),
        "{mismatched}"
    );

    let empty_paper = db
        .set_plot_settings(
            LayoutId(2),
            plot_record(2, "   "),
            TransactionId(1),
            "pagesetup",
        )
        .expect_err("a blank paper name is refused");
    assert!(
        empty_paper.to_string().contains("layout.invalid_paper"),
        "{empty_paper}"
    );

    let mut zero_width = plot_record(2, "A4");
    zero_width.paper_width = 0.0;
    let zero_width = db
        .set_plot_settings(LayoutId(2), zero_width, TransactionId(1), "pagesetup")
        .expect_err("zero paper width is refused");
    assert!(
        zero_width.to_string().contains("layout.invalid_paper"),
        "{zero_width}"
    );

    let mut zero_scale = plot_record(2, "A4");
    zero_scale.scale_denominator = 0.0;
    let zero_scale = db
        .set_plot_settings(LayoutId(2), zero_scale, TransactionId(1), "pagesetup")
        .expect_err("a zero scale denominator is refused");
    assert!(
        zero_scale.to_string().contains("layout.invalid_scale"),
        "{zero_scale}"
    );

    assert_eq!(
        db.revision(),
        start,
        "every refusal leaves the revision alone"
    );
    assert!(db.plot_settings(LayoutId(2)).is_none());
}
