//! The typed command lines of four commands whose payloads already existed but
//! that had **no** reachable argument form: `FSMODE`, `ELEV`, `ALIGNSPACE` and
//! `MSTRETCH`.
//!
//! Each block pins the same three facts:
//!
//! 1. the selector is admitted as an argument selector and `typed_command_argument`
//!    splits it (so the shell gate queues the line);
//! 2. the parser produces the **exact** payload the handler consumes — the typed
//!    line is not a second, parallel path;
//! 3. the line really reaches the handler through `CadSession::command`: a session
//!    token changes, or the handler's own named refusal comes back (never the
//!    generic `unsupported CAD command` sentence).
//!
//! Grammar refusals are pinned by their stable codes (`fsmode.invalid_value`,
//! `elev.argument`, `alignspace.argument`, `mstretch.argument_odd`, …) and every
//! one of them changes nothing.

use super::super::super::*;
use super::super::test_support::*;
use cad_app::{CommandPayload, ExtendedCommandId};

use crate::session::commands::{
    parse_alignspace_argument, parse_elev_argument, parse_fsmode_argument, parse_mstretch_argument,
    parse_vpsync_argument, ALIGNSPACE_SELECTOR, ARGUMENT_SELECTORS, ELEV_SELECTOR, FSMODE_SELECTOR,
    MSTRETCH_SELECTOR, VPSYNC_SELECTOR,
};

/// The refusal text of a parser that must not accept `what` (the payload type has
/// no `Debug`, so `expect_err` cannot be used).
fn parse_refusal<T>(result: Result<T, String>, what: &str) -> String {
    match result {
        Ok(_) => panic!("{what} must be refused"),
        Err(reason) => reason,
    }
}

fn fresh() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn revision(session: &CadSession) -> u64 {
    let controller = session.core.controller.borrow();
    controller
        .drawing()
        .expect("the demo document")
        .revision()
        .0
}

fn undo_available(session: &CadSession) -> bool {
    let controller = session.core.controller.borrow();
    controller.application.can_undo(&controller.document_id)
}

// ---------------------------------------------------------------------------
// FSMODE
// ---------------------------------------------------------------------------

#[test]
fn fsmode_is_admitted_and_parses_on_off() {
    assert!(ARGUMENT_SELECTORS.contains(&FSMODE_SELECTOR));
    assert_eq!(
        typed_command_argument("FSMODE ON"),
        Some((FSMODE_SELECTOR, "ON"))
    );
    assert_eq!(
        normalize_command("FSMODE"),
        Some(CommandId::Extended(ExtendedCommandId::FsMode))
    );
    for (argument, expected) in [("ON", true), ("off", false), ("1", true), ("0", false)] {
        match parse_fsmode_argument(argument) {
            Ok(CommandPayload::FsMode(enabled)) => assert_eq!(enabled, expected, "{argument}"),
            _ => panic!("{argument} must parse to FsMode({expected}), got another payload"),
        }
    }
}

#[test]
fn fsmode_refuses_anything_outside_its_grammar_with_nothing_changed() {
    for argument in ["", "MAYBE", "ON OFF", "yes", "2"] {
        let error = parse_refusal(parse_fsmode_argument(argument), argument);
        assert!(
            error.starts_with("fsmode.invalid_value"),
            "{argument}: {error}"
        );
    }
    let session = fresh();
    let revision_before = revision(&session);
    let undo_before = undo_available(&session);
    let error = session.command("FSMODE MAYBE").expect_err("refused");
    assert!(error.starts_with("fsmode.invalid_value"), "{error}");
    assert_eq!(revision(&session), revision_before, "nothing was written");
    assert_eq!(undo_available(&session), undo_before, "no undo step");
}

#[test]
fn fsmode_sets_the_token_fs_reads_and_the_bare_selector_reports_it() {
    let session = fresh();
    let revision_before = revision(&session);
    assert!(
        !session
            .core
            .controller
            .borrow()
            .session
            .drafting_extras
            .fs_mode(),
        "OFF is the default"
    );

    session.command("FSMODE ON").expect("FSMODE ON is admitted");
    assert!(
        session
            .core
            .controller
            .borrow()
            .session
            .drafting_extras
            .fs_mode(),
        "the token FS reads is set"
    );
    session
        .command("FSMODE off")
        .expect("FSMODE off is admitted");
    assert!(!session
        .core
        .controller
        .borrow()
        .session
        .drafting_extras
        .fs_mode());

    // The bare selector is the report: it changes nothing and records no error.
    session.command("FSMODE").expect("the bare FSMODE reports");
    assert!(!session
        .core
        .controller
        .borrow()
        .session
        .drafting_extras
        .fs_mode());
    assert_eq!(
        revision(&session),
        revision_before,
        "FSMODE is session state only"
    );
    assert!(!undo_available(&session), "FSMODE records no undo step");
}

#[test]
fn setvar_fsmode_reads_and_writes_the_same_token() {
    let session = fresh();
    let revision_before = revision(&session);
    session
        .command("SETVAR FSMODE ON")
        .expect("SETVAR FSMODE ON writes the token");
    assert!(session
        .core
        .controller
        .borrow()
        .session
        .drafting_extras
        .fs_mode());
    // The read form goes through the modelled-variable reader with no error.
    session
        .command("SETVAR FSMODE")
        .expect("SETVAR FSMODE reads");
    assert_eq!(
        revision(&session),
        revision_before,
        "SETVAR FSMODE writes no drawing"
    );
    // `FSMODE ON` again is the named no-op, never a toggle.
    session.command("FSMODE ON").expect("already ON is a no-op");
    assert!(session
        .core
        .controller
        .borrow()
        .session
        .drafting_extras
        .fs_mode());
    let error = session.command("SETVAR FSMODE MAYBE").expect_err("refused");
    assert!(error.starts_with("fsmode.invalid_value"), "{error}");
}

// ---------------------------------------------------------------------------
// ELEV
// ---------------------------------------------------------------------------

#[test]
fn elev_is_admitted_and_parses_one_finite_height() {
    assert!(ARGUMENT_SELECTORS.contains(&ELEV_SELECTOR));
    assert_eq!(
        typed_command_argument("ELEV 2.5"),
        Some((ELEV_SELECTOR, "2.5"))
    );
    for (argument, expected) in [("2.5", 2.5), ("-3", -3.0), ("0", 0.0)] {
        match parse_elev_argument(argument) {
            Ok(CommandPayload::Elevation(height)) => assert_eq!(height, expected, "{argument}"),
            _ => panic!("{argument} must parse to Elevation({expected}), got another payload"),
        }
    }
}

#[test]
fn elev_refuses_a_bad_height_with_nothing_changed() {
    for argument in ["", "abc", "1 2", "NaN", "1e3"] {
        let error = parse_refusal(parse_elev_argument(argument), argument);
        assert!(error.starts_with("elev.argument"), "{argument}: {error}");
    }
    let session = fresh();
    let revision_before = revision(&session);
    let error = session.command("ELEV abc").expect_err("refused");
    assert!(error.starts_with("elev.argument"), "{error}");
    assert_eq!(revision(&session), revision_before);
    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .session
            .drafting_extras
            .elev(),
        0.0,
        "the token keeps its default"
    );
}

#[test]
fn elev_sets_the_token_the_create_funnel_reads_and_setvar_agrees() {
    let session = fresh();
    let revision_before = revision(&session);
    let undo_before = undo_available(&session);
    session
        .command("ELEV 2.5")
        .expect("ELEV <height> is admitted");
    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .session
            .drafting_extras
            .elev(),
        2.5
    );
    // `SETVAR ELEV` is the same token: the write goes through the same command.
    session
        .command("SETVAR ELEV -1.5")
        .expect("SETVAR ELEV writes");
    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .session
            .drafting_extras
            .elev(),
        -1.5
    );
    session.command("SETVAR ELEV").expect("SETVAR ELEV reads");
    // The bare selector reports without changing the token.
    session.command("ELEV").expect("the bare ELEV reports");
    assert_eq!(
        revision(&session),
        revision_before,
        "ELEV is session state only"
    );
    assert_eq!(undo_available(&session), undo_before, "no undo step");
}

// ---------------------------------------------------------------------------
// ALIGNSPACE
// ---------------------------------------------------------------------------

#[test]
fn alignspace_is_admitted_and_parses_the_two_point_pairs() {
    assert!(ARGUMENT_SELECTORS.contains(&ALIGNSPACE_SELECTOR));
    assert_eq!(
        typed_command_argument("ALIGNSPACE 1 2 3 4 5 6 7 8"),
        Some((ALIGNSPACE_SELECTOR, "1 2 3 4 5 6 7 8"))
    );
    let points =
        parse_alignspace_argument("1000 500 10 20 1100 500 30 20").expect("eight decimals");
    assert_eq!(points.len(), 4, "two model<->paper pairs");
    assert_eq!(
        (points[0].x, points[0].y, points[0].z),
        (1000.0, 500.0, 0.0)
    );
    assert_eq!((points[1].x, points[1].y), (10.0, 20.0));
    assert_eq!((points[2].x, points[2].y), (1100.0, 500.0));
    assert_eq!((points[3].x, points[3].y), (30.0, 20.0));
}

#[test]
fn alignspace_refuses_any_other_arity_or_a_non_number_with_nothing_changed() {
    for argument in [
        "",
        "1 2 3 4",
        // four points would be twelve decimals: the grammar is the four 2D points.
        "1 2 3 4 5 6 7 8 9 10 11 12",
        "1 2 3 4 5 6 7 x",
    ] {
        let error = parse_refusal(parse_alignspace_argument(argument), argument);
        assert!(
            error.starts_with("alignspace.argument"),
            "{argument}: {error}"
        );
    }
    let session = fresh();
    let revision_before = revision(&session);
    let error = session.command("ALIGNSPACE 1 2 3 4").expect_err("refused");
    assert!(error.starts_with("alignspace.argument"), "{error}");
    assert_eq!(revision(&session), revision_before, "nothing was written");
    assert!(!undo_available(&session), "no undo step");
}

#[test]
fn the_alignspace_line_reaches_the_handler_and_its_own_refusal() {
    // Model space has no viewport to align, so the handler answers with its own
    // stable reason — proof that the typed line is not refused as an unknown
    // command and that the payload reached `align_space`.
    let session = fresh();
    let revision_before = revision(&session);
    let error = session
        .command("ALIGNSPACE 1000 500 10 20 1100 500 30 20")
        .expect_err("model space cannot be aligned");
    assert!(
        error.contains("alignspace.paper_space_required")
            || error.contains("alignspace.no_viewport"),
        "{error}"
    );
    assert_eq!(revision(&session), revision_before, "nothing was written");
    assert!(!undo_available(&session), "no undo step");
}

// ---------------------------------------------------------------------------
// VPSYNC
// ---------------------------------------------------------------------------

#[test]
fn vpsync_is_admitted_and_parses_a_zero_based_master_index() {
    assert!(ARGUMENT_SELECTORS.contains(&VPSYNC_SELECTOR));
    assert_eq!(
        typed_command_argument("VPSYNC 2"),
        Some((VPSYNC_SELECTOR, "2"))
    );
    assert_eq!(parse_vpsync_argument("0"), Ok(0));
    assert_eq!(parse_vpsync_argument("2"), Ok(2));
}

#[test]
fn vpsync_refuses_any_other_arity_or_a_non_integer_with_nothing_changed() {
    for argument in ["", "1 2", "-1", "1.5", "x", "4294967296"] {
        let error = parse_refusal(parse_vpsync_argument(argument), argument);
        assert!(error.starts_with("vpsync.argument"), "{argument}: {error}");
        assert!(error.contains("nothing was changed"), "{argument}: {error}");
    }
    let session = fresh();
    let revision_before = revision(&session);
    let error = session.command("VPSYNC x").expect_err("refused");
    assert!(error.starts_with("vpsync.argument"), "{error}");
    assert_eq!(revision(&session), revision_before, "nothing was written");
    assert!(!undo_available(&session), "no undo step");
}

#[test]
fn the_vpsync_line_reaches_the_handler_and_its_own_refusal() {
    // Model space is not a paper layout: the handler's own reason comes back,
    // proving the typed line reached `vp_sync` with its master index.
    let session = fresh();
    let revision_before = revision(&session);
    let error = session
        .command("VPSYNC 3")
        .expect_err("model space cannot be synchronized");
    assert!(error.contains("vpsync.paper_space_required"), "{error}");
    assert_eq!(revision(&session), revision_before, "nothing was written");
    assert!(!undo_available(&session), "no undo step");
}

// ---------------------------------------------------------------------------
// MSTRETCH
// ---------------------------------------------------------------------------

#[test]
fn mstretch_is_admitted_and_parses_base_delta_and_windows() {
    assert!(ARGUMENT_SELECTORS.contains(&MSTRETCH_SELECTOR));
    assert_eq!(
        typed_command_argument("MSTRETCH 0 0 0 5 -1 -1 1 1"),
        Some((MSTRETCH_SELECTOR, "0 0 0 5 -1 -1 1 1"))
    );
    let Some(CommandPayload::MStretch {
        rects_logical,
        base,
        delta,
    }) = parse_mstretch_argument("0 0 0 5 -1 -1 1 1 9 -1 11 1").ok()
    else {
        panic!("a base point, a delta and two windows must parse");
    };
    assert_eq!((base.x, base.y, base.z), (0.0, 0.0, 0.0));
    assert_eq!((delta.x, delta.y, delta.z), (0.0, 5.0, 0.0));
    assert_eq!(
        rects_logical,
        vec![[[-1.0, -1.0], [1.0, 1.0]], [[9.0, -1.0], [11.0, 1.0]]]
    );
}

#[test]
fn mstretch_refuses_an_odd_token_count_and_a_missing_window() {
    let error = parse_refusal(parse_mstretch_argument("0 0 0 5 -1 -1 1"), "odd window");
    assert!(error.starts_with("mstretch.argument_odd"), "{error}");
    for argument in ["", "0 0 0", "0 0 0 5", "0 0 0 5 1 2 3 4 5"] {
        let error = parse_refusal(parse_mstretch_argument(argument), argument);
        assert!(
            error.starts_with("mstretch.argument") || error.starts_with("mstretch.argument_odd"),
            "{argument}: {error}"
        );
    }
    let error = parse_refusal(parse_mstretch_argument("0 0 0 5 a 1 2 2"), "non-number");
    assert!(error.starts_with("mstretch.invalid_number"), "{error}");

    let session = fresh();
    let revision_before = revision(&session);
    let error = session
        .command("MSTRETCH 0 0 0 5 1 2 3")
        .expect_err("refused");
    assert!(error.starts_with("mstretch.argument_odd"), "{error}");
    assert_eq!(revision(&session), revision_before, "nothing was written");
    assert!(!undo_available(&session), "no undo step");
}

#[test]
fn the_mstretch_line_reaches_the_planner_with_the_typed_payload() {
    // A zero displacement is the planner's own named refusal, so the line reached
    // `mstretch_entities` with a real `MStretch` payload (not the unsupported
    // command path) and wrote nothing.
    let session = fresh();
    let revision_before = revision(&session);
    let error = session
        .command("MSTRETCH 1 1 1 1 -1 -1 1 1")
        .expect_err("zero displacement");
    assert!(error.contains("mstretch.zero_displacement"), "{error}");
    assert_eq!(revision(&session), revision_before, "nothing was written");
    assert!(!undo_available(&session), "no undo step");
}
