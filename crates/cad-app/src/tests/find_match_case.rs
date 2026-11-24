//! `FINDMATCH` (2026-11-24): the session flag that makes `-FIND`/`FIND` compare
//! **case-sensitively**.
//!
//! The flag is the session's own token (default OFF, AutoCAD's documented
//! default) and it is read by the one matcher
//! ([`crate::Application::find_text`]) for every text channel, both for the
//! read-only `-FIND` report and for the `FIND … => …` replacement. These tests
//! pin both states over the channels the matcher really covers: MTEXT markup, a
//! single-line `AcDbText` and table cells (which the viewport draws through the
//! same text pipeline).

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    TableCellBorders, TableCellGeometry, TableColumnGeometry, TableFrame, TableGeometry,
    TableParts, TableRowGeometry,
};

const MTEXT: EntityId = EntityId(11);
const TEXT: EntityId = EntityId(12);
const TABLE: EntityId = EntityId(13);

fn entity(id: EntityId, type_key: &str, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id.0),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id,
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: 0,
    }
}

fn text(value: &str, x: f64) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: value.into(),
        position: point(x, 0.0),
        style: StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
        line_spacing: cad_domain::TextLineSpacing::default(),
    }
}

fn cell(value: &str) -> TableCellGeometry {
    TableCellGeometry {
        text: value.to_string(),
        text_height: 2.5,
        style_name: "Standard".into(),
        style: StyleId(0),
        font: None,
        alignment: cad_domain::TableCellAlignment::DEFAULT,
        rotation: 0.0,
        merge_width: 1,
        merge_height: 1,
        borders: TableCellBorders::default(),
        text_color: None,
        fill_color: None,
    }
}

/// A one-row, two-cell table: `ALPHA` | `alpha`.
fn table() -> TableGeometry {
    TableGeometry::new(TableParts {
        frame: TableFrame::default(),
        columns: vec![
            TableColumnGeometry {
                width: 4.0,
                name: "A".into(),
            },
            TableColumnGeometry {
                width: 4.0,
                name: "B".into(),
            },
        ],
        rows: vec![TableRowGeometry {
            height: 3.0,
            cells: vec![cell("ALPHA"), cell("alpha")],
        }],
        style_name: "Standard".into(),
        name: "Data".into(),
        description: String::new(),
    })
    .expect("valid table")
}

/// One MTEXT with a bold run (`Alpha`), one single-line TEXT (`Alpha`) and the
/// table above. The three channels agree on the letters `alpha` but not on the
/// case, so the two comparison modes differ on every one of them.
fn find_app() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity(
        MTEXT,
        "AcDbMText",
        text("{\\fArial|b1;Alpha} beta", 0.0),
    ))
    .unwrap();
    b.insert_entity(entity(TEXT, "AcDbText", text("Alpha", 10.0)))
        .unwrap();
    b.insert_entity(entity(
        TABLE,
        "AcDbTable",
        SemanticGeometry::Table(Box::new(table())),
    ))
    .unwrap();
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    needle: &str,
    replacement: Option<&str>,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(
            id,
            CommandPayload::TextFind {
                needle: needle.into(),
                replacement: replacement.map(str::to_string),
            },
        ),
    )
}

fn report_message(outcome: &CommandOutcome) -> &str {
    &outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "find.result")
        .expect("a find report")
        .message
}

fn stored_text(app: &Application, id: EntityId) -> String {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(id)
        .expect("entity exists")
        .geometry
    {
        SemanticGeometry::Text { text, .. } => text.clone(),
        other => panic!("expected a text geometry, got {other:?}"),
    }
}

fn stored_cells(app: &Application) -> Vec<String> {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(TABLE)
        .expect("the table exists")
        .geometry
    {
        SemanticGeometry::Table(table) => table
            .rows()
            .iter()
            .flat_map(|row| row.cells.iter().map(|cell| cell.text.clone()))
            .collect(),
        other => panic!("expected a table, got {other:?}"),
    }
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

#[test]
fn findmatch_defaults_off_and_the_report_stays_case_insensitive() {
    let (mut app, mut session) = find_app();
    assert!(
        !session.drafting_extras.find_match_case(),
        "FINDMATCH starts OFF (AutoCAD's documented default)"
    );

    // MTEXT + TEXT + the two table cells all draw the letters `alpha`, in mixed
    // case. Off means ASCII case-insensitive: every one of them is a hit.
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "aLpHa", None).unwrap();
    let message = report_message(&outcome);
    assert!(message.contains("4 match(es)"), "{message}");
    assert!(
        message.contains("FINDMATCH off: ASCII case-insensitive"),
        "{message}"
    );
}

#[test]
fn findmatch_on_makes_the_report_compare_exact_case() {
    let (mut app, mut session) = find_app();
    session.drafting_extras.set_find_match_case(true);

    // `Alpha` is exactly the MTEXT run text and the single-line TEXT; the table
    // cells `ALPHA`/`alpha` are not.
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "Alpha", None).unwrap();
    let message = report_message(&outcome);
    assert!(message.contains("2 match(es)"), "{message}");
    assert!(
        message.contains("FINDMATCH on: case-sensitive"),
        "{message}"
    );

    // `alpha` is exactly the lower-case table cell: one cell hit, no TEXT/MTEXT.
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "alpha", None).unwrap();
    let message = report_message(&outcome);
    assert!(message.contains("1 match(es)"), "{message}");
    assert!(
        message.contains("row 0, column 1"),
        "the cell is named: {message}"
    );
}

#[test]
fn findmatch_on_replaces_only_the_exact_case_and_keeps_mtext_run_codes() {
    let (mut app, mut session) = find_app();
    session.drafting_extras.set_find_match_case(true);
    let before_revision = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let before_undo = undo_depth(&app);

    let outcome = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "Alpha",
        Some("Zulu"),
    )
    .unwrap();
    assert!(
        outcome.changes.is_some(),
        "the exact-case match is replaced"
    );
    assert!(
        stored_text(&app, MTEXT).contains("{\\fArial|b1;Zulu} beta"),
        "the bold run keeps its codes: {}",
        stored_text(&app, MTEXT)
    );
    assert_eq!(stored_text(&app, TEXT), "Zulu");
    assert_eq!(
        stored_cells(&app),
        vec!["ALPHA".to_string(), "alpha".to_string()],
        "table cells with a different case are untouched"
    );
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        Revision(before_revision.0 + 1)
    );
    assert_eq!(undo_depth(&app), before_undo + 1, "one undo step");
}

#[test]
fn findmatch_off_replaces_every_case_across_mtext_and_table_cells() {
    let (mut app, mut session) = find_app();
    assert!(!session.drafting_extras.find_match_case());
    let before_undo = undo_depth(&app);

    let outcome = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "alpha",
        Some("omega"),
    )
    .unwrap();
    assert!(outcome.changes.is_some());
    assert!(
        stored_text(&app, MTEXT).contains("{\\fArial|b1;omega} beta"),
        "{}",
        stored_text(&app, MTEXT)
    );
    assert_eq!(stored_text(&app, TEXT), "omega");
    assert_eq!(
        stored_cells(&app),
        vec!["omega".to_string(), "omega".to_string()],
        "both table cells match case-insensitively"
    );
    assert_eq!(undo_depth(&app), before_undo + 1, "one undo step");

    // Turning the flag ON and repeating the same replacement changes nothing:
    // the text is already lower-case, so this also proves the flag is read per
    // command (no stale copy captured from the first call).
    session.drafting_extras.set_find_match_case(true);
    let revision = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let outcome = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "ALPHA",
        Some("ignored"),
    )
    .unwrap();
    assert!(
        outcome.changes.is_none(),
        "no exact-case match exists any more"
    );
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        revision,
        "a case-sensitive miss writes nothing"
    );
}

#[test]
fn the_findmatch_command_switches_the_one_matcher_both_ways() {
    let (mut app, mut session) = find_app();
    let toggle = |app: &mut Application, session: &mut SessionState| {
        app.execute(
            session,
            command(
                CommandId::Extended(ExtendedCommandId::FindMatch),
                CommandPayload::None,
            ),
        )
        .expect("FINDMATCH is admitted");
    };

    toggle(&mut app, &mut session);
    assert!(session.drafting_extras.find_match_case());
    let message =
        report_message(&run(&mut app, &mut session, CommandId::FindReport, "Alpha", None).unwrap())
            .to_string();
    assert!(message.contains("2 match(es)"), "{message}");
    assert!(
        message.contains("FINDMATCH on: case-sensitive"),
        "{message}"
    );

    toggle(&mut app, &mut session);
    assert!(!session.drafting_extras.find_match_case());
    let message =
        report_message(&run(&mut app, &mut session, CommandId::FindReport, "Alpha", None).unwrap())
            .to_string();
    assert!(
        message.contains("4 match(es)"),
        "back to insensitive: {message}"
    );
    assert!(
        message.contains("FINDMATCH off: ASCII case-insensitive"),
        "{message}"
    );
}
