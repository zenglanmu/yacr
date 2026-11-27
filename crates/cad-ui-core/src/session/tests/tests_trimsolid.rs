//! `TRIMSOLID` selector contract.
//!
//! The command is selection-based (no payload, no capture): the session admits
//! the selector and reaches the command layer, whose own refusal names the
//! selection rule. The geometry/commit contract lives in `cad-app`.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_trimsolid_selector_is_admitted_without_a_capture() {
    assert!(admissible_names().contains(&"TRIMSOLID"));
    assert_eq!(canonical_command_name("TRIMSOLID"), Some("TRIMSOLID"));
    assert_eq!(normalize_command("trimsolid"), Some(CommandId::TrimSolid));
    assert_eq!(command_name(CommandId::TrimSolid), Some("TRIMSOLID"));
    assert!(commands::interaction_request_for(CommandId::TrimSolid).is_none());
}

#[test]
fn the_trimsolid_line_reaches_the_command_layer_and_names_its_selection_rule() {
    let session = fresh_session();
    let error = session
        .command("TRIMSOLID")
        .expect_err("nothing is selected in a fresh session");
    assert!(
        error.contains("trimsolid.needs_two"),
        "the command's own refusal is named: {error}"
    );
}
