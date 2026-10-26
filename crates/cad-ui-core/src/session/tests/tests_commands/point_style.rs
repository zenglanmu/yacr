//! The typed `PDMODE <n>` / `PDSIZE <n>` lines: the drawing POINT display style
//! sysvars, at the router boundary.
//!
//! These pin the **textual grammar** and the funnel: a valid line reaches the
//! drawing header through the ordinary command path (one revision bump, no undo
//! entry), a line outside the grammar is refused by name and changes nothing, and
//! a bare name keeps the explicit payload refusal. `cad-app`'s
//! `tests::point_display_style` pins the payload-level contracts (stored values,
//! revision, history, Viewer refusal); the grammar lives here because `cad-app`
//! sits below this crate and must not own a second command-line grammar.

use super::super::super::*;
use super::super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

/// `(mode, size, revision)` of the open drawing — the three things these two
/// sysvars can move. Read through the host so a write is observed exactly as a
/// consumer would see it.
fn style(session: &CadSession) -> (i16, f64, cad_domain::Revision) {
    let drawing = session
        .core
        .controller
        .borrow()
        .drawing()
        .expect("a drawing is open");
    (
        drawing.point_display_mode(),
        drawing.point_display_size(),
        drawing.revision(),
    )
}

#[test]
fn pdmode_and_pdsize_write_the_drawing_header_and_nothing_else() {
    let session = fresh_session();
    let (_, _, before_revision) = style(&session);
    let document = session.core.controller.borrow().document_id;

    session.command("PDMODE 35").expect("35 is a legal $PDMODE");
    let (mode, size, revision) = style(&session);
    assert_eq!(mode, 35, "the raw mode is stored");
    assert_eq!(size, 0.0, "a $PDMODE write never touches $PDSIZE");
    assert_eq!(
        revision,
        cad_domain::Revision(before_revision.0 + 1),
        "the write bumps the revision, which is the display refresh signal"
    );

    session
        .command("pdsize -20")
        .expect("a negative $PDSIZE is a viewport percentage");
    let (mode, size, revision) = style(&session);
    assert_eq!(size, -20.0);
    assert_eq!(mode, 35, "each command writes one value");
    assert_eq!(revision, cad_domain::Revision(before_revision.0 + 2));

    // No undo entry: there is no header-variable patch kind, so the command layer
    // records no history step rather than fabricating one.
    {
        let c = session.core.controller.borrow();
        assert!(!c.application.can_undo(&document));
        assert!(!c.application.can_redo(&document));
    }

    // The admitted canonical selector (never the typed value or its spelling) is
    // echoed in the command log, exactly like the other sysvar lines.
    let echoed: Vec<String> = session
        .snapshot(None, None, 0)
        .command_log
        .iter()
        .filter_map(|entry| entry.command.clone())
        .collect();
    assert!(echoed.contains(&"PDMODE".to_string()), "{echoed:?}");
    assert!(echoed.contains(&"PDSIZE".to_string()), "{echoed:?}");
    assert!(
        !echoed.iter().any(|line| line.contains(' ')),
        "the echo never carries the typed value: {echoed:?}"
    );
}

#[test]
fn the_pdmode_grammar_is_whole_numbers_in_the_i16_range() {
    for accepted in ["0", "3", "35", " 4 ", "+2", "-1", "32767", "-32768"] {
        assert!(
            parse_pdmode_argument(accepted).is_some(),
            "{accepted:?} is a legal $PDMODE"
        );
    }
    // A fraction is refused, never rounded: `$PDMODE` is an integer bit field, so
    // `PDMODE 3.5` can only mean the typist has the wrong variable.
    for refused in [
        "3.5", "0.0", "40000", "-40000", "abc", "3 5", "1e1", "NaN", "inf", "+", "-", "", "3,",
    ] {
        assert_eq!(parse_pdmode_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn the_pdsize_grammar_accepts_sign_and_zero_but_no_non_finite_value() {
    assert_eq!(parse_pdsize_argument("2.5"), Some(2.5));
    assert_eq!(parse_pdsize_argument("0"), Some(0.0));
    assert_eq!(parse_pdsize_argument("-20"), Some(-20.0));
    assert_eq!(parse_pdsize_argument("+4"), Some(4.0));
    assert_eq!(parse_pdsize_argument(" 10.0 "), Some(10.0));
    // A magnitude that overflows `f64` is not a finite size.
    let overflow = "1".repeat(400);
    for refused in [
        "NaN",
        "inf",
        "-inf",
        "abc",
        "2 3",
        ".5",
        "5.",
        "1e3",
        "",
        "--1",
        overflow.as_str(),
    ] {
        assert_eq!(parse_pdsize_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn an_argument_outside_either_grammar_is_refused_by_name_and_changes_nothing() {
    let session = fresh_session();
    let before = style(&session);
    let document = session.core.controller.borrow().document_id;

    for (typed, needle) in [
        ("PDMODE 3.5", "PDMODE needs one whole number"),
        ("PDMODE 40000", "PDMODE needs one whole number"),
        ("PDMODE abc", "PDMODE needs one whole number"),
        ("PDMODE 3 5", "PDMODE needs one whole number"),
        ("PDSIZE NaN", "PDSIZE needs one finite decimal size"),
        ("PDSIZE inf", "PDSIZE needs one finite decimal size"),
        ("PDSIZE 2 3", "PDSIZE needs one finite decimal size"),
    ] {
        let error = session
            .command(typed)
            .expect_err(&format!("{typed:?} must be refused"));
        assert!(error.contains(needle), "{typed:?} got {error:?}");
        assert!(error.contains(typed.trim()), "the typed line is echoed");
        // The refusal is recorded for `cad_status.error`, never silently dropped.
        assert_eq!(
            session.snapshot(None, None, 0).error.as_deref(),
            Some(error.as_str())
        );
    }

    let after = style(&session);
    assert_eq!(after.0, before.0, "the mode is untouched");
    assert_eq!(after.1, before.1, "the size is untouched");
    assert_eq!(
        after.2, before.2,
        "a refused line must not bump the revision (no partial state)"
    );
    assert!(
        !session
            .core
            .controller
            .borrow()
            .application
            .can_undo(&document),
        "a refused line must not record an undo step"
    );
}

#[test]
fn bare_pdmode_and_pdsize_keep_the_name_only_payload_refusal() {
    let session = fresh_session();
    let before = style(&session);
    for (typed, name) in [("PDMODE", "PDMODE"), ("PDSIZE", "PDSIZE")] {
        let error = session
            .command(typed)
            .expect_err("a bare sysvar name carries no value");
        assert!(error.contains(name), "{typed:?} got {error:?}");
    }
    let after = style(&session);
    assert_eq!(after.0, before.0);
    assert_eq!(after.1, before.1);
    assert_eq!(after.2, before.2);
}

#[test]
fn the_two_selectors_round_trip_through_the_command_vocabulary() {
    // Advertised, normalizable, and mapped to their own `CommandId` — no alias
    // table entry, because `PDMODE`/`PDSIZE` are the canonical AutoCAD spellings
    // of exactly these two system variables.
    for (selector, id) in [
        ("PDMODE", CommandId::SetPointDisplayMode),
        ("PDSIZE", CommandId::SetPointDisplaySize),
    ] {
        assert!(
            admissible_names().contains(&selector),
            "{selector} must be advertised"
        );
        assert_eq!(canonical_command_name(selector), Some(selector));
        assert_eq!(normalize_command(selector), Some(id));
        assert_eq!(command_name(id), Some(selector));
    }
    assert!(
        !COMMAND_ALIASES
            .iter()
            .any(|(alias, target)| *alias == "PDMODE"
                || *alias == "PDSIZE"
                || *target == "PDMODE"
                || *target == "PDSIZE"),
        "neither sysvar needs or owns an alias row"
    );
    // Both are documented **document** mutations, so both are Work-only (a Viewer
    // session refuses them through `requires_work_mode`).
    assert!(CommandId::SetPointDisplayMode.requires_work_mode());
    assert!(CommandId::SetPointDisplaySize.requires_work_mode());
}
