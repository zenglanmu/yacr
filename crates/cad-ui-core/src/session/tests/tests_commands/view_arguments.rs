//! The 2026-11-07 view batch typed lines: `VIEW`, `-VIEW`, `VIEWGO`, `VPOINT`.
//!
//! Pins the grammar, the single-selector rule (`-VIEW` is an alias, not a second
//! command), the named-view round trip through the session, the back/forward
//! no-ops and the `VPOINT` zero-vector refusal. Every assertion is contract
//! level: no GUI, no GPU.

use super::super::super::*;
use super::super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn camera(session: &CadSession) -> CadCamera {
    session
        .snapshot(None, None, 0)
        .camera
        .expect("the demo view is a 2D plan")
}

fn assert_no_history(session: &CadSession) {
    let c = session.core.controller.borrow();
    assert!(!c.application.can_undo(&c.document_id));
    assert!(!c.application.can_redo(&c.document_id));
}

#[test]
fn view_has_one_typed_grammar_and_view_is_an_alias() {
    assert_eq!(
        parse_view_argument("SAVE Plan A"),
        Some(ViewOption::Save("Plan A".into()))
    );
    assert_eq!(
        parse_view_argument("restore plan a"),
        Some(ViewOption::Restore("plan a".into()))
    );
    assert_eq!(parse_view_argument("LIST"), Some(ViewOption::List));
    assert_eq!(
        parse_view_argument("delete  Old  "),
        Some(ViewOption::Delete("Old".into()))
    );
    for refused in ["", "SAVE", "RESTORE", "DELETE", "LIST extra", "BOGUS"] {
        assert_eq!(parse_view_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("VIEW SAVE Plan A"),
        Some(("VIEW", "SAVE Plan A"))
    );
    assert_eq!(typed_command_argument("VIEW"), None);
    // `-VIEW` is the command-line spelling of the same selector.
    assert_eq!(canonical_command_name("-VIEW"), Some("VIEW"));
    assert_eq!(normalize_command("-VIEW"), Some(CommandId::View));
    assert_eq!(
        typed_command_argument("-VIEW RESTORE Plan A"),
        Some(("VIEW", "RESTORE Plan A"))
    );
    assert_eq!(command_name(CommandId::View), Some("VIEW"));
    assert!(!CommandId::View.requires_work_mode());
}

#[test]
fn viewgo_and_vpoint_grammars_are_narrow() {
    assert_eq!(parse_viewgo_argument("Plan A"), Some("Plan A".into()));
    assert_eq!(parse_viewgo_argument(" Plan A "), Some("Plan A".into()));
    assert_eq!(parse_viewgo_argument("   "), None);
    assert_eq!(
        typed_command_argument("VIEWGO Plan A"),
        Some(("VIEWGO", "Plan A"))
    );
    assert_eq!(typed_command_argument("VIEWGO"), None);
    assert_eq!(command_name(CommandId::ViewGo), Some("VIEWGO"));

    assert_eq!(
        parse_vpoint_argument("TOP"),
        Some(Point3Input::Standard(cad_app::StandardView::Top))
    );
    assert_eq!(
        parse_vpoint_argument("back"),
        Some(Point3Input::Standard(cad_app::StandardView::Back))
    );
    assert_eq!(
        parse_vpoint_argument("1 -2.5 3"),
        Some(Point3Input::Vector([1.0, -2.5, 3.0]))
    );
    for refused in ["", "ISOMETRIC", "1 2", "1 2 3 4", "x 0 0", "1e3 0 0"] {
        assert_eq!(parse_vpoint_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("VPOINT 0 0 1"),
        Some(("VPOINT", "0 0 1"))
    );
    assert_eq!(typed_command_argument("VPOINT"), None);
    assert_eq!(command_name(CommandId::VPoint), Some("VPOINT"));
}

#[test]
fn named_view_round_trips_through_the_session() {
    let session = fresh_session();
    let saved = camera(&session);

    session
        .command("VIEW SAVE Start")
        .expect("VIEW SAVE is admitted");
    session.command("ZOOM 2").expect("ZOOM 2 is admitted");
    assert_ne!(camera(&session), saved);

    session
        .command("VIEWGO start")
        .expect("VIEWGO resolves names case-insensitively");
    assert_eq!(camera(&session), saved);
    assert_no_history(&session);

    session.command("ZOOM 3").expect("ZOOM 3 is admitted");
    // The `-VIEW` alias reaches the same restore path.
    session
        .command("-VIEW RESTORE Start")
        .expect("-VIEW is the same command");
    assert_eq!(camera(&session), saved);
    assert_no_history(&session);
}

#[test]
fn unknown_named_view_is_refused_by_name() {
    let session = fresh_session();
    let error = session.command("VIEWGO missing").unwrap_err();
    assert!(error.contains("view.unknown"), "{error}");
    let error = session.command("VIEW DELETE missing").unwrap_err();
    assert!(error.contains("view.unknown"), "{error}");
}

#[test]
fn viewback_and_viewforward_empty_stacks_are_named_no_ops() {
    let session = fresh_session();
    // A fresh session has an empty view history on its viewport: both are named
    // no-op diagnostics, never an error.
    session
        .command("VIEWBACK")
        .expect("an empty back stack is not an error");
    session
        .command("VIEWFORWARD")
        .expect("an empty forward stack is not an error");
}

#[test]
fn vpoint_zero_vector_is_refused_before_any_camera_change() {
    let session = fresh_session();
    let before = camera(&session);
    let error = session.command("VPOINT 0 0 0").unwrap_err();
    assert!(error.contains("vpoint.invalid_vector"), "{error}");
    assert_eq!(camera(&session), before);
    assert_no_history(&session);

    session
        .command("VPOINT 0 0 1")
        .expect("a non-zero vector is admitted");
    let error = session.command("VPOINT 1 2").unwrap_err();
    assert!(!error.is_empty());
}
