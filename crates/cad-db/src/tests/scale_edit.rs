//! Post-import writes for the annotation-scale table (`SCALELISTEDIT`).
//!
//! Every case drives the database API directly, so the contract is pinned
//! independently of the command layer: one revision per real write, no revision
//! for an authoritative no-op, and a refusal that changes nothing.

use super::*;

fn scale_db() -> DrawingDatabase {
    scale_db_builder().finish().unwrap()
}

/// The same two-row table plus the `CANNOSCALE` header value `1:100`.
fn scale_db_with_active() -> DrawingDatabase {
    let mut b = scale_db_builder();
    b.set_active_annotation_scale("1:100", 0.01).unwrap();
    b.finish().unwrap()
}

fn scale_db_builder() -> DrawingDatabaseBuilder {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_scale(crate::tables::Scale {
        id: ScaleId(0),
        name: "1:1".into(),
        paper_units: 1.0,
        drawing_units: 1.0,
    })
    .unwrap();
    b.insert_scale(crate::tables::Scale {
        id: ScaleId(1),
        name: "1:100".into(),
        paper_units: 1.0,
        drawing_units: 100.0,
    })
    .unwrap();
    b
}

#[test]
fn create_scale_adds_one_row_and_bumps_the_revision_once() {
    let mut db = scale_db();
    let before = db.revision();
    let (changeset, row) = db
        .create_scale("1:200", 1.0, 200.0, TransactionId(7), "scalelist add")
        .expect("a new, finite, unique scale is accepted");
    assert_eq!(row.name, "1:200");
    assert_eq!(db.revision().0, before.0 + 1);
    assert_eq!(changeset.before, before);
    assert_eq!(changeset.after, db.revision());
    assert!(changeset.changes.is_empty(), "a table row is not an entity");
    // The allocator moved past every imported id, so the new row cannot alias.
    assert_eq!(row.id, ScaleId(2));
    assert_eq!(db.scales().count(), 3);
    assert!((db.scale_by_name("1:200").unwrap().factor() - 0.005).abs() < 1e-12);
}

#[test]
fn create_scale_refuses_an_empty_duplicate_or_non_positive_row() {
    let mut db = scale_db();
    let revision = db.revision();

    let empty = db
        .create_scale("   ", 1.0, 1.0, TransactionId(1), "scalelist add")
        .unwrap_err();
    assert!(
        empty.to_string().contains("scalelistedit.name_empty"),
        "{empty}"
    );

    // A newline would be written as two names by a line-based reader.
    let multiline = db
        .create_scale("1:2\n1:3", 1.0, 2.0, TransactionId(1), "scalelist add")
        .unwrap_err();
    assert!(
        multiline
            .to_string()
            .contains("scalelistedit.name_illegal_char"),
        "{multiline}"
    );

    let duplicate = db
        .create_scale("1:100", 1.0, 100.0, TransactionId(1), "scalelist add")
        .unwrap_err();
    assert!(
        duplicate
            .to_string()
            .contains("scalelistedit.duplicate_name"),
        "{duplicate}"
    );
    // Uniqueness is case-insensitive, like the name resolution it feeds.
    let duplicate_case = db
        .create_scale("1:1", 1.0, 1.0, TransactionId(1), "scalelist add")
        .unwrap_err();
    assert!(
        duplicate_case
            .to_string()
            .contains("scalelistedit.duplicate_name"),
        "{duplicate_case}"
    );

    let zero = db
        .create_scale("1:0", 1.0, 0.0, TransactionId(1), "scalelist add")
        .unwrap_err();
    assert!(
        zero.to_string().contains("scalelistedit.invalid_ratio"),
        "{zero}"
    );
    let negative = db
        .create_scale("2:1", -2.0, 1.0, TransactionId(1), "scalelist add")
        .unwrap_err();
    assert!(
        negative.to_string().contains("scalelistedit.invalid_ratio"),
        "{negative}"
    );
    let non_finite = db
        .create_scale("nan:1", f64::NAN, 1.0, TransactionId(1), "scalelist add")
        .unwrap_err();
    assert!(
        non_finite
            .to_string()
            .contains("scalelistedit.invalid_ratio"),
        "{non_finite}"
    );

    assert_eq!(
        db.revision(),
        revision,
        "no refused create bumped the revision"
    );
    assert_eq!(db.scales().count(), 2, "no refused create added a row");
}

#[test]
fn rename_scale_retargets_the_active_annotation_scale_and_entity_overrides() {
    let mut db = scale_db_with_active();
    let before = db.revision();
    let changeset = db
        .rename_scale("1:100", "1:150", TransactionId(2), "scalelist rename")
        .expect("an unreferenced rename is accepted");
    assert!(db.scale_by_name("1:100").is_none());
    let renamed = db.scale_by_name("1:150").expect("the row kept its ratio");
    assert_eq!((renamed.paper_units, renamed.drawing_units), (1.0, 100.0));
    assert_eq!(db.revision().0, before.0 + 1);
    // The header reference followed the row, so the active scale is still named.
    let active = db.active_annotation_scale().expect("active scale survives");
    assert_eq!(active.name, "1:150");
    assert!(db.annotation_scale().2, "1:150 resolves in the table");
    assert!(
        changeset.changes.is_empty(),
        "no entity carries an override"
    );
}

#[test]
fn rename_scale_publishes_the_retargeted_entities_as_style_updates() {
    // An annotative entity with one placement override at 1:100, built the way
    // the importer builds it.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_scale(crate::tables::Scale {
        id: ScaleId(0),
        name: "1:100".into(),
        paper_units: 1.0,
        drawing_units: 100.0,
    })
    .unwrap();
    b.set_active_annotation_scale("1:100", 0.01).unwrap();
    b.insert_entity(raw_entity(
        9,
        SpaceId::Model,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    ))
    .unwrap();
    let mut attributes = crate::EntityRenderAttributes::default();
    attributes.annotative.annotative = true;
    attributes.annotative.overrides = vec![crate::AnnotativeScaleOverride {
        scale: "1:100".into(),
        placement: crate::AnnotativePlacement {
            position: point(0.0, 0.0),
            rotation: 0.0,
            height: None,
        },
    }];
    b.set_entity_render_attributes(EntityId(9), attributes)
        .unwrap();
    let mut db = b.finish().unwrap();

    let changeset = db
        .rename_scale("1:100", "1:200", TransactionId(3), "scalelist rename")
        .expect("the rename is accepted");
    assert_eq!(changeset.changes.len(), 1);
    assert_eq!(
        changeset.changes[0],
        ObjectChange::Update(ObjectId(9), ChangeMask::STYLE),
        "the override rewrite must rebuild that entity's representation"
    );
    let stored = db.entity_render_attributes(EntityId(9));
    assert_eq!(stored.annotative.overrides[0].scale, "1:200");
}

#[test]
fn rename_scale_to_the_stored_spelling_is_an_authoritative_no_op() {
    let mut db = scale_db();
    let before = db.revision();
    let changeset = db
        .rename_scale("1:100", "1:100", TransactionId(4), "scalelist rename")
        .expect("the same name is not an error");
    assert_eq!(changeset.before, changeset.after);
    assert_eq!(db.revision(), before, "nothing changed, so nothing bumped");
}

#[test]
fn rename_scale_refuses_an_unknown_row_or_a_taken_name() {
    let mut db = scale_db();
    let revision = db.revision();
    let unknown = db
        .rename_scale("9:9", "9:8", TransactionId(5), "scalelist rename")
        .unwrap_err();
    assert!(
        unknown.to_string().contains("scalelistedit.unknown_scale"),
        "{unknown}"
    );
    let taken = db
        .rename_scale("1:1", "1:100", TransactionId(5), "scalelist rename")
        .unwrap_err();
    assert!(
        taken.to_string().contains("scalelistedit.duplicate_name"),
        "{taken}"
    );
    assert_eq!(db.revision(), revision);
    assert!(db.scale_by_name("1:1").is_some());
}

#[test]
fn delete_scale_refuses_while_a_name_reference_survives() {
    let mut db = scale_db_with_active();
    let revision = db.revision();
    let refused = db
        .delete_scale("1:100", TransactionId(6), "scalelist delete")
        .unwrap_err();
    let text = refused.to_string();
    assert!(text.contains("scalelistedit.in_use"), "{text}");
    assert!(text.contains("CANNOSCALE"), "{text}");
    assert_eq!(db.revision(), revision, "a refusal writes nothing");
    assert!(db.scale_by_name("1:100").is_some());

    // The reference counts the command layer reports are the same ones the write
    // checks, so the pre-flight can never disagree with the write.
    let counts = db.scale_reference_counts("1:100");
    assert_eq!(counts.annotation_scale, 1);
    assert_eq!(counts.entity_overrides, 0);
    assert_eq!(counts.total(), 1);
}

#[test]
fn delete_scale_removes_an_unreferenced_row_in_one_revision() {
    let mut db = scale_db();
    let before = db.revision();
    let changeset = db
        .delete_scale("1:1", TransactionId(8), "scalelist delete")
        .expect("1:1 is referenced by nothing");
    assert!(db.scale_by_name("1:1").is_none());
    assert_eq!(db.revision().0, before.0 + 1);
    assert!(changeset.changes.is_empty());
    // The deleted id is never handed out again.
    let (_, row) = db
        .create_scale("2:1", 2.0, 1.0, TransactionId(9), "scalelist add")
        .unwrap();
    assert_eq!(row.id, ScaleId(2));
}
