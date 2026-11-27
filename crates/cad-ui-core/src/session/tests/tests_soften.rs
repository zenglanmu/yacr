//! `SOFTENEDGES` typed line: grammar and dispatch.
//!
//! The command re-validates the angle and owns every semantic refusal, so the
//! session only splits the optional argument off and dispatches.

use super::super::*;
use super::test_support::*;
use crate::session::commands::parse_softenedges_argument;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_softenedges_grammar_accepts_a_bare_selector_or_one_angle() {
    assert_eq!(parse_softenedges_argument(""), Some(None));
    assert_eq!(parse_softenedges_argument("   "), Some(None));
    assert_eq!(parse_softenedges_argument("30"), Some(Some(30.0)));
    assert_eq!(parse_softenedges_argument(" 12.5 "), Some(Some(12.5)));
    assert_eq!(parse_softenedges_argument("0"), Some(Some(0.0)));
    assert_eq!(parse_softenedges_argument("180"), Some(Some(180.0)));
    for refused in ["181", "-1", "abc", "30 40", "NaN", "inf"] {
        assert_eq!(parse_softenedges_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("SOFTENEDGES 30"),
        Some(("SOFTENEDGES", "30"))
    );
    assert_eq!(typed_command_argument("SOFTENEDGES"), None);
    assert_eq!(command_name(CommandId::SoftenEdges), Some("SOFTENEDGES"));
    assert!(CommandId::SoftenEdges.requires_work_mode());
}

#[test]
fn the_typed_lines_reach_the_command_and_an_empty_selection_is_named() {
    let session = fresh_session();
    // No selection: the command's own refusal proves the line dispatched.
    let error = session.command("SOFTENEDGES 30").unwrap_err();
    assert!(error.contains("soften.selection_required"), "{error}");
    let error = session.command("SOFTENEDGES").unwrap_err();
    assert!(error.contains("soften.selection_required"), "{error}");
    // An out-of-grammar angle is refused before the command.
    let error = session.command("SOFTENEDGES 200").unwrap_err();
    assert!(
        error.contains("SOFTENEDGES takes at most one angle"),
        "{error}"
    );
}
