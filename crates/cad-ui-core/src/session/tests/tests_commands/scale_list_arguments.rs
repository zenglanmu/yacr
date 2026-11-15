//! The typed `SCALELISTEDIT` / `-SCALELISTEDIT` command lines.
//!
//! Pinned here: the one-line grammar (`?`, `ADD`, `RENAME`, `DELETE`, the named
//! `RESET` refusal), selector admission (both spellings are argument forms of
//! their existing selectors, not new commands), and the dispatch each form takes.
//! The table semantics (one revision, the delete refusal, the rename cascade) are
//! pinned in `cad-app`'s `extended_w10` tests; nothing about them is re-verified
//! here beyond the payload the dispatch carries.

use super::super::super::*;
use super::super::test_support::*;

use crate::session::commands::{
    ARGUMENT_SELECTORS, COMMAND_TABLE, DASH_SCALELIST_SELECTOR, SCALELIST_SELECTOR,
};
use crate::session::scale_arguments::{
    invalid_scale_list_argument_reason, parse_scale_list_argument, ScaleListAction,
};
use cad_app::{ExtendedCommandId, ScaleListCommand};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn add_parses_one_name_and_two_ratio_components() {
    assert_eq!(
        parse_scale_list_argument("ADD 1:200 1 200"),
        Some(ScaleListAction::Write(ScaleListCommand::Add {
            name: "1:200".into(),
            paper_units: 1.0,
            drawing_units: 200.0,
        }))
    );
    // Keyword case, surrounding whitespace and a quoted name with spaces.
    assert_eq!(
        parse_scale_list_argument("  add  \"my scale\"  1  12 "),
        Some(ScaleListAction::Write(ScaleListCommand::Add {
            name: "my scale".into(),
            paper_units: 1.0,
            drawing_units: 12.0,
        }))
    );
    // A name containing a double quote (AutoCAD's inch spelling) cannot be
    // spelled here: the shared tokenizer treats `"` as the quote delimiter and
    // has no escape. The **table** handles such a row (import, rename cascade,
    // delete refusal, export); only the typed grammar cannot express it, which
    // the module documents as a named boundary rather than silently dropping the
    // quote.
    assert_eq!(parse_scale_list_argument("ADD 1\" = 1'-0\" 1 12"), None);
    // A fractional component is a real ratio (`1:2.5`).
    assert_eq!(
        parse_scale_list_argument("ADD half 0.5 2"),
        Some(ScaleListAction::Write(ScaleListCommand::Add {
            name: "half".into(),
            paper_units: 0.5,
            drawing_units: 2.0,
        }))
    );
    // A zero or negative component is **not** rejected here: `cad-db` owns that
    // rule and refuses it by name (`scalelistedit.invalid_ratio`), so the typed
    // line must reach it instead of returning a generic argument error.
    assert!(matches!(
        parse_scale_list_argument("ADD 1:0 1 0"),
        Some(ScaleListAction::Write(ScaleListCommand::Add { .. }))
    ));
}

#[test]
fn rename_delete_report_and_reset_have_their_own_shapes() {
    assert_eq!(
        parse_scale_list_argument("RENAME 1:100 1:150"),
        Some(ScaleListAction::Write(ScaleListCommand::Rename {
            old: "1:100".into(),
            new: "1:150".into(),
        }))
    );
    assert_eq!(
        parse_scale_list_argument("DELETE 1:100"),
        Some(ScaleListAction::Write(ScaleListCommand::Delete {
            name: "1:100".into(),
        }))
    );
    assert_eq!(
        parse_scale_list_argument("?"),
        Some(ScaleListAction::Report)
    );
    assert_eq!(
        parse_scale_list_argument("RESET"),
        Some(ScaleListAction::Reset)
    );
}

#[test]
fn every_malformed_line_is_refused_without_defaulting() {
    for argument in [
        "",
        "ADD",
        "ADD 1:200",
        "ADD 1:200 1",
        "ADD 1:200 1 200 EXTRA",
        "ADD 1:200 one 200",
        "RENAME",
        "RENAME onlyone",
        "RENAME a b c",
        "DELETE",
        "DELETE a b",
        "RESET now",
        "? filter",
        "LIST",
        "ADDITION 1 1",
    ] {
        assert_eq!(
            parse_scale_list_argument(argument),
            None,
            "'{argument}' must be refused, not defaulted"
        );
    }
    assert!(invalid_scale_list_argument_reason("LIST").contains("ADD <name> <paper_units>"));
}

#[test]
fn both_selectors_are_admitted_argument_forms() {
    assert!(ARGUMENT_SELECTORS.contains(&SCALELIST_SELECTOR));
    assert!(ARGUMENT_SELECTORS.contains(&DASH_SCALELIST_SELECTOR));
    // They are argument forms of their own existing commands, not new ids.
    let id_of = |selector: &str| {
        COMMAND_TABLE
            .iter()
            .find(|(name, _)| *name == selector)
            .map(|(_, id)| *id)
    };
    assert_eq!(
        id_of(SCALELIST_SELECTOR),
        Some(cad_app::CommandId::Extended(
            ExtendedCommandId::ScaleListEdit
        ))
    );
    assert_eq!(
        id_of(DASH_SCALELIST_SELECTOR),
        Some(cad_app::CommandId::Extended(
            ExtendedCommandId::DashScaleListEdit
        ))
    );
}

#[test]
fn a_typed_add_reaches_the_scale_list_payload() {
    // The bare selector keeps the name-only payload refusal; a typed line
    // dispatches the one handler with the parsed operation.
    let session = fresh_session();
    session
        .command("-SCALELISTEDIT ADD 1:200 1 200")
        .expect("the typed line is admitted");
    let echoed: Vec<String> = session
        .snapshot(None, None, 0)
        .command_log
        .iter()
        .filter_map(|entry| entry.command.clone())
        .collect();
    assert!(
        echoed.iter().any(|name| name == "-SCALELISTEDIT"),
        "the typed line echoes its canonical selector: {echoed:?}"
    );

    // `RESET` is refused by name before any dispatch, and `?` is the report.
    let reset = session
        .command("-SCALELISTEDIT RESET")
        .expect_err("RESET is refused by name");
    assert!(reset.contains("scalelistedit.reset_unsupported"), "{reset}");
    session
        .command("-SCALELISTEDIT ?")
        .expect("the report spelling is admitted");
    let malformed = session
        .command("-SCALELISTEDIT DELETE")
        .expect_err("a missing name is refused");
    assert!(
        malformed.contains("invalid SCALELISTEDIT argument"),
        "{malformed}"
    );
}
