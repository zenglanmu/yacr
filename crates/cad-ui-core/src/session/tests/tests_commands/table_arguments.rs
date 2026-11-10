//! The typed `TABLE` / `-TABLE` / `TABLEDIT` grammars and their dispatch.
//!
//! These pin the textual grammar (six creation tokens with 1-based-free
//! dimensions, or a 1-based cell address plus the text remainder), the
//! admission of all three selectors as argument forms of their existing
//! extended commands, and the payload path: a valid `TABLE` creates a real
//! table row, while a valid `TABLEDIT` with no selection reaches the command
//! layer and is refused there by name (`tabledit.no_selection`), which proves
//! the mapping without duplicating `cad-app`'s semantics.

use super::super::super::*;
use super::super::test_support::*;

use crate::session::commands::{
    parse_table_create_argument, parse_tabledit_argument, ARGUMENT_SELECTORS, DASH_TABLE_SELECTOR,
    TABLEDIT_SELECTOR, TABLE_SELECTOR,
};
use cad_app::{ExtendedCommandId, TableCreateCommand, TableEditCommand};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn table_argument_is_two_counts_two_sizes_and_an_origin() {
    assert_eq!(
        parse_table_create_argument("3 2 5 2 10 20"),
        Some(TableCreateCommand {
            rows: 3,
            cols: 2,
            col_width: 5.0,
            row_height: 2.0,
            origin: Point3 {
                x: 10.0,
                y: 20.0,
                z: 0.0,
            },
        })
    );
    // Sign and fraction are legal on the origin; the sizes must be positive.
    assert!(parse_table_create_argument("1 1 0.5 0.5 -10.5 20").is_some());
    for refused in [
        "",
        "3 2 5 2 10",
        "3 2 5 2 10 20 0",
        "0 2 5 2 0 0",
        "3 0 5 2 0 0",
        "-3 2 5 2 0 0",
        "3.5 2 5 2 0 0",
        "3 2 0 2 0 0",
        "3 2 5 0 0 0",
        "3 2 -5 2 0 0",
        "3 2 5 2 NaN 0",
        "3 2 5 2 0 inf",
        "3 2 5 2 1e3 0",
        "4294967296 1 1 1 0 0",
    ] {
        assert_eq!(parse_table_create_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn tabledit_argument_is_a_one_based_address_plus_the_text_remainder() {
    assert_eq!(
        parse_tabledit_argument("2 1 hello world"),
        Some(TableEditCommand {
            row: 2,
            col: 1,
            text: "hello world".into(),
        })
    );
    // Only the leading whitespace is dropped: internal/trailing spaces survive.
    assert_eq!(
        parse_tabledit_argument("  2   1   hello   world  "),
        Some(TableEditCommand {
            row: 2,
            col: 1,
            text: "hello   world  ".into(),
        })
    );
    // No text is an explicit empty string (clear), never a default.
    assert_eq!(
        parse_tabledit_argument("1 1"),
        Some(TableEditCommand {
            row: 1,
            col: 1,
            text: String::new(),
        })
    );
    for refused in [
        "",
        "2",
        "0 1 hello",
        "1 0 hello",
        "-1 1 hello",
        "1.5 1 hello",
        "a 1 hello",
        "1 a hello",
    ] {
        assert_eq!(parse_tabledit_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn all_three_selectors_are_admitted_argument_forms_of_their_extended_commands() {
    for selector in [TABLE_SELECTOR, DASH_TABLE_SELECTOR, TABLEDIT_SELECTOR] {
        assert!(
            ARGUMENT_SELECTORS.contains(&selector),
            "{selector} must be an argument selector"
        );
    }
    assert_eq!(
        normalize_command("TABLE"),
        Some(CommandId::Extended(ExtendedCommandId::TableCreate))
    );
    assert_eq!(
        normalize_command("-TABLE"),
        Some(CommandId::Extended(ExtendedCommandId::DashTable))
    );
    assert_eq!(
        normalize_command("TABLEDIT"),
        Some(CommandId::Extended(ExtendedCommandId::TableEdit))
    );
    assert_eq!(canonical_command_name("TABLE"), Some("TABLE"));
    assert_eq!(canonical_command_name("-TABLE"), Some("-TABLE"));
    assert_eq!(canonical_command_name("TABLEDIT"), Some("TABLEDIT"));
}

#[test]
fn typed_table_dispatches_a_real_table_entity() {
    let session = fresh_session();
    session
        .command("TABLE 3 2 5 2 10 20")
        .expect("the typed TABLE creates one table");
    let c = session.core.controller.borrow();
    let drawing = c.drawing().expect("a drawing is open");
    let tables: Vec<_> = drawing
        .model_space()
        .iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 1, "exactly one table row");
    let table = tables[0];
    assert_eq!(table.rows().len(), 3);
    assert_eq!(table.columns().len(), 2);
    assert!((table.columns()[0].width - 5.0).abs() < 1e-9);
    assert!((table.rows()[0].height - 2.0).abs() < 1e-9);
    assert!((table.frame().origin.x - 10.0).abs() < 1e-9);
    assert!((table.frame().origin.y - 20.0).abs() < 1e-9);
    drop(c);
    // Every cell starts empty.
    assert!(table.texts().is_empty());

    let session = fresh_session();
    session
        .command("-TABLE 1 1 2.5 2.5 -5 -5")
        .expect("the dash spelling creates the same table");
    let c = session.core.controller.borrow();
    let drawing = c.drawing().expect("a drawing is open");
    assert_eq!(
        drawing
            .model_space()
            .iter()
            .filter(|entity| matches!(&entity.geometry, SemanticGeometry::Table(_)))
            .count(),
        1
    );
}

#[test]
fn typed_tabledit_reaches_the_command_layer_and_refuses_without_a_selection() {
    let session = fresh_session();
    let error = session
        .command("TABLEDIT 1 1 hello world")
        .expect_err("TABLEDIT needs one selected table");
    assert!(
        error.contains("tabledit.no_selection"),
        "the typed line must dispatch to the real command: {error}"
    );
}
