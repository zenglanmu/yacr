//! The typed `SPACE <argument>` line: layout-tab switching through the ordinary
//! command funnel.
//!
//! A tab click must not need a second IPC, so `SPACE MODEL` / `SPACE #2` /
//! `SPACE <display name>` all dispatch the same `SwitchSpace` payload the
//! panel path uses. These tests pin the resolution rules: exact name first, then
//! a **unique** case-insensitive name, and a named refusal for an unknown or
//! ambiguous layout — never a guessed id. A space switch is session state only,
//! so every admitted switch leaves the drawing revision and history untouched.

use super::super::super::*;
use super::super::test_support::*;

const LAYOUTS: &[(u128, &str, Option<&str>)] = &[
    (2, "Sheet One", None),
    (3, "Sheet Two", None),
    // Two layouts differing only by case: an exact spelling still resolves,
    // while a third casing is ambiguous and must be refused.
    (4, "Dup", None),
    (5, "dup", None),
];

fn active_space(session: &CadSession) -> CadActiveSpace {
    session.snapshot(None, None, 0).active_space
}

fn revision(session: &CadSession) -> cad_domain::Revision {
    session
        .core
        .controller
        .borrow()
        .drawing()
        .expect("the layout fixture drawing")
        .revision()
}

/// A space switch is session state only: no database revision and no history.
fn assert_no_write(session: &CadSession, before: cad_domain::Revision) {
    assert_eq!(
        revision(session),
        before,
        "a space switch writes no entity data"
    );
    let c = session.core.controller.borrow();
    assert!(
        !c.application.can_undo(&c.document_id),
        "a space switch records no undo step"
    );
    assert!(
        !c.application.can_redo(&c.document_id),
        "a space switch records no redo step"
    );
}

fn echoed_commands(session: &CadSession) -> Vec<String> {
    session
        .snapshot(None, None, 0)
        .command_log
        .iter()
        .filter_map(|entry| entry.command.clone())
        .collect()
}

#[test]
fn space_model_and_a_numeric_layout_id_dispatch_switch_space() {
    let session = session_with_layouts(LAYOUTS);
    let before = revision(&session);
    assert_eq!(active_space(&session), CadActiveSpace::Model);

    session.command("SPACE #2").expect("SPACE #2 is admitted");
    assert_eq!(active_space(&session), CadActiveSpace::Paper { id: 2 });
    assert_no_write(&session, before);

    session
        .command("SPACE model")
        .expect("MODEL is case-insensitive");
    assert_eq!(active_space(&session), CadActiveSpace::Model);
    assert_no_write(&session, before);

    // Only the admitted selector name is echoed, never the typed argument.
    assert_eq!(echoed_commands(&session), vec!["SPACE", "SPACE"]);
}

#[test]
fn a_layout_name_resolves_exactly_then_by_a_unique_case_insensitive_match() {
    let session = session_with_layouts(LAYOUTS);
    let before = revision(&session);

    // The whole remainder is the name, so spaces inside a layout name work.
    session
        .command("SPACE Sheet One")
        .expect("an exact display name is admitted");
    assert_eq!(active_space(&session), CadActiveSpace::Paper { id: 2 });
    assert_no_write(&session, before);

    session
        .command("SPACE sHeEt tWo")
        .expect("a unique case-insensitive name is admitted");
    assert_eq!(active_space(&session), CadActiveSpace::Paper { id: 3 });
    assert_no_write(&session, before);

    // Exact spelling wins over the case-insensitive ambiguity.
    session
        .command("SPACE Dup")
        .expect("an exact name is unambiguous even beside a case twin");
    assert_eq!(active_space(&session), CadActiveSpace::Paper { id: 4 });
    assert_no_write(&session, before);
}

#[test]
fn unknown_and_ambiguous_layout_names_are_refused_without_a_guess() {
    let session = session_with_layouts(LAYOUTS);
    let before = revision(&session);
    session
        .command("SPACE Sheet One")
        .expect("establish a non-model space first");
    let established = active_space(&session);
    assert_eq!(established, CadActiveSpace::Paper { id: 2 });

    let error = session
        .command("SPACE Nowhere")
        .expect_err("an unknown layout is refused");
    assert_eq!(error, "unknown layout: Nowhere");
    assert_eq!(
        session.snapshot(None, None, 0).error.as_deref(),
        Some("unknown layout: Nowhere"),
        "the refusal is reported, never silently dropped"
    );
    assert_eq!(
        active_space(&session),
        established,
        "the space is unchanged"
    );
    assert_no_write(&session, before);

    // No exact match and two case-insensitive matches: refuse by name rather
    // than dispatch the first one.
    let error = session
        .command("SPACE dUP")
        .expect_err("a case-twin is ambiguous");
    assert_eq!(error, "ambiguous layout: dUP");
    assert_eq!(
        active_space(&session),
        established,
        "the space is unchanged"
    );
    assert_no_write(&session, before);
}

#[test]
fn bare_space_keeps_the_existing_payload_refusal() {
    let session = session_with_layouts(LAYOUTS);
    let before = revision(&session);
    let error = session
        .command("SPACE")
        .expect_err("bare SPACE has no payload and is refused");
    assert!(
        error.contains("needs a space payload"),
        "the application's explicit refusal is kept: {error:?}"
    );
    assert_eq!(active_space(&session), CadActiveSpace::Model);
    assert_no_write(&session, before);
}
