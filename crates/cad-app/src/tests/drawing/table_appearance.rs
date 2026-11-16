//! The per-cell appearance commands: `TABLEDIT FORMAT COLOR/FILL/LINEWEIGHT/
//! BORDERCOLOR` and the `TABLESTYLE` row-scoped equivalents.
//!
//! Both are the pre-selection table writes: one transaction / one undo step, and
//! every named refusal happens before the transaction, so a refused write stores
//! nothing and records no history. Split out of `table_ops.rs` for the code-size
//! ceiling.

use super::table_ops::{address, assert_no_write, create_table, stored_table_of, undo_depth};
use super::*;

use crate::app_drawing::table_payload::{
    TableColorValue, TableLineweightValue, TablePayload, TableRowStyleCommand, TableStyleCommand,
    TableStyleRowScope,
};

// ---------------------------------------------------------------------------
// The per-cell appearance (`TABLEDIT FORMAT COLOR/FILL/LINEWEIGHT/BORDERCOLOR`).
// ---------------------------------------------------------------------------

#[test]
fn tabledit_format_sets_every_appearance_field_in_one_undo_step() {
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
            color: Some(TableColorValue::Aci(1)), // red
            fill: Some(TableColorValue::Aci(2)),  // yellow
            lineweight: Some(TableLineweightValue::Millimetres(0.35)),
            border_color: Some(TableColorValue::Aci(5)), // blue
            border: None,
            margin: None,
        })),
    )
    .expect("the appearance write");
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");
    assert_eq!(
        outcome.changes.as_ref().expect("change set").after.0,
        before.0 + 1
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "tabledit.format"));

    let table = stored_table_of(&app, id);
    let cell = table.cell(0, 0).cloned().expect("cell");
    assert_eq!(cell.text_color, Some([255, 0, 0]));
    assert_eq!(cell.fill_color, Some([255, 255, 0]));
    for border in [
        cell.borders.top,
        cell.borders.right,
        cell.borders.bottom,
        cell.borders.left,
    ] {
        assert_eq!(border.color, Some([0, 0, 255]));
        assert_eq!(border.lineweight_hundredths, Some(35));
    }
    // The display follows in the same commit: the fill quad and the border
    // colour are part of the stored geometry.
    assert!(table
        .display()
        .iter()
        .any(|child| matches!(child, SemanticGeometry::Mesh(_))));
    assert!(
        table
            .display_appearances()
            .iter()
            .any(|appearance| appearance.color == Some([255, 255, 0])),
        "the fill"
    );
    assert!(
        table
            .display_appearances()
            .iter()
            .any(|appearance| appearance.color == Some([0, 0, 255])),
        "the borders"
    );
    // One neighbour is untouched.
    assert_eq!(table.cell(0, 1).unwrap().text_color, None);
    assert_eq!(table.cell(0, 1).unwrap().fill_color, None);

    // `BYLAYER` / `NONE` / `BYLAYER` clear the overrides again, in one step.
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Format {
            cell: address(1, 1),
            alignment: None,
            rotation: None,
            color: Some(TableColorValue::None),
            fill: Some(TableColorValue::None),
            lineweight: Some(TableLineweightValue::ByLayer),
            border_color: Some(TableColorValue::None),
            border: None,
            margin: None,
        })),
    )
    .expect("the clearing write");
    assert_eq!(undo_depth(&app), undo_before + 2, "one step per write");
    let cell = stored_table_of(&app, id).cell(0, 0).cloned().unwrap();
    assert_eq!(cell.text_color, None);
    assert_eq!(cell.fill_color, None);
    assert_eq!(cell.borders.top.color, None);
    assert_eq!(cell.borders.top.lineweight_hundredths, None);
}

#[test]
fn a_table_appearance_write_combines_with_alignment_in_one_step() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 2, 2);
    let undo_before = undo_depth(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(TablePayload::Format {
            cell: address(1, 2),
            alignment: Some(cad_domain::TableCellAlignment::from_dxf_code(7)), // BL
            rotation: Some(0.5),
            color: Some(TableColorValue::Aci(3)),
            fill: None,
            lineweight: None,
            border_color: None,
            border: None,
            margin: None,
        })),
    )
    .expect("alignment + colour is one write");
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");
    let cell = stored_table_of(&app, id).cell(0, 1).cloned().expect("cell");
    assert_eq!(cell.alignment.dxf_code(), 7);
    assert_eq!(cell.rotation, 0.5);
    assert_eq!(cell.text_color, Some([0, 255, 0]));
}

#[test]
fn a_non_canonical_appearance_value_is_refused_by_name_and_writes_nothing() {
    let (mut app, mut session) = application_with_document();
    let _id = create_table(&mut app, &mut session, 2, 2);
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    // An ACI index without a palette entry (the symbolic `0`) and a weight the
    // DXF 1/100 mm groups cannot carry exactly are both refused by name.
    for (payload, code) in [
        (
            TablePayload::Format {
                cell: address(1, 1),
                alignment: None,
                rotation: None,
                color: None,
                fill: None,
                lineweight: Some(TableLineweightValue::Millimetres(0.37)),
                border_color: None,
                border: None,
                margin: None,
            },
            "tabledit.format_lineweight",
        ),
        (
            TablePayload::Format {
                cell: address(1, 1),
                alignment: None,
                rotation: None,
                color: Some(TableColorValue::Aci(0)),
                fill: None,
                lineweight: None,
                border_color: None,
                border: None,
                margin: None,
            },
            "tabledit.format_color",
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

    // The same value twice is `tabledit.unchanged`, never a second transaction.
    let write = TablePayload::Format {
        cell: address(1, 1),
        alignment: None,
        rotation: None,
        color: Some(TableColorValue::Aci(1)),
        fill: None,
        lineweight: None,
        border_color: None,
        border: None,
        margin: None,
    };
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(write.clone())),
    )
    .expect("the first colour write");
    let after_first = undo_depth(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        CommandPayload::Table(Box::new(write)),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.starts_with("tabledit.unchanged")),
        "{error:?}"
    );
    assert_eq!(undo_depth(&app), after_first, "no second undo entry");
}

#[test]
fn a_tablestyle_row_appearance_re_resolves_inheriting_cells() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 3, 1);
    let undo_before = undo_depth(&app);
    let command = TableStyleCommand {
        rows: vec![TableRowStyleCommand {
            scope: Some(TableStyleRowScope::One(TableRowKind::Title)),
            text_color: Some(TableColorValue::Aci(2)),
            fill_color: Some(TableColorValue::Aci(3)),
            border_lineweight: Some(TableLineweightValue::Millimetres(0.5)),
            border_color: Some(TableColorValue::Aci(5)),
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
    .expect("a row-scoped appearance write");
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");

    let table = stored_table_of(&app, id);
    let title = table.style().row_style(TableRowKind::Title);
    assert_eq!(title.text_color, Some([255, 255, 0]));
    assert_eq!(title.fill_color, Some([0, 255, 0]));
    assert_eq!(title.borders.top.lineweight_hundredths, Some(50));
    assert_eq!(title.borders.left.color, Some([0, 0, 255]));
    // Row 0 is the title row: its cell re-resolved...
    let title_cell = table.cell(0, 0).cloned().expect("title cell");
    assert_eq!(title_cell.text_color, Some([255, 255, 0]));
    assert_eq!(title_cell.fill_color, Some([0, 255, 0]));
    assert_eq!(title_cell.borders.bottom.color, Some([0, 0, 255]));
    // ...and a data row's cell did not.
    let data_cell = table.cell(1, 0).cloned().expect("data cell");
    assert_eq!(data_cell.text_color, None);
    assert_eq!(data_cell.fill_color, None);
    assert_eq!(data_cell.borders.bottom.color, None);

    // The report names the appearance of each row value set.
    let report = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableStyleList),
        CommandPayload::None,
    )
    .expect("the TABLESTYLE report");
    let message = &report.diagnostics[0].message;
    assert!(message.contains("text colour"), "{message}");
    assert!(message.contains("fill"), "{message}");
}
