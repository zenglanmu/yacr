//! Extended batch read-only report contracts: `BCOUNT`, `COUNT`, `EDITTIME`,
//! `JULIAN`, `XLIST`.
//!
//! Every handler is a pure read: no transaction, no revision bump, no undo step
//! and no `ChangeSet`. `COUNT` may read the selection but never mutates it.

use super::*;
use crate::selection::model_ref;
use std::sync::Arc;

use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Layout};

fn line(id: u128, space: SpaceId, order: i64) -> DbEntity {
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
            start: point(0.0, 0.0),
            end: point(4.0, 0.0),
        },
        draw_order: order,
    }
}

fn insert(id: u128, block: BlockId, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbBlockReference".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Insert {
            block,
            transform: Transform3::identity(),
            attributes: Vec::new(),
        },
        draw_order: order,
    }
}

/// Two blocks (`CHAIR` empty, `TABLE` with one member) plus three model-space
/// INSERT references and one LINE.
fn app_with_blocks() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(line(10, SpaceId::Block(BlockId(2)), 0))
        .unwrap();
    for (id, entities) in [(BlockId(1), Vec::new()), (BlockId(2), vec![EntityId(10)])] {
        builder
            .insert_block(BlockDefinition {
                id,
                entities,
                dynamic_visibility: None,
            })
            .unwrap();
    }
    builder
        .set_block_metadata(
            BlockId(1),
            "CHAIR",
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        )
        .unwrap();
    builder
        .set_block_metadata(
            BlockId(2),
            "TABLE",
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        )
        .unwrap();
    for entity in [
        insert(1, BlockId(1), 1),
        insert(2, BlockId(1), 2),
        insert(3, BlockId(2), 3),
    ] {
        builder.insert_entity(entity).unwrap();
    }
    builder.insert_entity(line(4, SpaceId::Model, 4)).unwrap();
    let drawing = builder.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Extended(ext), CommandPayload::None),
    )
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn report<'a>(outcome: &'a CommandOutcome, code: &str) -> &'a Diagnostic {
    outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("missing {code}"))
}

#[test]
fn bcount_groups_model_space_inserts_by_block_name() {
    let (mut app, mut session) = app_with_blocks();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::BlockCount).expect("BCOUNT is admitted");
    assert!(outcome.changes.is_none());
    let block_rows: Vec<&Diagnostic> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "bcount.block")
        .collect();
    assert_eq!(block_rows.len(), 2, "one row per distinct block");
    let summary = report(&outcome, "bcount.summary");
    assert!(
        summary.message.contains("3 block instance"),
        "{}",
        summary.message
    );
    assert!(
        summary.message.contains("2 distinct"),
        "{}",
        summary.message
    );
    assert!(
        summary.message.contains("whole drawing"),
        "the whole-drawing scope is named: {}",
        summary.message
    );
    assert!(
        summary.message.contains("paper space"),
        "paper space is part of the scope: {}",
        summary.message
    );
    assert_eq!(revision(&app), before, "BCOUNT writes no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

/// A drawing with one model INSERT, one INSERT in each of two paper layouts,
/// and one nested INSERT inside `TABLE`'s definition.
fn app_with_paper_and_nested() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    let mut nested = insert(20, BlockId(1), 0);
    nested.space = SpaceId::Block(BlockId(2));
    builder.insert_entity(nested).unwrap();
    for (id, name) in [(BlockId(1), "CHAIR"), (BlockId(2), "TABLE")] {
        builder
            .insert_block(BlockDefinition {
                id,
                entities: if id == BlockId(2) {
                    vec![EntityId(20)]
                } else {
                    Vec::new()
                },
                dynamic_visibility: None,
            })
            .unwrap();
        builder
            .set_block_metadata(
                id,
                name,
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .unwrap();
    }
    for (id, name) in [(LayoutId(7), "Sheet1"), (LayoutId(8), "Sheet2")] {
        builder
            .insert_layout(Layout {
                id,
                name: name.into(),
                viewports: Vec::new(),
            })
            .unwrap();
    }
    let mut model_insert = insert(1, BlockId(2), 1);
    model_insert.space = SpaceId::Model;
    builder.insert_entity(model_insert).unwrap();
    let mut first_paper = insert(2, BlockId(1), 2);
    first_paper.space = SpaceId::Paper(LayoutId(7));
    builder.insert_entity(first_paper).unwrap();
    let mut second_paper = insert(3, BlockId(2), 3);
    second_paper.space = SpaceId::Paper(LayoutId(8));
    builder.insert_entity(second_paper).unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    (app, session)
}

#[test]
fn bcount_counts_paper_space_and_names_nested_references() {
    let (mut app, mut session) = app_with_paper_and_nested();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::BlockCount).expect("BCOUNT is admitted");
    let mut rows: Vec<(&str, &str)> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "bcount.block")
        .map(|diagnostic| {
            let (name, count) = diagnostic
                .message
                .split_once(" instances=")
                .expect("block row carries a count");
            (name, count)
        })
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        vec![
            ("BCOUNT: block 'CHAIR'", "1"),
            ("BCOUNT: block 'TABLE'", "2")
        ],
        "model space and both paper layouts are counted"
    );
    let summary = report(&outcome, "bcount.summary");
    assert!(
        summary.message.contains("3 block instance"),
        "{}",
        summary.message
    );
    assert!(
        summary.message.contains("whole drawing"),
        "{}",
        summary.message
    );
    let nested = report(&outcome, "bcount.nested_not_counted");
    assert!(
        nested.message.contains("CHAIR=1"),
        "the nested reference is named, not silently counted: {}",
        nested.message
    );
    assert_eq!(revision(&app), before, "BCOUNT writes no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn bcount_counts_only_selected_block_references() {
    let (mut app, mut session) = app_with_blocks();
    session.selection = SelectionSet::from_refs([
        model_ref(DocumentId(1), EntityId(1)),
        model_ref(DocumentId(1), EntityId(4)),
    ]);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::BlockCount).expect("BCOUNT is admitted");
    let summary = report(&outcome, "bcount.summary");
    assert!(
        summary.message.contains("current selection"),
        "{}",
        summary.message
    );
    assert!(
        summary.message.contains("1 block instance"),
        "{}",
        summary.message
    );
    assert!(report(&outcome, "bcount.non_block_ignored")
        .message
        .contains("1 selected non-block"));
    assert_eq!(
        session.selection.len(),
        2,
        "BCOUNT never mutates the selection"
    );
}

#[test]
fn count_reports_model_space_when_the_selection_is_empty() {
    let (mut app, mut session) = app_with_blocks();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::CountObjects)
        .expect("COUNT is admitted");
    let type_rows: Vec<&Diagnostic> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "count.type")
        .collect();
    assert_eq!(type_rows.len(), 2, "two type keys in model space");
    let summary = report(&outcome, "count.summary");
    assert!(
        summary.message.contains("4 object(s)"),
        "{}",
        summary.message
    );
    assert!(
        summary.message.contains("model space"),
        "{}",
        summary.message
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn count_reads_the_selection_without_mutating_it() {
    let (mut app, mut session) = app_with_blocks();
    session.selection = SelectionSet::from_refs([model_ref(DocumentId(1), EntityId(1))]);
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::CountObjects)
        .expect("COUNT is admitted");
    let summary = report(&outcome, "count.summary");
    assert!(
        summary.message.contains("current selection"),
        "{}",
        summary.message
    );
    assert!(
        summary.message.contains("1 object(s)"),
        "{}",
        summary.message
    );
    assert_eq!(
        session.selection.len(),
        1,
        "COUNT never mutates the selection"
    );
    assert_eq!(revision(&app), before);
}

#[test]
fn edittime_reports_clock_and_names_the_untracked_sources() {
    let (mut app, mut session) = app_with_blocks();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::EditTime).expect("EDITTIME is admitted");
    assert!(report(&outcome, "edittime.report")
        .message
        .contains("revision"));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "edittime.not_stored"));
    let untracked = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "edittime.not_tracked")
        .count();
    assert_eq!(untracked, 1, "active editing time/uptime is not tracked");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn edittime_reports_the_stored_creation_and_update_instants() {
    let (mut app, mut session) = app_with_blocks();
    let drawing = Arc::make_mut(
        &mut app
            .workspace
            .documents
            .get_mut(&DocumentId(1))
            .unwrap()
            .drawing,
    );
    drawing.record_document_saved_at(1_700_000_000);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::EditTime).expect("EDITTIME is admitted");
    let creation = report(&outcome, "edittime.creation");
    assert!(
        creation.message.contains("2023-11-14T22:13:20Z"),
        "{}",
        creation.message
    );
    let updated = report(&outcome, "edittime.last_updated");
    assert!(
        updated.message.contains("2023-11-14T22:13:20Z"),
        "{}",
        updated.message
    );
    assert!(
        !outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "edittime.not_stored"),
        "both stored instants are reported"
    );
}

#[test]
fn julian_reports_a_day_number_for_the_current_instant() {
    let (mut app, mut session) = app_with_blocks();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::JulianDate).expect("JULIAN is admitted");
    let value = report(&outcome, "julian.value");
    assert!(value.message.contains("integer day"), "{}", value.message);
    assert!(
        value.message.contains("current UTC instant"),
        "{}",
        value.message
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));

    // The typed date form converts a real calendar instant: 2000-01-01 12:00 UT
    // is the J2000 epoch (2451545.0), and the Unix epoch is 2440587.5.
    for (date, expected) in [
        ("2000-01-01T12:00:00", "2451545.000000"),
        ("1970-01-01", "2440587.500000"),
        ("2026/11/24", "2461368.500000"),
    ] {
        let outcome = app
            .execute(
                &mut session,
                command(
                    CommandId::Extended(ExtendedCommandId::JulianDate),
                    CommandPayload::JulianDate(date.into()),
                ),
            )
            .expect("a real date converts");
        let value = report(&outcome, "julian.value");
        assert!(
            value.message.contains(expected),
            "{date}: expected {expected}, got {}",
            value.message
        );
    }
    assert_eq!(revision(&app), before);

    // Impossible dates are named refusals; nothing is clamped.
    for date in [
        "2026-02-30",
        "2026-13-01",
        "2026-01-01T24:00",
        "not-a-date",
        "202601",
    ] {
        let error = app
            .execute(
                &mut session,
                command(
                    CommandId::Extended(ExtendedCommandId::JulianDate),
                    CommandPayload::JulianDate(date.into()),
                ),
            )
            .expect_err("an impossible date is refused");
        assert!(
            error.to_string().contains("julian.invalid_date"),
            "{date}: {error}"
        );
    }
    assert_eq!(revision(&app), before, "refusals write nothing");
}

#[test]
fn xlist_lists_block_definitions_and_declares_xdata_loss() {
    let (mut app, mut session) = app_with_blocks();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::XList).expect("XLIST is admitted");
    let block_rows = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "xlist.block")
        .count();
    assert_eq!(block_rows, 2, "one row per block definition");
    assert!(report(&outcome, "xlist.summary")
        .message
        .contains("2 block definition"));
    assert!(report(&outcome, "xlist.xdata_not_retained")
        .message
        .contains("not retained"));
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn time_names_every_untracked_field_and_reports_only_real_values() {
    let (mut app, mut session) = app_with_blocks();
    let before = revision(&app);
    let outcome = app
        .execute(&mut session, command(CommandId::Time, CommandPayload::None))
        .expect("TIME is admitted");

    let document = report(&outcome, "time.document");
    assert!(
        document.message.contains("database"),
        "{}",
        document.message
    );
    assert!(
        document.message.contains("entities "),
        "{}",
        document.message
    );
    assert!(
        report(&outcome, "time.tracked").message.contains("tracked"),
        "the report names what it can measure"
    );
    // The clock is real; an unsaved document names both missing instants rather
    // than printing an epoch.
    assert!(report(&outcome, "time.utc").message.contains("current UTC"));
    let not_stored = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "time.not_stored")
        .count();
    assert_eq!(not_stored, 2, "creation and last-update are not stored yet");
    let untracked: Vec<&str> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "time.not_tracked")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert_eq!(untracked.len(), 2, "{untracked:?}");
    for field in ["elapsed command time", "elapsed timer"] {
        assert!(
            untracked.iter().any(|message| message.contains(field)),
            "missing {field}: {untracked:?}"
        );
    }
    assert!(report(&outcome, "time.uptime_not_tracked")
        .message
        .contains("uptime"));
    assert!(report(&outcome, "time.local_not_tracked")
        .message
        .contains("local-time"));
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn time_reports_the_stored_creation_and_update_instants() {
    let (mut app, mut session) = app_with_blocks();
    let drawing = Arc::make_mut(
        &mut app
            .workspace
            .documents
            .get_mut(&DocumentId(1))
            .unwrap()
            .drawing,
    );
    // A first save, then a later one: creation stays, update moves.
    drawing.record_document_saved_at(1_700_000_000);
    drawing.record_document_saved_at(1_700_000_100);
    let outcome = app
        .execute(&mut session, command(CommandId::Time, CommandPayload::None))
        .expect("TIME is admitted");
    let creation = report(&outcome, "time.creation");
    assert!(
        creation.message.contains("2023-11-14T22:13:20Z"),
        "{}",
        creation.message
    );
    let updated = report(&outcome, "time.last_updated");
    assert!(
        updated.message.contains("2023-11-14T22:15:00Z"),
        "{}",
        updated.message
    );
    assert!(
        !outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "time.not_stored"),
        "both stored instants are reported"
    );
}
