//! `WALK` / `LOOKAROUND` selectors and typed navigation forms.
//!
//! The bare selectors start one-point captures; the typed forms carry a distance
//! or yaw/pitch. Contract level: no GUI, no GPU.

use super::super::*;
use super::test_support::*;
use crate::session::commands::{parse_lookaround_argument, parse_walk_argument};
use cad_app::{LookAroundCommand, WalkCommand};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_walk_selector_starts_the_one_point_capture() {
    assert!(admissible_names().contains(&"WALK"));
    assert_eq!(normalize_command("walk"), Some(CommandId::Walk));
    assert_eq!(command_name(CommandId::Walk), Some("WALK"));
    assert!(matches!(
        commands::interaction_request_for(CommandId::Walk),
        Some(cad_app::interaction::InteractionRequest::BeginDraw(
            cad_app::DrawToolKind::Walk
        ))
    ));
    let session = fresh_session();
    session.command("WALK").expect("WALK is admitted");
    let tool = session.snapshot(None, None, 0).tool.expect("a tool");
    assert_eq!(tool.key, "draw.walk");
    assert_eq!(tool.points, 0);
    assert!(!tool.ready);
}

#[test]
fn the_lookaround_selector_starts_the_one_point_capture() {
    assert!(admissible_names().contains(&"LOOKAROUND"));
    assert_eq!(normalize_command("lookaround"), Some(CommandId::LookAround));
    assert_eq!(command_name(CommandId::LookAround), Some("LOOKAROUND"));
    assert!(matches!(
        commands::interaction_request_for(CommandId::LookAround),
        Some(cad_app::interaction::InteractionRequest::BeginDraw(
            cad_app::DrawToolKind::LookAround
        ))
    ));
    let session = fresh_session();
    session
        .command("LOOKAROUND")
        .expect("LOOKAROUND is admitted");
    let tool = session.snapshot(None, None, 0).tool.expect("a tool");
    assert_eq!(tool.key, "draw.lookaround");
}

#[test]
fn the_typed_walk_and_lookaround_grammars_are_numeric_only() {
    assert_eq!(parse_walk_argument("5"), Some(WalkCommand::By(5.0)));
    assert_eq!(parse_walk_argument("-2.5"), Some(WalkCommand::By(-2.5)));
    for refused in ["", "  ", "abc", "NaN", "inf", "5 6"] {
        assert_eq!(parse_walk_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        parse_lookaround_argument("15 -10"),
        Some(LookAroundCommand::By {
            yaw_degrees: 15.0,
            pitch_degrees: -10.0
        })
    );
    for refused in ["", "15", "15 -10 3", "x y", "NaN 0"] {
        assert_eq!(parse_lookaround_argument(refused), None, "{refused:?}");
    }
    assert_eq!(typed_command_argument("WALK 5"), Some(("WALK", "5")));
    assert_eq!(
        typed_command_argument("LOOKAROUND 15 -10"),
        Some(("LOOKAROUND", "15 -10"))
    );
    // A malformed typed line is refused by the grammar, before any camera move.
    let session = fresh_session();
    let error = session
        .command("LOOKAROUND 15")
        .expect_err("not enough angles");
    assert!(error.contains("unsupported CAD command"), "{error}");
    assert!(session.snapshot(None, None, 0).tool.is_none());
}
