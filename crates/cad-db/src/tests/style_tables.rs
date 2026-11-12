//! Contract tests for the post-import style-table writes (`drawing::style_edit`).
//!
//! The three tables share one batch: a name is trimmed and validated, names are
//! unique case-insensitively **inside** a table, a delete is refused while a row
//! (entity or another style) references it, and the whole batch is one revision.
//! The cascade case — renaming a text style and retargeting `DIMTXSTY` in the
//! same call — is the reason the batch exists.

use super::*;
use crate::dim_style::DimStyle;
use crate::mleader_style::MLeaderStyle;
use crate::Style;

fn style(id: u128, name: &str, keys: &[&str]) -> Style {
    Style {
        id: StyleId(id),
        name: name.into(),
        resource_keys: keys.iter().map(|key| (*key).to_string()).collect(),
    }
}

fn text_entity(id: u128, style_id: StyleId) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbText".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: "note".into(),
            position: point(0.0, 0.0),
            style: style_id,
            height: 2.5,
            rotation: 0.0,
            font: Some("arial.ttf".into()),
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
            width_factor: 1.0,
        },
        draw_order: id as i64,
    }
}

/// `Standard` text/dim/leader rows plus a `Note` text style, a `Fancy` dim style
/// whose `DIMTXSTY` is `Standard`, and one text entity using `Note` id 1.
fn table_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(crate::Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_style(style(0, "Standard", &["txt.shx"])).unwrap();
    b.insert_style(style(1, "Note", &["arial.ttf"])).unwrap();
    b.insert_dim_style(DimStyle::standard_metric(DimStyleId(0)))
        .unwrap();
    let mut fancy = DimStyle::standard_metric(DimStyleId(1));
    fancy.name = "Fancy".into();
    b.insert_dim_style(fancy).unwrap();
    b.insert_mleader_style(MLeaderStyle::standard_metric(MLeaderStyleId(0)))
        .unwrap();
    b.insert_entity(text_entity(1, StyleId(1))).unwrap();
    b.finish().unwrap()
}

#[test]
fn create_rename_and_delete_commit_one_revision_each() {
    let mut db = table_db();
    let start = db.revision();
    assert_eq!(
        db.next_style_id(),
        StyleId(2),
        "allocator follows the max id"
    );

    let created = style(db.next_style_id().0, "Header", &["arialbd.ttf"]);
    let id = created.id;
    let change = db
        .apply_style_table_changes(
            "create",
            TransactionId(1),
            StyleTableChanges {
                styles: &[(id, Some(created))],
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(change.before, start);
    assert_eq!(change.after, Revision(start.0 + 1));
    assert!(change.changes.is_empty(), "no fake entity change");
    assert_eq!(db.style(id).unwrap().name, "Header");
    assert_eq!(db.next_style_id(), StyleId(3));

    let mut renamed = db.style(id).unwrap().clone();
    renamed.name = "  Title  ".into();
    let change = db
        .apply_style_table_changes(
            "rename",
            TransactionId(2),
            StyleTableChanges {
                styles: &[(id, Some(renamed))],
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(change.after, Revision(start.0 + 2));
    assert_eq!(
        db.style(id).unwrap().name,
        "Title",
        "the stored name is trimmed"
    );

    let change = db
        .apply_style_table_changes(
            "delete",
            TransactionId(3),
            StyleTableChanges {
                styles: &[(id, None)],
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(change.after, Revision(start.0 + 3));
    assert!(db.style(id).is_none());
}

#[test]
fn an_empty_batch_leaves_the_revision_alone() {
    let mut db = table_db();
    let start = db.revision();
    let change = db
        .apply_style_table_changes("nothing", TransactionId(1), StyleTableChanges::default())
        .unwrap();
    assert_eq!(change.before, start);
    assert_eq!(change.after, start);
    assert!(change.changes.is_empty());
}

#[test]
fn names_are_unique_per_table_but_may_repeat_across_tables() {
    let mut db = table_db();
    // Same name as a text style is legal in the dim-style table.
    let mut same = DimStyle::standard_metric(DimStyleId(2));
    same.name = "Note".into();
    db.apply_style_table_changes(
        "dim create",
        TransactionId(1),
        StyleTableChanges {
            dim_styles: &[(DimStyleId(2), Some(same))],
            ..Default::default()
        },
    )
    .unwrap();

    // A case-insensitive duplicate inside the text-style table is refused.
    let duplicate = style(9, "standard", &[]);
    let error = db
        .apply_style_table_changes(
            "duplicate",
            TransactionId(2),
            StyleTableChanges {
                styles: &[(StyleId(9), Some(duplicate))],
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(
        format!("{error}").contains("style.duplicate_name"),
        "{error}"
    );
    assert!(db.style(StyleId(9)).is_none());
}

#[test]
fn empty_and_illegal_names_are_refused_before_any_mutation() {
    let mut db = table_db();
    let start = db.revision();
    for bad in ["   ", "a/b", "a:b"] {
        let row = style(7, bad, &[]);
        let error = db
            .apply_style_table_changes(
                "bad name",
                TransactionId(1),
                StyleTableChanges {
                    styles: &[(StyleId(7), Some(row))],
                    ..Default::default()
                },
            )
            .unwrap_err();
        let text = format!("{error}");
        assert!(
            text.contains("style.name_empty") || text.contains("style.name_illegal_char"),
            "{text}"
        );
    }
    assert_eq!(db.revision(), start);
    assert!(db.style(StyleId(7)).is_none());
}

#[test]
fn a_referenced_text_style_cannot_be_deleted() {
    let mut db = table_db();
    let error = db
        .apply_style_table_changes(
            "delete in use",
            TransactionId(1),
            StyleTableChanges {
                styles: &[(StyleId(1), None)],
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(format!("{error}").contains("style.in_use"), "{error}");

    // A text style named by DIMTXSTY is referenced too.
    let error = db
        .apply_style_table_changes(
            "delete standard",
            TransactionId(2),
            StyleTableChanges {
                styles: &[(StyleId(0), None)],
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(format!("{error}").contains("style.in_use"), "{error}");
    assert!(db.style(StyleId(0)).is_some());
}

#[test]
fn renaming_a_text_style_retargets_dim_and_leader_names_in_one_revision() {
    let mut db = table_db();
    let mut leader = MLeaderStyle::standard_metric(MLeaderStyleId(0));
    leader.text_style_name = "Standard".into();
    db.apply_style_table_changes(
        "seed leader",
        TransactionId(1),
        StyleTableChanges {
            mleader_styles: &[(MLeaderStyleId(0), Some(leader))],
            ..Default::default()
        },
    )
    .unwrap();
    // The seed batch is itself one revision; capture the state it leaves.
    let start = db.revision();
    let mut style_row = db.style(StyleId(0)).unwrap().clone();
    style_row.name = "Body".into();
    let mut dim = db.dim_style(DimStyleId(0)).unwrap().clone();
    dim.text_style_name = "Body".into();
    let mut leader_row = db.mleader_style(MLeaderStyleId(0)).unwrap().clone();
    leader_row.text_style_name = "Body".into();
    let change = db
        .apply_style_table_changes(
            "cascade rename",
            TransactionId(2),
            StyleTableChanges {
                styles: &[(StyleId(0), Some(style_row))],
                dim_styles: &[(DimStyleId(0), Some(dim))],
                mleader_styles: &[(MLeaderStyleId(0), Some(leader_row))],
            },
        )
        .unwrap();
    assert_eq!(change.after, Revision(start.0 + 1), "one revision");
    assert_eq!(db.style(StyleId(0)).unwrap().name, "Body");
    assert_eq!(db.dim_style(DimStyleId(0)).unwrap().text_style_name, "Body");
    assert_eq!(
        db.mleader_style(MLeaderStyleId(0)).unwrap().text_style_name,
        "Body"
    );
}

#[test]
fn both_standard_rows_are_undeletable_and_names_resolve_by_index() {
    let mut db = table_db();
    for error in [
        db.apply_style_table_changes(
            "delete dim standard",
            TransactionId(1),
            StyleTableChanges {
                dim_styles: &[(DimStyleId(0), None)],
                ..Default::default()
            },
        )
        .unwrap_err(),
        db.apply_style_table_changes(
            "delete leader standard",
            TransactionId(2),
            StyleTableChanges {
                mleader_styles: &[(MLeaderStyleId(0), None)],
                ..Default::default()
            },
        )
        .unwrap_err(),
    ] {
        let text = format!("{error}");
        assert!(
            text.contains("standard_undeletable"),
            "expected the Standard refusal, got {text}"
        );
    }

    // Renaming a dim style moves its exact-name index entry.
    let mut fancy = db.dim_style(DimStyleId(1)).unwrap().clone();
    fancy.name = "Detail".into();
    db.apply_style_table_changes(
        "rename dim",
        TransactionId(3),
        StyleTableChanges {
            dim_styles: &[(DimStyleId(1), Some(fancy))],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(db.dim_style_by_name("Fancy").is_none());
    assert_eq!(db.dim_style_by_name("Detail").unwrap().id, DimStyleId(1));
    // Case-insensitive fallback still resolves through the table scan.
    assert_eq!(db.dim_style_by_name("detail").unwrap().id, DimStyleId(1));
}

#[test]
fn duplicate_keys_and_delete_of_a_missing_row_are_refused() {
    let mut db = table_db();
    let start = db.revision();
    let error = db
        .apply_style_table_changes(
            "twice",
            TransactionId(1),
            StyleTableChanges {
                styles: &[(StyleId(0), None), (StyleId(0), None)],
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(format!("{error}").contains("twice"), "{error}");

    let error = db
        .apply_style_table_changes(
            "missing",
            TransactionId(2),
            StyleTableChanges {
                styles: &[(StyleId(42), None)],
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(format!("{error}").contains("unknown_id"), "{error}");
    assert_eq!(db.revision(), start);
}
