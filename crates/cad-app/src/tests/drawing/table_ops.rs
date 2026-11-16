//! `TABLEDIT MERGE|UNMERGE`, `MATCHCELL` and `TABLESTYLE` contracts.
//!
//! All three are the pre-selection table writes: exactly one directly
//! addressable table, one transaction / one undo step, and every named refusal
//! happens before the transaction (no revision, no undo entry, no stored change).
//! The style write is the one that must also reach the cells that inherit the
//! style, which is what makes `TABLESTYLE` a real style table rather than a
//! relabelling.

use super::*;
use cad_history::History;

use cad_domain::{
    TableCellBorders, TableCellGeometry, TableColumnGeometry, TableFrame, TableGeometry,
    TableParts, TableRowGeometry,
};

use crate::app_drawing::table_payload::{
    TableCellAddress, TablePayload, TableRowStyleCommand, TableStyleCommand, TableStyleEdge,
    TableStyleRowScope,
};
use crate::command_payload::TableCreateCommand;

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(History::undo_depth)
        .unwrap_or(0)
}

fn create_table(
    app: &mut Application,
    session: &mut SessionState,
    rows: u32,
    cols: u32,
) -> EntityId {
    let outcome = execute(
        app,
        session,
        CommandId::Extended(ExtendedCommandId::TableCreate),
        CommandPayload::TableCreate(Box::new(TableCreateCommand {
            rows,
            cols,
            col_width: 4.0,
            row_height: 3.0,
            origin: p3(0.0, 0.0, 0.0),
        })),
    )
    .expect("TABLE creates one table");
    let id = EntityId(outcome.objects[0].0);
    session.selection.replace([ref_of(id.0)]);
    id
}

fn stored_table_of(app: &Application, id: EntityId) -> TableGeometry {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(id)
        .expect("the table entity");
    match &entity.geometry {
        SemanticGeometry::Table(table) => table.as_ref().clone(),
        other => panic!("expected a table, got {other:?}"),
    }
}

fn assert_no_write(app: &Application, before_revision: Revision, before_undo: usize) {
    assert_eq!(
        revision(app),
        before_revision,
        "a refusal bumps no revision"
    );
    assert_eq!(undo_depth(app), before_undo, "a refusal records no undo");
}

fn address(row: u32, col: u32) -> TableCellAddress {
    TableCellAddress { row, col }
}

#[test]
fn tabledit_merge_is_one_undo_step_and_unmerge_reverses_it() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 2, 2);
    let before = revision(&app);
    let undo_before = undo_depth(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Merge {
            anchor: address(1, 1),
            height: 1,
            width: 2,
        })),
    )
    .expect("MERGE writes");
    assert_eq!(
        outcome.changes.as_ref().expect("change set").after.0,
        before.0 + 1
    );
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "tabledit.merge"));
    let merged = stored_table_of(&app, id);
    assert_eq!(merged.cell(0, 0).unwrap().merge_width, 2);
    assert!(merged.is_cell_covered(0, 1));

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Unmerge {
            cell: address(1, 2),
        })),
    )
    .expect("UNMERGE of a covered address writes");
    assert_eq!(undo_depth(&app), undo_before + 2, "one more undo step");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "tabledit.merge"));
    assert_eq!(stored_table_of(&app, id).cell(0, 0).unwrap().merge_width, 1);
}

#[test]
fn a_merge_refusal_writes_nothing() {
    let (mut app, mut session) = application_with_document();
    let _id = create_table(&mut app, &mut session, 2, 2);
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);

    // Outside the 1-based address space.
    for anchor in [address(0, 1), address(1, 0), address(3, 1)] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::TableEdit),
            CommandPayload::Table(Box::new(TablePayload::Merge {
                anchor,
                height: 1,
                width: 1,
            })),
        )
        .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(message) if message.starts_with("tabledit.out_of_range")),
            "{anchor:?}: {error:?}"
        );
        assert_no_write(&app, before_revision, before_undo);
    }

    // A span that reaches past the grid, and a zero span.
    for (height, width, code) in [
        (3u32, 1u32, "table.merge_out_of_grid"),
        (0, 1, "table.merge_span"),
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::TableEdit),
            CommandPayload::Table(Box::new(TablePayload::Merge {
                anchor: address(1, 1),
                height,
                width,
            })),
        )
        .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(message) if message.starts_with(code)),
            "{error:?}"
        );
        assert_no_write(&app, before_revision, before_undo);
    }

    // Unmerging a cell that is not merged.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Unmerge {
            cell: address(1, 1),
        })),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.starts_with("table.not_merged")),
        "{error:?}"
    );
    assert_no_write(&app, before_revision, before_undo);
}

#[test]
fn tabledit_style_assigns_the_snapshot_through_the_same_write() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 1, 1);
    let undo_before = undo_depth(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Style(Box::new(TableStyleCommand {
            name: Some("Report".into()),
            ..TableStyleCommand::default()
        })))),
    )
    .expect("TABLEDIT STYLE writes");
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "tablestyle.applied"));
    assert_eq!(stored_table_of(&app, id).style_name(), "Report");
}

#[test]
fn a_table_edit_payload_on_the_wrong_command_is_refused() {
    let (mut app, mut session) = application_with_document();
    let _id = create_table(&mut app, &mut session, 2, 2);
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MatchCell),
        CommandPayload::Table(Box::new(TablePayload::Merge {
            anchor: address(1, 1),
            height: 1,
            width: 1,
        })),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.starts_with("matchcell.payload")),
        "{error:?}"
    );
    assert_no_write(&app, before_revision, before_undo);
}

/// A 2x2 table whose (0, 0) cell carries its own explicit style: taller text, the
/// `Title` text style and a hidden top border. The other cells inherit the
/// `Standard` style, so a style copy has something real to move.
fn table_with_explicit_cell_app() -> (Application, SessionState) {
    let mut source = cell_parts();
    source.rows[0].cells[0].text = "source".into();
    source.rows[0].cells[0].text_height = 5.0;
    source.rows[0].cells[0].style_name = "Title".into();
    source.rows[0].cells[0].borders.top = cad_domain::TableBorder::HIDDEN;
    source.rows[0].cells[0].alignment = cad_domain::TableCellAlignment::from_dxf_code(3); // TR
    source.rows[0].cells[0].rotation = 0.25;
    let geometry = TableGeometry::new(source).expect("valid table");

    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(1, SemanticGeometry::Table(Box::new(geometry)), 0))
        .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([11u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    session.selection.replace([ref_of(1)]);
    (app, session)
}

fn cell_parts() -> TableParts {
    TableParts {
        frame: TableFrame::default(),
        columns: vec![
            TableColumnGeometry {
                width: 4.0,
                name: String::new(),
            },
            TableColumnGeometry {
                width: 4.0,
                name: String::new(),
            },
        ],
        rows: vec![
            TableRowGeometry {
                height: 3.0,
                cells: vec![plain_cell(), plain_cell()],
            },
            TableRowGeometry {
                height: 3.0,
                cells: vec![plain_cell(), plain_cell()],
            },
        ],
        style_name: "Standard".into(),
        name: String::new(),
        description: String::new(),
    }
}

fn plain_cell() -> TableCellGeometry {
    TableCellGeometry {
        text: String::new(),
        text_height: cad_domain::DEFAULT_CELL_TEXT_HEIGHT,
        style_name: "Standard".into(),
        style: StyleId(0),
        font: None,
        alignment: cad_domain::TableCellAlignment::DEFAULT,
        rotation: 0.0,
        merge_width: 1,
        merge_height: 1,
        borders: TableCellBorders::default(),
    }
}

#[test]
fn matchcell_copies_the_source_cell_style_to_every_target_in_one_step() {
    let (mut app, mut session) = table_with_explicit_cell_app();
    let undo_before = undo_depth(&app);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MatchCell),
        CommandPayload::Table(Box::new(TablePayload::MatchCell {
            source: address(1, 1),
            targets: vec![address(2, 2), address(1, 2)],
        })),
    )
    .expect("MATCHCELL writes");
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");
    assert_eq!(
        outcome.changes.as_ref().expect("change set").after.0,
        before.0 + 1
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "matchcell.applied"));
    let table = stored_table_of(&app, EntityId(1));
    let source = table.cell(0, 0).expect("source").clone();
    for (row, column) in [(1, 1), (0, 1)] {
        let target = table.cell(row, column).expect("target");
        assert_eq!(target.text_height, source.text_height);
        assert_eq!(target.style_name, source.style_name);
        assert_eq!(target.style, source.style);
        assert_eq!(target.borders, source.borders);
        assert_eq!(target.alignment, source.alignment);
        assert_eq!(target.rotation, source.rotation);
        // A format copy never moves text.
        assert_eq!(target.text, "");
    }
    assert_eq!(table.cell(0, 0).unwrap().text, "source");
}

#[test]
fn matchcell_refuses_a_missing_target_or_an_out_of_range_cell() {
    let (mut app, mut session) = table_with_explicit_cell_app();
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MatchCell),
        CommandPayload::Table(Box::new(TablePayload::MatchCell {
            source: address(1, 1),
            targets: Vec::new(),
        })),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.starts_with("matchcell.no_targets")),
        "{error:?}"
    );
    assert_no_write(&app, before_revision, before_undo);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MatchCell),
        CommandPayload::Table(Box::new(TablePayload::MatchCell {
            source: address(1, 1),
            targets: vec![address(3, 1)],
        })),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.starts_with("matchcell.out_of_range")),
        "{error:?}"
    );
    assert_no_write(&app, before_revision, before_undo);
}

#[test]
fn tablestyle_reports_without_writing_and_writes_the_style_once() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 2, 2);
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);

    // The bare command is the read-only report.
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableStyleList),
        CommandPayload::None,
    )
    .expect("report");
    assert!(outcome.changes.is_none());
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "tablestyle.report"));
    assert_no_write(&app, before_revision, before_undo);

    // The write assigns a named snapshot and re-resolves the inheriting cells.
    let command = TableStyleCommand {
        name: Some("Report".into()),
        horizontal_margin: Some(0.2),
        rows: vec![TableRowStyleCommand {
            text_height: Some(4.5),
            text_style_name: Some("Standard".into()),
            borders: vec![(TableStyleEdge::Bottom, false, None)],
            ..TableRowStyleCommand::default()
        }],
        ..TableStyleCommand::default()
    };
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableStyleList),
        CommandPayload::Table(Box::new(TablePayload::Style(Box::new(command)))),
    )
    .expect("TABLESTYLE writes");
    assert_eq!(undo_depth(&app), before_undo + 1, "one undo step");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "tablestyle.applied"));
    let table = stored_table_of(&app, id);
    assert_eq!(table.style_name(), "Report");
    assert!((table.style().text_height - 4.5).abs() < 1e-9);
    assert!((table.style().horizontal_margin - 0.2).abs() < 1e-9);
    assert_eq!(
        table.style().borders.bottom,
        cad_domain::TableBorder::HIDDEN
    );
    // Every cell inherited the old standard values, so every cell follows.
    for row in 0..2 {
        for column in 0..2 {
            let cell = table.cell(row, column).expect("cell");
            assert!((cell.text_height - 4.5).abs() < 1e-9, "({row}, {column})");
            assert_eq!(cell.borders.bottom, cad_domain::TableBorder::HIDDEN);
        }
    }
}

#[test]
fn a_tablestyle_refusal_writes_nothing() {
    let (mut app, mut session) = application_with_document();
    let _id = create_table(&mut app, &mut session, 2, 2);
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);

    // No field to write.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableStyleList),
        CommandPayload::Table(Box::new(TablePayload::Style(Box::default()))),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.starts_with("tablestyle.empty")),
        "{error:?}"
    );
    assert_no_write(&app, before_revision, before_undo);

    // A non-positive height and a negative margin.
    for command in [
        TableStyleCommand {
            rows: vec![TableRowStyleCommand {
                text_height: Some(0.0),
                ..TableRowStyleCommand::default()
            }],
            ..TableStyleCommand::default()
        },
        TableStyleCommand {
            horizontal_margin: Some(-0.5),
            ..TableStyleCommand::default()
        },
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::TableStyleList),
            CommandPayload::Table(Box::new(TablePayload::Style(Box::new(command)))),
        )
        .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(message) if message.starts_with("tablestyle.invalid")),
            "{error:?}"
        );
        assert_no_write(&app, before_revision, before_undo);
    }

    // A style that is already exactly the table's own.
    let current = stored_table_of(&app, _id).style().clone();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableStyleList),
        CommandPayload::Table(Box::new(TablePayload::Style(Box::new(TableStyleCommand {
            name: Some(current.name.clone()),
            ..TableStyleCommand::default()
        })))),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.starts_with("tablestyle.unchanged")),
        "{error:?}"
    );
    assert_no_write(&app, before_revision, before_undo);
}

#[test]
fn the_post_creation_table_commands_need_one_selected_table() {
    let (mut app, mut session) = application_with_document();
    let _id = create_table(&mut app, &mut session, 2, 2);
    session.selection.clear();
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    for (ext, payload, code) in [
        (
            ExtendedCommandId::TableEdit,
            TablePayload::Merge {
                anchor: address(1, 1),
                height: 1,
                width: 2,
            },
            "tabledit.no_selection",
        ),
        (
            ExtendedCommandId::MatchCell,
            TablePayload::MatchCell {
                source: address(1, 1),
                targets: vec![address(2, 2)],
            },
            "matchcell.no_selection",
        ),
        (
            ExtendedCommandId::TableStyleList,
            TablePayload::Style(Box::new(TableStyleCommand {
                rows: vec![TableRowStyleCommand {
                    text_height: Some(4.0),
                    ..TableRowStyleCommand::default()
                }],
                ..TableStyleCommand::default()
            })),
            "tablestyle.no_selection",
        ),
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ext),
            CommandPayload::Table(Box::new(payload)),
        )
        .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(message) if message.starts_with(code)),
            "{code}: {error:?}"
        );
        assert_no_write(&app, before_revision, before_undo);
    }
}

#[test]
fn tabledit_format_sets_alignment_and_rotation_in_one_undo_step() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 2, 2);
    let undo_before = undo_depth(&app);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Format {
            cell: address(1, 2),
            alignment: Some(cad_domain::TableCellAlignment::from_dxf_code(7)), // BL
            rotation: Some(0.5),
        })),
    )
    .expect("TABLEDIT FORMAT writes");
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");
    assert_eq!(
        outcome.changes.as_ref().expect("change set").after.0,
        before.0 + 1
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "tabledit.format"));
    let cell = stored_table_of(&app, id).cell(0, 1).cloned().expect("cell");
    assert_eq!(cell.alignment.dxf_code(), 7);
    assert_eq!(cell.rotation, 0.5);
    // The neighbour is untouched.
    let neighbour = stored_table_of(&app, id).cell(0, 0).cloned().expect("cell");
    assert_eq!(neighbour.alignment, cad_domain::TableCellAlignment::DEFAULT);
    assert_eq!(neighbour.rotation, 0.0);

    // `ALIGN` and `ROTATION` alone each write their own field only.
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Format {
            cell: address(1, 2),
            alignment: None,
            rotation: Some(0.75),
        })),
    )
    .expect("a rotation-only write");
    let cell = stored_table_of(&app, id).cell(0, 1).cloned().expect("cell");
    assert_eq!(cell.alignment.dxf_code(), 7, "the alignment is kept");
    assert_eq!(cell.rotation, 0.75);
    assert_eq!(undo_depth(&app), undo_before + 2, "one step per write");
}

#[test]
fn a_tabledit_format_refusal_writes_nothing() {
    let (mut app, mut session) = application_with_document();
    let _id = create_table(&mut app, &mut session, 2, 2);
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);

    for (payload, code) in [
        (
            TablePayload::Format {
                cell: address(1, 1),
                alignment: None,
                rotation: None,
            },
            "tabledit.format_empty",
        ),
        (
            TablePayload::Format {
                cell: address(3, 1),
                alignment: Some(cad_domain::TableCellAlignment::DEFAULT),
                rotation: None,
            },
            "tabledit.out_of_range",
        ),
        (
            TablePayload::Format {
                cell: address(1, 1),
                alignment: Some(cad_domain::TableCellAlignment::DEFAULT),
                rotation: Some(0.0),
            },
            "tabledit.unchanged",
        ),
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::TableEdit),
            CommandPayload::Table(Box::new(payload)),
        )
        .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(message) if message.starts_with(code)),
            "{code}: {error:?}"
        );
        assert_no_write(&app, before_revision, before_undo);
    }
}

#[test]
fn a_tablestyle_row_scope_writes_only_that_row_value_set() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 3, 1);
    let undo_before = undo_depth(&app);
    let command = TableStyleCommand {
        rows: vec![TableRowStyleCommand {
            scope: Some(TableStyleRowScope::One(TableRowKind::Title)),
            text_height: Some(6.0),
            alignment: Some(cad_domain::TableCellAlignment::from_dxf_code(2)), // TC
            ..TableRowStyleCommand::default()
        }],
        ..TableStyleCommand::default()
    };
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableStyleList),
        CommandPayload::Table(Box::new(TablePayload::Style(Box::new(command)))),
    )
    .expect("a row-scoped TABLESTYLE write");
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");
    let table = stored_table_of(&app, id);
    let style = table.style();
    assert_eq!(style.row_style(TableRowKind::Title).text_height, 6.0);
    assert_eq!(
        style.data_row().text_height,
        cad_domain::DEFAULT_CELL_TEXT_HEIGHT,
        "the data row is untouched"
    );
    // Row 0 is the title row, so only its cells re-resolve.
    assert_eq!(table.cell(0, 0).unwrap().text_height, 6.0);
    assert_eq!(
        table.cell(0, 0).unwrap().alignment.dxf_code(),
        2,
        "the title row's alignment reached its cells"
    );
    assert_eq!(
        table.cell(1, 0).unwrap().text_height,
        cad_domain::DEFAULT_CELL_TEXT_HEIGHT
    );
    assert_eq!(
        table.cell(1, 0).unwrap().alignment,
        cad_domain::TableCellAlignment::DEFAULT
    );
}
