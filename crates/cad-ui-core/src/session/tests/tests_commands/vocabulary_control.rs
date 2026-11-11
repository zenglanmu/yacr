//! The session-only vocabulary controls: `UNDEFINE`, `REDEFINE`, `ALIASEDIT`.
//!
//! These pin the resolved contract of the three real commands:
//!
//! * a disabled name behaves as an unknown command (`command.undefined`), never
//!   dispatching — no capture, no transaction, no revision, no undo entry;
//! * `REDEFINE` restores exactly the disabled name;
//! * a runtime alias shadows a built-in selector and can be removed; the shipped
//!   `COMMAND_ALIASES` rows cannot;
//! * the essential commands refuse `redefine.essential` in both directions;
//! * every malformed line is a named refusal that changes nothing.

use super::super::super::*;
use super::super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn drawing_revision(session: &CadSession) -> cad_domain::Revision {
    session
        .core
        .controller
        .borrow()
        .drawing()
        .unwrap()
        .revision()
}

fn assert_no_history(session: &CadSession) {
    let c = session.core.controller.borrow();
    let document = c.document_id;
    assert!(!c.application.can_undo(&document), "no undo entry");
    assert!(!c.application.can_redo(&document), "no redo entry");
}

#[test]
fn undefine_disables_a_builtin_and_dispatch_refuses_with_command_undefined() {
    let session = fresh_session();
    let (document, before) = {
        let c = session.core.controller.borrow();
        (c.document_id, c.drawing().unwrap())
    };

    session
        .command("UNDEFINE CIRCLE")
        .expect("UNDEFINE CIRCLE is admitted");
    assert_eq!(session.undefined_commands(), vec!["CIRCLE"]);

    // The disabled name is not dispatched: no draw capture starts.
    let error = session.command("CIRCLE").unwrap_err();
    assert_eq!(error, "command.undefined: CIRCLE");
    assert_eq!(
        session.snapshot(None, None, 0).error.as_deref(),
        Some("command.undefined: CIRCLE"),
        "the refusal is recorded for cad_status.error"
    );
    assert_eq!(session.snapshot(None, None, 0).tool, None);

    // Case-insensitive, and the shipped alias is disabled with its target.
    assert_eq!(
        session.command("circle").unwrap_err(),
        "command.undefined: CIRCLE"
    );
    assert_eq!(session.command("C").unwrap_err(), "command.undefined: C");

    // Session-only: the drawing, its revision and its history are untouched.
    let after = session.core.controller.borrow().drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.entity_count(), before.entity_count());
    drop(after);
    assert_no_history(&session);
    assert_eq!(session.core.controller.borrow().document_id, document);
}

#[test]
fn redefine_restores_a_disabled_builtin() {
    let session = fresh_session();
    session.command("UNDEFINE FIT").expect("admitted");
    assert!(session.command("FIT").is_err());

    session
        .command("REDEFINE FIT")
        .expect("re-enabling is admitted");
    assert!(session.undefined_commands().is_empty());
    session.command("FIT").expect("FIT dispatches again");
    // The re-enabled selector only changes the view; no drawing/history write.
    assert_no_history(&session);
}

#[test]
fn undefining_a_typed_argument_base_selector_disables_its_argument_form() {
    let session = fresh_session();
    session.command("UNDEFINE ZOOM").expect("admitted");
    assert_eq!(
        session.command("ZOOM 2").unwrap_err(),
        "command.undefined: ZOOM"
    );
    // The bare name-only spelling too.
    assert_eq!(
        session.command("ZOOM").unwrap_err(),
        "command.undefined: ZOOM"
    );
    session.command("REDEFINE ZOOM").expect("admitted");
    session
        .command("ZOOM 2")
        .expect("the argument form dispatches again");
}

#[test]
fn undefining_a_shell_setting_disables_its_argument_form() {
    let session = fresh_session();
    session.command("UNDEFINE NAVBAR").expect("admitted");
    assert_eq!(
        session.command("NAVBAR ON").unwrap_err(),
        "command.undefined: NAVBAR"
    );
    session.command("REDEFINE NAVBAR").expect("admitted");
    session
        .command("NAVBAR ON")
        .expect("the settings write dispatches again");
}

#[test]
fn essential_commands_refuse_redefine_essential_in_both_directions() {
    let session = fresh_session();
    for name in [
        "OPEN",
        "SAVE",
        "CANCEL",
        "REDEFINE",
        "UNDEFINE",
        "ALIASEDIT",
    ] {
        let undefined = session.command(&format!("UNDEFINE {name}")).unwrap_err();
        assert!(
            undefined.starts_with(&format!("redefine.essential: {name}")),
            "{undefined}"
        );
        let redefined = session.command(&format!("REDEFINE {name}")).unwrap_err();
        assert!(
            redefined.starts_with(&format!("redefine.essential: {name}")),
            "{redefined}"
        );
    }
    // Nothing was disabled by the refusals.
    assert!(session.undefined_commands().is_empty());
}

#[test]
fn unknown_and_redundant_names_refuse_by_name() {
    let session = fresh_session();
    assert_eq!(
        session.command("UNDEFINE NOSUCHCOMMAND").unwrap_err(),
        "redefine.unknown: NOSUCHCOMMAND"
    );
    assert_eq!(
        session.command("REDEFINE NOSUCHCOMMAND").unwrap_err(),
        "redefine.unknown: NOSUCHCOMMAND"
    );
    assert!(session
        .command("REDEFINE FIT")
        .unwrap_err()
        .starts_with("redefine.not_undefined: FIT"));
    session.command("UNDEFINE FIT").expect("admitted");
    assert!(session
        .command("UNDEFINE FIT")
        .unwrap_err()
        .starts_with("redefine.already_undefined: FIT"));
    // The argument resolves through the static tables: an alias spelling names
    // its target.
    session
        .command("UNDEFINE L")
        .expect("L canonicalises to LINE");
    assert_eq!(session.undefined_commands(), vec!["FIT", "LINE"]);
    assert_eq!(
        session.command("UNDEFINE").unwrap_err(),
        "redefine.missing_name: UNDEFINE needs one built-in command name"
    );
    assert_eq!(
        session.command("REDEFINE").unwrap_err(),
        "redefine.missing_name: REDEFINE needs one built-in command name"
    );
}

#[test]
fn aliasedit_add_shadows_a_builtin_and_remove_restores_it() {
    let session = fresh_session();
    session
        .command("ALIASEDIT ADD LINE CIRCLE")
        .expect("an alias may shadow a built-in selector");
    assert_eq!(
        session.runtime_command_aliases(),
        vec![("LINE".to_string(), "CIRCLE".to_string())]
    );

    // The alias shadows `LINE`: the same typed name now starts CIRCLE.
    session.command("LINE").expect("the alias dispatches");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("a capture started");
    assert_eq!(tool.key, "draw.circle");
    session.command("CANCEL").expect("cancel the capture");

    session
        .command("ALIASEDIT REMOVE LINE")
        .expect("removal is admitted");
    assert!(session.runtime_command_aliases().is_empty());
    session.command("LINE").expect("the built-in is back");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("a capture started");
    assert_eq!(tool.key, "draw.line");
    assert_no_history(&session);
}

#[test]
fn aliasedit_list_and_bare_aliasedit_change_nothing() {
    let session = fresh_session();
    let before = drawing_revision(&session);
    session
        .command("ALIASEDIT")
        .expect("bare ALIASEDIT is LIST");
    session
        .command("ALIASEDIT LIST")
        .expect("explicit LIST is admitted");
    let after = session.snapshot(None, None, 0);
    assert_eq!(drawing_revision(&session), before);
    assert_eq!(after.error, None);
    assert!(session.runtime_command_aliases().is_empty());
}

#[test]
fn alias_to_a_disabled_target_is_disabled_too() {
    let session = fresh_session();
    session.command("ALIASEDIT ADD QQ FIT").expect("admitted");
    session.command("UNDEFINE FIT").expect("admitted");
    assert_eq!(session.command("QQ").unwrap_err(), "command.undefined: QQ");
    session.command("REDEFINE FIT").expect("admitted");
    session.command("QQ").expect("the alias works again");
}

#[test]
fn a_runtime_alias_carries_the_typed_argument_of_its_target() {
    let session = fresh_session();
    session.command("ALIASEDIT ADD Z2 ZOOM").expect("admitted");
    // The base name is a runtime alias; the argument survives the substitution.
    session.command("Z2 2").expect("Z2 2 resolves to ZOOM 2");
}

#[test]
fn aliasedit_refuses_invalid_duplicate_unknown_and_builtin_rows() {
    let session = fresh_session();
    assert!(session
        .command("ALIASEDIT ADD A@B FIT")
        .unwrap_err()
        .starts_with("aliasedit.invalid_name: A@B"));
    assert!(session
        .command("ALIASEDIT ADD QQ NOSUCHCOMMAND")
        .unwrap_err()
        .starts_with("aliasedit.unknown_target: NOSUCHCOMMAND"));
    session.command("ALIASEDIT ADD QQ FIT").expect("admitted");
    assert_eq!(
        session.command("ALIASEDIT ADD QQ ZOOM").unwrap_err(),
        "aliasedit.duplicate: QQ"
    );
    // Shipped `acad.pgp` rows are the build's, not the session's.
    assert_eq!(
        session.command("ALIASEDIT ADD L FIT").unwrap_err(),
        "aliasedit.duplicate: L"
    );
    assert_eq!(
        session.command("ALIASEDIT REMOVE L").unwrap_err(),
        "aliasedit.builtin: L"
    );
    assert_eq!(
        session.command("ALIASEDIT REMOVE NOPE").unwrap_err(),
        "aliasedit.unknown: NOPE"
    );
    // The essential commands cannot be shadowed either.
    assert_eq!(
        session.command("ALIASEDIT ADD OPEN FIT").unwrap_err(),
        "aliasedit.essential: OPEN"
    );
}

#[test]
fn malformed_control_lines_are_named_refusals_that_change_nothing() {
    let session = fresh_session();
    let before = drawing_revision(&session);
    for (line, code) in [
        ("ALIASEDIT ADD", "aliasedit.malformed:"),
        ("ALIASEDIT ADD QQ", "aliasedit.malformed:"),
        ("ALIASEDIT REMOVE A B", "aliasedit.malformed:"),
        ("ALIASEDIT LIST EXTRA", "aliasedit.malformed:"),
        ("ALIASEDIT FLURB", "aliasedit.malformed:"),
        ("UNDEFINE NOSUCH EXTRA", "redefine.unknown:"),
    ] {
        let error = session.command(line).unwrap_err();
        assert!(
            error.starts_with(code),
            "{line}: expected {code}, got {error}"
        );
    }
    assert_eq!(drawing_revision(&session), before);
    assert!(session.runtime_command_aliases().is_empty());
    assert!(session.undefined_commands().is_empty());
}

#[test]
fn the_shell_gate_admits_runtime_aliases_and_keeps_static_unknown_refusal() {
    let mut state = crate::shell::ShellState::new(true);
    // Without the owner-tick mirror the gate is the static one.
    assert!(crate::shell::admit_command(&mut state, "QQ").is_err());
    assert_eq!(state.cad_commands.len(), 0);

    state.command_aliases = vec![("QQ".to_string(), "FIT".to_string())];
    crate::shell::admit_command(&mut state, "qq").expect("a runtime alias is admitted");
    assert_eq!(state.cad_commands, vec!["FIT".to_string()]);

    state
        .command_aliases
        .push(("Z2".to_string(), "ZOOM".to_string()));
    crate::shell::admit_command(&mut state, "z2 2").expect("the alias argument is admitted");
    assert_eq!(
        state.cad_commands.last().map(String::as_str),
        Some("ZOOM 2")
    );

    // A name-only target cannot be given an argument, and unknown names stay
    // refused before the queue.
    assert!(crate::shell::admit_command(&mut state, "qq 2").is_err());
    assert!(crate::shell::admit_command(&mut state, "BOGUS").is_err());
}

#[test]
fn control_selectors_are_static_admitted_and_carry_no_command_id() {
    for selector in [UNDEFINE_SELECTOR, REDEFINE_SELECTOR, ALIASEDIT_SELECTOR] {
        assert_eq!(canonical_command_name(selector), Some(selector));
        assert!(admissible_names().contains(&selector), "{selector}");
        assert!(ARGUMENT_SELECTORS.contains(&selector), "{selector}");
    }
    // `UNDEFINE`/`REDEFINE` are session-only: no `CommandId`. `ALIASEDIT` keeps
    // its existing (read-only) extended selector.
    assert_eq!(normalize_command(UNDEFINE_SELECTOR), None);
    assert_eq!(normalize_command(REDEFINE_SELECTOR), None);
    assert_eq!(
        normalize_command(ALIASEDIT_SELECTOR),
        Some(CommandId::Extended(cad_app::ExtendedCommandId::AliasEdit))
    );
    // Essential commands are the documented set.
    assert_eq!(
        ESSENTIAL_COMMANDS,
        &[
            "ALIASEDIT",
            "CANCEL",
            "OPEN",
            "REDEFINE",
            "SAVE",
            "UNDEFINE"
        ]
    );
}
