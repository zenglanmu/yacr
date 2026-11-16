//! The `GRIDFORMAT` command paths: `TABLEDIT FORMAT BORDER/MARGIN`, the margin
//! refusal and `MATCHCELL`'s margin copy — each one transaction / one undo step.
//!
//! Split out of `table_ops.rs` for the code-size ceiling; the fixtures it uses
//! (`create_table`, `stored_table_of`, `address`, `undo_depth`,
//! `assert_no_write`) are that module's, so the two cannot drift.

use super::table_ops::{address, assert_no_write, create_table, stored_table_of, undo_depth};
use super::*;

use cad_domain::{TableCellMargins, TableGeometry, TableRowKind};

use crate::app_drawing::table_payload::{TableCellBorderCommand, TableMarginValue, TablePayload};

// ---------------------------------------------------------------------------
// `GRIDFORMAT`: the border type (double-line spacing) and the cell margins.
// ---------------------------------------------------------------------------

#[test]
fn tabledit_format_sets_the_border_type_and_margins_in_one_undo_step() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 2, 2);
    let undo_before = undo_depth(&app);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Format {
            cell: address(1, 1),
            alignment: None,
            rotation: None,
            color: None,
            fill: None,
            lineweight: None,
            border_color: None,
            border: Some(TableCellBorderCommand {
                visible: Some(true),
                spacing: Some(0.4),
            }),
            margin: Some(TableMarginValue::Values(TableCellMargins::new(
                0.2, 0.3, 0.4, 0.5,
            ))),
        })),
    )
    .expect("the GRIDFORMAT write");
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");
    assert_eq!(
        outcome.changes.as_ref().expect("change set").after.0,
        before.0 + 1
    );
    let message = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "tabledit.format")
        .expect("the command reports the write")
        .message
        .clone();
    assert!(message.contains("double(0.4)"), "{message}");
    assert!(message.contains("0.2/0.3/0.4/0.5"), "{message}");

    let table = stored_table_of(&app, id);
    let cell = table.cell(0, 0).cloned().expect("cell");
    for border in [
        cell.borders.top,
        cell.borders.right,
        cell.borders.bottom,
        cell.borders.left,
    ] {
        assert_eq!(border.double_spacing, 0.4, "a spacing above zero is Double");
    }
    assert_eq!(
        table.cell_margins(0, 0),
        Some(TableCellMargins::new(0.2, 0.3, 0.4, 0.5))
    );
    // The neighbour still inherits.
    assert_eq!(table.cell_margins(0, 1), Some(TableCellMargins::INHERIT));

    // `BORDER SINGLE` clears the spacing again and `MARGIN INHERIT` clears the
    // margins, each in its own step.
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Format {
            cell: address(1, 1),
            alignment: None,
            rotation: None,
            color: None,
            fill: None,
            lineweight: None,
            border_color: None,
            border: Some(TableCellBorderCommand {
                visible: Some(true),
                spacing: Some(0.0),
            }),
            margin: Some(TableMarginValue::Inherit),
        })),
    )
    .expect("the clearing write");
    let table = stored_table_of(&app, id);
    assert_eq!(
        table.cell(0, 0).expect("cell").borders.top.double_spacing,
        0.0
    );
    assert_eq!(table.cell_margins(0, 0), Some(TableCellMargins::INHERIT));
    assert_eq!(undo_depth(&app), undo_before + 2, "one step per write");
}

#[test]
fn a_non_finite_or_negative_margin_is_refused_by_name_and_writes_nothing() {
    let (mut app, mut session) = application_with_document();
    let _id = create_table(&mut app, &mut session, 2, 2);
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Format {
            cell: address(1, 1),
            alignment: None,
            rotation: None,
            color: None,
            fill: None,
            lineweight: None,
            border_color: None,
            border: None,
            margin: Some(TableMarginValue::Values(TableCellMargins::new(
                -1.0, 0.0, 0.0, 0.0,
            ))),
        })),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.starts_with("tabledit.format_margin")),
        "{error:?}"
    );
    assert_no_write(&app, before_revision, before_undo);
}

#[test]
fn matchcell_copies_the_margin_override_onto_every_target() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 2, 2);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Format {
            cell: address(1, 1),
            alignment: None,
            rotation: None,
            color: None,
            fill: None,
            lineweight: None,
            border_color: None,
            border: None,
            margin: Some(TableMarginValue::Values(TableCellMargins::uniform(0.6))),
        })),
    )
    .expect("the margin write");
    let undo_before = undo_depth(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MatchCell),
        CommandPayload::Table(Box::new(TablePayload::MatchCell {
            source: address(1, 1),
            targets: vec![address(1, 2), address(2, 1)],
        })),
    )
    .expect("MATCHCELL writes");
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");
    let message = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "matchcell.applied")
        .expect("the command reports the copy")
        .message
        .clone();
    assert!(message.contains("margins 0.6"), "{message}");
    let table = stored_table_of(&app, id);
    for (row, column) in [(0, 0), (0, 1), (1, 0)] {
        assert_eq!(
            table.cell_margins(row, column),
            Some(TableCellMargins::uniform(0.6))
        );
    }
}

#[test]
fn a_margin_override_moves_the_displayed_cell_text() {
    // The display is the only authority on where a cell's text is drawn, so the
    // margin is checked on the stored primitives rather than on a screenshot.
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 1, 1);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::TableEdit(Box::new(crate::command_payload::TableEditCommand {
            row: 1,
            col: 1,
            text: "cell".into(),
        })),
    )
    .expect("the cell text write");
    let centre = text_position(&stored_table_of(&app, id), "cell");
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Format {
            cell: address(1, 1),
            alignment: None,
            rotation: None,
            color: None,
            fill: None,
            lineweight: None,
            border_color: None,
            border: None,
            margin: Some(TableMarginValue::Values(TableCellMargins::new(
                1.0, 1.0, 0.0, 0.0,
            ))),
        })),
    )
    .expect("the margin write");
    let inset = text_position(&stored_table_of(&app, id), "cell");
    assert!(
        (inset.x - centre.x - 0.5).abs() < 1e-9,
        "a 1.0 left margin on a 4.0-wide cell moves a centred text by 0.5: {centre:?} -> {inset:?}"
    );
    assert!(inset.y < centre.y, "a 1.0 top margin lowers the text");
}

/// The position of one cell text in the derived display.
fn text_position(table: &TableGeometry, text: &str) -> cad_domain::Point3 {
    for child in table.display() {
        if let SemanticGeometry::Text {
            text: drawn,
            position,
            ..
        } = child
        {
            if drawn == text {
                return *position;
            }
        }
    }
    panic!("no Text primitive for `{text}`");
}

#[test]
fn tablestyle_row_border_single_and_double_rewrite_the_inheriting_cells() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 1, 1);
    let undo_before = undo_depth(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableStyleList),
        CommandPayload::Table(Box::new(TablePayload::Style(Box::new(TableStyleCommand {
            rows: vec![TableRowStyleCommand {
                scope: Some(TableStyleRowScope::One(TableRowKind::Data)),
                borders: vec![(TableStyleEdge::All, true, Some(0.5))],
                ..TableRowStyleCommand::default()
            }],
            ..TableStyleCommand::default()
        })))),
    )
    .expect("the row doubling");
    let table = stored_table_of(&app, id);
    let cell = table.cell(0, 0).expect("cell");
    assert_eq!(cell.borders.top.double_spacing, 0.5);
    assert_eq!(table.style().borders.top.double_spacing, 0.5);
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");

    // `SINGLE` clears the spacing on the row **and** on the cells that inherit
    // it, which is what makes the border type changeable at all.
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableStyleList),
        CommandPayload::Table(Box::new(TablePayload::Style(Box::new(TableStyleCommand {
            rows: vec![TableRowStyleCommand {
                scope: Some(TableStyleRowScope::One(TableRowKind::Data)),
                borders: vec![(TableStyleEdge::All, true, Some(0.0))],
                ..TableRowStyleCommand::default()
            }],
            ..TableStyleCommand::default()
        })))),
    )
    .expect("the row single write");
    let table = stored_table_of(&app, id);
    assert_eq!(
        table.cell(0, 0).expect("cell").borders.top.double_spacing,
        0.0
    );
    assert_eq!(table.style().borders.top.double_spacing, 0.0);
    assert_eq!(undo_depth(&app), undo_before + 2, "one step per write");
}
