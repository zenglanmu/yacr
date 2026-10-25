//! `SNAP` (grid snap, F9) and the typed `SNAPUNIT <value>` spacing sysvar.
//!
//! The quantiser and its session wiring are `cad-app`'s authority
//! (`crates/cad-app/src/snap.rs`, `SessionState::capture_point`); these tests pin
//! the **router** side: the two selectors, the shared typed-argument machinery and
//! the status projection. A refused value must change nothing, and a successful
//! one must stay session state (no revision, no history, no preference write).

use super::super::super::*;
use super::super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn snap_selector_toggles_the_projected_grid_snap_flag_only() {
    let session = fresh_session();
    let (document, before) = {
        let c = session.core.controller.borrow();
        (c.document_id, c.drawing().unwrap())
    };
    let settings_before = session.settings();
    let status = session.snapshot(None, None, 0);
    assert!(
        !status.snap_grid_enabled,
        "SNAP defaults off (AutoCAD's initial state) and is projected"
    );
    assert_eq!(
        status.snap_spacing,
        cad_app::DEFAULT_SNAP_SPACING,
        "the spacing sysvar starts at the documented default"
    );

    // The selector flips the session flag and nothing else: object snap, the
    // display grid and ORTHO are independent switches.
    session.command("SNAP").expect("SNAP is admitted");
    let status = session.snapshot(None, None, 0);
    assert!(status.snap_grid_enabled);
    assert!(status.snap_enabled, "SNAP must not flip object snapping");
    assert!(status.grid_enabled, "SNAP must not flip the display grid");
    assert!(!status.ortho_enabled, "SNAP must not flip ORTHO");

    // Case-insensitive, and it toggles back off with the same selector.
    session.command("snap").expect("SNAP is case-insensitive");
    assert!(!session.snapshot(None, None, 0).snap_grid_enabled);

    // Session/display state only: no revision bump, no undo entry and no
    // persisted preference write.
    let after = session.core.controller.borrow().drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.entity_count(), before.entity_count());
    {
        let c = session.core.controller.borrow();
        assert!(!c.application.can_undo(&document));
        assert!(!c.application.can_redo(&document));
    }
    assert_eq!(session.settings(), settings_before);
    assert!(!session.settings().grid);
    // Neither selector needs Work mode: both are Viewer-safe session state.
    assert!(!CommandId::ToggleSnapGrid.requires_work_mode());
    assert!(!CommandId::SetSnapSpacing.requires_work_mode());
}

#[test]
fn snapunit_sets_the_projected_spacing_and_stays_session_state() {
    let session = fresh_session();
    let (document, before) = {
        let c = session.core.controller.borrow();
        (c.document_id, c.drawing().unwrap())
    };
    let settings_before = session.settings();

    for (typed, expected) in [
        ("SNAPUNIT 25", 25.0),
        ("snapunit 0.5", 0.5),
        ("  SNAPUNIT\t2.5  ", 2.5),
        ("SnapUnit 10.0", 10.0),
    ] {
        session
            .command(typed)
            .unwrap_or_else(|error| panic!("{typed:?} refused: {error}"));
        assert_eq!(
            session.snapshot(None, None, 0).snap_spacing,
            expected,
            "{typed:?} must set exactly that spacing"
        );
        // The admitted selector name (never the typed value) is echoed.
        assert_eq!(
            session
                .snapshot(None, None, 0)
                .command_log
                .last()
                .and_then(|entry| entry.command.as_deref()),
            Some("SNAPUNIT"),
            "{typed:?} must echo the canonical selector"
        );
    }

    // The spacing sysvar never touches document state or preferences.
    let after = session.core.controller.borrow().drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.entity_count(), before.entity_count());
    {
        let c = session.core.controller.borrow();
        assert!(!c.application.can_undo(&document));
        assert!(!c.application.can_redo(&document));
    }
    assert_eq!(session.settings(), settings_before);
}

#[test]
fn a_snapunit_value_outside_the_grammar_is_refused_and_changes_nothing() {
    let session = fresh_session();
    session.command("SNAPUNIT 12").expect("12 is admitted");
    for typed in [
        "SNAPUNIT 0",
        "SNAPUNIT 0.0",
        "SNAPUNIT -5",
        "SNAPUNIT +5",
        "SNAPUNIT abc",
        "SNAPUNIT 2 3",
        "SNAPUNIT .5",
        "SNAPUNIT 5.",
        "SNAPUNIT 1e3",
        "SNAPUNIT ALL",
        "SNAPUNIT NaN",
    ] {
        let error = session
            .command(typed)
            .expect_err(&format!("{typed:?} must be refused"));
        assert!(
            error.contains("SNAPUNIT needs a positive decimal spacing"),
            "{typed:?} must carry the named reason, got {error:?}"
        );
        assert!(
            error.contains(typed.trim()),
            "{typed:?} must echo the typed line, got {error:?}"
        );
        assert_eq!(
            session.snapshot(None, None, 0).snap_spacing,
            12.0,
            "{typed:?} must leave the spacing untouched"
        );
        // The refusal is recorded for `cad_status.error`, never a silent ignore.
        assert_eq!(
            session.snapshot(None, None, 0).error.as_deref(),
            Some(error.as_str()),
            "{typed:?} must be visible in cad_status.error"
        );
    }
    let document = session.core.controller.borrow().document_id;
    assert!(
        !session
            .core
            .controller
            .borrow()
            .application
            .can_undo(&document),
        "a refused SNAPUNIT must not record an undo step"
    );
}

#[test]
fn bare_snapunit_keeps_the_name_only_payload_refusal() {
    let session = fresh_session();
    // `SNAPUNIT` alone is a payload-bearing selector: it is not given a
    // fabricated default spacing, and it does not silently become a no-op.
    let error = session
        .command("SNAPUNIT")
        .expect_err("bare SNAPUNIT is refused");
    assert!(
        error.contains("SNAPUNIT"),
        "the refusal names the command: {error:?}"
    );
    assert_eq!(
        session.snapshot(None, None, 0).snap_spacing,
        cad_app::DEFAULT_SNAP_SPACING,
        "a refusal never invents or changes a spacing"
    );
}

#[test]
fn the_snapunit_grammar_is_the_documented_one_and_reuses_the_argument_machinery() {
    assert_eq!(parse_snap_unit_argument("25"), Some(25.0));
    assert_eq!(parse_snap_unit_argument("0.5"), Some(0.5));
    assert_eq!(parse_snap_unit_argument("2.5"), Some(2.5));
    assert_eq!(parse_snap_unit_argument(" 10.0 "), Some(10.0));
    for refused in [
        "0", "0.0", "-5", "+5", "abc", "2 3", ".5", "5.", "1e3", "ALL", "NaN", "inf", "",
    ] {
        assert_eq!(parse_snap_unit_argument(refused), None, "{refused:?}");
    }

    // The shared splitter knows both argument-taking selectors.
    assert_eq!(ARGUMENT_SELECTORS, &["ZOOM", "SNAPUNIT"]);
    assert_eq!(
        typed_command_argument("SNAPUNIT 5"),
        Some(("SNAPUNIT", "5"))
    );
    assert_eq!(
        typed_command_argument("  snapunit\t0.5  "),
        Some(("SNAPUNIT", "0.5"))
    );
    assert_eq!(
        typed_command_argument("SNAPUNIT"),
        None,
        "a bare name has no argument"
    );
    assert_eq!(
        typed_command_argument("SNAP 5"),
        None,
        "SNAP takes no argument"
    );
    assert_eq!(typed_command_argument("LINE 2"), None);
    // `SNAPUNIT` is advertised, `SNAPUNIT 5` is not a name.
    assert!(admissible_names().contains(&"SNAPUNIT"));
    assert!(admissible_names().contains(&"SNAP"));
    assert_eq!(
        normalize_command("SNAPUNIT"),
        Some(CommandId::SetSnapSpacing)
    );
    assert_eq!(normalize_command("SNAP"), Some(CommandId::ToggleSnapGrid));
    // No alias turns SNAP into OSNAP, GRID or ORTHO: the drafting aids are only
    // ever named by their own canonical selectors.
    assert!(
        alias_rows_for("SNAP").is_empty(),
        "SNAP must have no alias rows: {:?}",
        alias_rows_for("SNAP")
    );

    // The shell gate queues the full typed line (never just the name), exactly
    // like `ZOOM 2`; an out-of-grammar value is still admitted by name because
    // the session — not the gate — validates it.
    let mut state = crate::shell::ShellState::new(true);
    crate::shell::admit_command(&mut state, "snapunit 5")
        .expect("a typed spacing line is admitted");
    assert_eq!(state.cad_commands, vec!["SNAPUNIT 5".to_string()]);
    crate::shell::admit_command(&mut state, "SNAPUNIT 0").expect("admitted by name");
    assert_eq!(
        state.cad_commands.last().map(String::as_str),
        Some("SNAPUNIT 0")
    );
    assert!(crate::shell::admit_command(&mut state, "SNAP 5").is_err());
    assert_eq!(state.cad_commands.len(), 2);
}

/// Every alias row that names `name` on either side, for the invariant that no
/// alias silently re-points a drafting aid at another one.
fn alias_rows_for(name: &str) -> Vec<&'static str> {
    COMMAND_ALIASES
        .iter()
        .filter(|(alias, target)| *alias == name || *target == name)
        .map(|(alias, _)| *alias)
        .collect()
}
