//! Extended batch read-only report contracts: `BCOUNT`, `COUNT`, `EDITTIME`,
//! `JULIAN`, `XLIST`.
//!
//! Every handler is a pure read: no transaction, no revision bump, no undo step
//! and no `ChangeSet`. `COUNT` may read the selection but never mutates it.

use super::*;
use crate::selection::model_ref;

use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

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
        summary.message.contains("whole model space"),
        "the divergence is declared: {}",
        summary.message
    );
    assert_eq!(revision(&app), before, "BCOUNT writes no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
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
    let untracked = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "edittime.not_tracked")
        .count();
    assert_eq!(
        untracked, 2,
        "creation/edit time and uptime are not tracked"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
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
        value.message.contains("NOT IMPLEMENTED"),
        "date arguments are declared partial: {}",
        value.message
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
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
    // The clock is real; creation/last update/elapsed command/elapsed timer are
    // each named `time.not_tracked` rather than printed as a made-up value.
    assert!(report(&outcome, "time.utc").message.contains("current UTC"));
    let untracked: Vec<&str> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "time.not_tracked")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert_eq!(untracked.len(), 4, "{untracked:?}");
    for field in [
        "creation time",
        "last-update time",
        "elapsed command time",
        "elapsed timer",
    ] {
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
