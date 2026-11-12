//! The typed `REFSET ADD|REMOVE` / `REFCLOSE SAVE|DISCARD` lines, at the router
//! boundary.
//!
//! These pin the **textual grammar** and the funnel: the option is admitted by
//! its canonical selector, a line outside the grammar is refused with the stable
//! `refset.invalid_option` / `refclose.invalid_option` code before any dispatch,
//! and the command layer's own `refedit.*` refusals reach the same error channel.
//! `cad-app`'s `tests::refedit` pins the payload-level contracts (extraction,
//! save/discard, refusals, Viewer mode); the grammar lives here because `cad-app`
//! sits below this crate and must not own a second command-line grammar.

use super::super::super::*;
use super::super::test_support::*;
use cad_app::{RefCloseOption, RefSetOption};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn typed_refset_and_refclose_options_are_admitted_case_insensitively() {
    assert_eq!(
        typed_command_argument("REFSET REMOVE"),
        Some(("REFSET", "REMOVE"))
    );
    assert_eq!(
        typed_command_argument(" refset\tadd "),
        Some(("REFSET", "add"))
    );
    assert_eq!(
        typed_command_argument("REFCLOSE save"),
        Some(("REFCLOSE", "save"))
    );
    assert_eq!(
        typed_command_argument("refclose DISCARD"),
        Some(("REFCLOSE", "DISCARD"))
    );
    // The bare names are canonical name-only selectors, never an argument split.
    assert_eq!(typed_command_argument("REFSET"), None);
    assert_eq!(typed_command_argument("REFCLOSE"), None);
    assert_eq!(normalize_command("REFSET"), Some(CommandId::RefSet));
    assert_eq!(normalize_command("REFCLOSE"), Some(CommandId::RefClose));
    assert_eq!(normalize_command("REFEDIT"), Some(CommandId::RefEdit));
    assert_eq!(normalize_command("-REFEDIT"), Some(CommandId::DashRefEdit));

    assert_eq!(parse_refset_option(" ADD "), Some(RefSetOption::Add));
    assert_eq!(parse_refset_option("remove"), Some(RefSetOption::Remove));
    assert_eq!(parse_refset_option("adds"), None);
    assert_eq!(parse_refclose_option("SAVE"), Some(RefCloseOption::Save));
    assert_eq!(
        parse_refclose_option(" discard "),
        Some(RefCloseOption::Discard)
    );
    assert_eq!(parse_refclose_option("CANCEL"), None);
}

#[test]
fn an_option_outside_the_grammar_is_refused_by_name_and_changes_nothing() {
    let session = fresh_session();
    let (document, before) = {
        let c = session.core.controller.borrow();
        (c.document_id, c.drawing().unwrap())
    };

    let error = session.command("REFSET BOGUS").unwrap_err();
    assert!(error.contains("refset.invalid_option"), "{error}");
    let error = session.command("REFCLOSE CANCEL").unwrap_err();
    assert!(error.contains("refclose.invalid_option"), "{error}");

    let after = session.core.controller.borrow().drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    {
        let c = session.core.controller.borrow();
        assert!(!c.application.can_undo(&document));
        assert!(!c.application.can_redo(&document));
    }
}

#[test]
fn the_typed_options_reach_the_command_layer_refusals() {
    // A fresh session has no active in-place block edit and an empty selection:
    // the application's named refusals are the real completion errors.
    let session = fresh_session();
    let error = session.command("REFSET ADD").unwrap_err();
    assert!(error.contains("refedit.no_session"), "{error}");
    let error = session.command("REFCLOSE SAVE").unwrap_err();
    assert!(error.contains("refedit.no_session"), "{error}");
    let error = session.command("REFEDIT").unwrap_err();
    assert!(error.contains("refedit.selection"), "{error}");
    // A bare REFCLOSE keeps its own needs_option refusal.
    let error = session.command("REFCLOSE").unwrap_err();
    assert!(error.contains("refclose.needs_option"), "{error}");
}
