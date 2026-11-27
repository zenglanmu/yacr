//! `OUTERSHELL` selector contract.
//!
//! The command is a selection-based mesh write: it carries no payload and no
//! interactive capture, so the session only has to admit the selector and reach
//! the command layer. The geometry/refusal contract lives in `cad-app`.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_outershell_selector_is_admitted_without_a_capture() {
    assert!(admissible_names().contains(&"OUTERSHELL"));
    assert_eq!(canonical_command_name("OUTERSHELL"), Some("OUTERSHELL"));
    assert_eq!(normalize_command("outershell"), Some(CommandId::OuterShell));
    assert_eq!(command_name(CommandId::OuterShell), Some("OUTERSHELL"));
    // A selection-based command, never a draw/measurement capture.
    assert!(commands::interaction_request_for(CommandId::OuterShell).is_none());
}

#[test]
fn the_outershell_line_reaches_the_command_layer_and_names_its_selection_rule() {
    let session = fresh_session();
    let error = session
        .command("OUTERSHELL")
        .expect_err("nothing is selected in a fresh session");
    assert!(
        error.contains("outershell.needs_two"),
        "the command's own refusal is named: {error}"
    );
}
