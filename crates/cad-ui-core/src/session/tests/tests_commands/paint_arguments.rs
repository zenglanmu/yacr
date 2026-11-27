//! The `PAINT <material>` typed line: grammar and dispatch.
//!
//! The session does not resolve the name (the command owns the document /
//! built-in lookup), so these contracts pin the split, the bare-selector
//! refusal and the fact that the typed value reaches the command.

use super::super::super::*;
use super::super::test_support::*;
use crate::session::commands::parse_paint_argument;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn paint_has_one_name_grammar_and_the_rest_of_the_line_is_the_name() {
    assert_eq!(parse_paint_argument("Red"), Some("Red".into()));
    assert_eq!(parse_paint_argument("  Wood "), Some("Wood".into()));
    assert_eq!(
        parse_paint_argument("Light Wood 02"),
        Some("Light Wood 02".into())
    );
    assert_eq!(parse_paint_argument("   "), None);
    assert_eq!(typed_command_argument("PAINT Red"), Some(("PAINT", "Red")));
    assert_eq!(typed_command_argument("PAINT"), None);
    assert_eq!(
        command_name(CommandId::Extended(cad_app::ExtendedCommandId::Paint)),
        Some("PAINT")
    );
    assert!(CommandId::Extended(cad_app::ExtendedCommandId::Paint).requires_work_mode());
    assert_eq!(
        command_name(CommandId::Extended(cad_app::ExtendedCommandId::Materials)),
        Some("MATERIALS")
    );
    assert!(!CommandId::Extended(cad_app::ExtendedCommandId::Materials).requires_work_mode());
}

#[test]
fn the_typed_name_reaches_the_command_and_a_bare_paint_is_refused_by_name() {
    let session = fresh_session();
    // An empty selection refuses before any table write, which proves the typed
    // name was parsed and dispatched (the refusal is the command's own).
    let error = session.command("PAINT Red").unwrap_err();
    assert!(error.contains("paint.selection_required"), "{error}");
    let error = session.command("PAINT").unwrap_err();
    assert!(error.contains("paint.requires_material"), "{error}");
    // The report is read-only and names the palette.
    session
        .command("MATERIALS")
        .expect("MATERIALS is an admitted read-only report");
}
