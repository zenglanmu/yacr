//! `SPLITSOLID` selector contract.
//!
//! Selection-based (no payload, no capture): the session admits the selector and
//! reaches the command layer, whose own refusal names the selection rule. The
//! split/commit contract lives in `cad-app`.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_splitsolid_selector_is_admitted_without_a_capture() {
    assert!(admissible_names().contains(&"SPLITSOLID"));
    assert_eq!(canonical_command_name("SPLITSOLID"), Some("SPLITSOLID"));
    assert_eq!(normalize_command("splitsolid"), Some(CommandId::SplitSolid));
    assert_eq!(command_name(CommandId::SplitSolid), Some("SPLITSOLID"));
    assert!(commands::interaction_request_for(CommandId::SplitSolid).is_none());
}

#[test]
fn the_splitsolid_line_reaches_the_command_layer_and_names_its_selection_rule() {
    let session = fresh_session();
    let error = session
        .command("SPLITSOLID")
        .expect_err("nothing is selected in a fresh session");
    assert!(
        error.contains("splitsolid.needs_two"),
        "the command's own refusal is named: {error}"
    );
}
