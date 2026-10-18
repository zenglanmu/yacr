//! Interactive tool routing, point-capture classification and `status.tool`.

use super::super::*;
use super::test_support::*;

/// The 11 draw selectors mapped to their `cad-app` machine keys.
const DRAW_SELECTORS: &[(&str, &str)] = &[
    ("LINE", "draw.line"),
    ("CIRCLE", "draw.circle"),
    ("ARC", "draw.arc"),
    ("MOVE", "draw.move"),
    ("TRIM", "draw.trim"),
    ("COPY", "draw.copy"),
    ("EXTEND", "draw.extend"),
    ("ROTATE", "draw.rotate"),
    ("SCALE", "draw.scale"),
    ("MIRROR", "draw.mirror"),
    ("OFFSET", "draw.offset"),
];

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn command_routing_starts_every_draw_tool_with_its_cad_app_key() {
    for (selector, key) in DRAW_SELECTORS {
        let session = fresh_session();
        session
            .command(selector)
            .unwrap_or_else(|error| panic!("{selector} refused: {error}"));
        let tool = session
            .snapshot(None, None, 0)
            .tool
            .unwrap_or_else(|| panic!("{selector} started no tool"));
        assert_eq!(&tool.key, key, "{selector}");
        assert_eq!(tool.points, 0, "{selector} starts with no captured point");
        assert!(!tool.ready, "{selector} must not be ready before any point");
    }
}

#[test]
fn command_routing_starts_measurement_distance() {
    let session = fresh_session();
    session.command("MEASURE").expect("MEASURE is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("MEASURE starts a measurement tool");
    assert_eq!(tool.key, "measure.distance");
    assert_eq!(tool.points, 0);
    assert!(!tool.ready);
}

#[test]
fn cancel_clears_the_started_tool() {
    let session = fresh_session();
    session.command("LINE").expect("LINE is admitted");
    assert!(session.snapshot(None, None, 0).tool.is_some());
    session.command("CANCEL").expect("CANCEL is admitted");
    assert_eq!(
        session.snapshot(None, None, 0).tool,
        None,
        "CANCEL clears the active tool"
    );
    assert!(matches!(
        session.core.tool_snapshot(),
        cad_app::ToolSnapshot::Idle
    ));
    // NOTE (evidence honesty): this assertion is **post-state only** — after
    // `CANCEL` both the correct pre-submit sampling and the buggy post-submit
    // sampling leave the tool Idle, so it does not pin the sampling order. The
    // ordering decision itself is pinned by the pure `cancel_target` test in
    // `tests_input.rs`; a discriminating session-level test needs an observable
    // `dispatch_command` seam and is recorded as a follow-up in `docs/handoff.md`.
    assert!(!session.capturing());
}

#[test]
fn a_second_cancel_stays_idle_safe() {
    // Idle-safety only: the host-cancel forwarding of this selector is not
    // observable from this harness (no live import exists here).
    let session = fresh_session();
    session.command("CANCEL").expect("CANCEL is admitted");
    session.command("CANCEL").expect("CANCEL stays admitted");
    assert!(matches!(
        session.core.tool_snapshot(),
        cad_app::ToolSnapshot::Idle
    ));
}

#[test]
fn unknown_command_is_still_refused_with_the_existing_reason() {
    let session = fresh_session();
    let before = session.snapshot(None, None, 0);
    let error = session.command("BOGUS").unwrap_err();
    assert_eq!(error, "unsupported CAD command: BOGUS");
    assert_eq!(session.snapshot(None, None, 0), before);
    assert_eq!(session.snapshot(None, None, 0).tool, None);
}

#[test]
fn a_click_inside_a_capture_captures_a_point_instead_of_selecting() {
    let press = primary_press([10.0, 20.0]);
    let up = pointer_at(InputKind::PointerUp, 2, [10.5, 20.0], None);
    let (requests, next) = plan_input(Some(press), &up, 7, true).unwrap();
    assert!(next.is_none(), "a release ends the press");
    assert_eq!(requests.len(), 2, "release then capture: {requests:?}");
    assert!(matches!(
        requests[0],
        InteractionRequest::Pointer {
            phase: PointerPhase::Up,
            ..
        }
    ));
    assert!(
        matches!(
            &requests[1],
            InteractionRequest::CapturePoint(PointInput::CanvasLocal {
                logical_position,
                metrics_revision,
            }) if *logical_position == [10.5, 20.0] && *metrics_revision == 7
        ),
        "a capture click carries the current revision: {requests:?}"
    );
    assert!(
        !requests
            .iter()
            .any(|request| matches!(request, InteractionRequest::SelectAt { .. })),
        "a click never captures and selects at once"
    );
}

#[test]
fn a_click_without_a_capture_still_selects() {
    let press = primary_press([10.0, 20.0]);
    let up = pointer_at(InputKind::PointerUp, 2, [10.5, 20.0], None);
    let (requests, _) = plan_input(Some(press), &up, 7, false).unwrap();
    assert!(matches!(
        requests[1],
        InteractionRequest::SelectAt {
            mode: SelectionMode::Replace,
            ..
        }
    ));
    assert!(
        !requests
            .iter()
            .any(|request| matches!(request, InteractionRequest::CapturePoint(_))),
        "an idle click must not capture"
    );
}

#[test]
fn a_drag_inside_a_capture_neither_captures_nor_selects() {
    let press = primary_press([10.0, 20.0]);
    let far = pointer_at(
        InputKind::PointerUp,
        2,
        [10.0 + DRAG_THRESHOLD_LOGICAL_PX, 20.0],
        None,
    );
    let (requests, next) = plan_input(Some(press), &far, 7, true).unwrap();
    assert!(next.is_none());
    assert_eq!(
        requests.len(),
        1,
        "a dragged release is only the ordinary release: {requests:?}"
    );
    assert!(
        !requests.iter().any(|request| matches!(
            request,
            InteractionRequest::CapturePoint(_) | InteractionRequest::SelectAt { .. }
        )),
        "a drag never captures a point and never selects"
    );
}

#[test]
fn a_non_primary_click_inside_a_capture_never_captures() {
    let press = PointerPress {
        button: PointerButton::Middle,
        down: [10.0, 20.0],
        dragged: false,
    };
    let up = pointer_at(InputKind::PointerUp, 2, [10.0, 20.0], Some(BUTTON_MIDDLE));
    let (requests, _) = plan_input(Some(press), &up, 7, true).unwrap();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(!requests
        .iter()
        .any(|request| matches!(request, InteractionRequest::CapturePoint(_))));
}

#[test]
fn handle_input_captures_points_while_a_tool_is_active_and_status_reports_them() {
    let session = fresh_session();
    assert_eq!(
        session.snapshot(None, None, 0).tool,
        None,
        "an idle session has no active tool"
    );

    session.command("LINE").expect("LINE is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("LINE is active");
    assert_eq!(tool.key, "draw.line");
    assert_eq!(tool.points, 0);
    assert!(!tool.ready);

    let click = |sequence: u64, pixel: [f64; 2], session: &CadSession| {
        session
            .handle_input(&pointer_at(InputKind::PointerDown, sequence, pixel, None))
            .expect("the press is admitted");
        session
            .handle_input(&pointer_at(InputKind::PointerUp, sequence + 1, pixel, None))
            .expect("the release is admitted");
    };

    click(1, [400.0, 300.0], &session);
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("LINE is active");
    assert_eq!(tool.points, 1, "a click in a capture captures one point");
    assert!(!tool.ready, "one point does not define a LINE");
    assert!(
        session
            .core
            .controller
            .borrow()
            .session
            .selection
            .is_empty(),
        "a capture click must not select"
    );

    click(3, [500.0, 350.0], &session);
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("LINE is active");
    assert_eq!(tool.points, 2, "a second click captures the end point");
    assert!(tool.ready, "two points define a LINE");
}
