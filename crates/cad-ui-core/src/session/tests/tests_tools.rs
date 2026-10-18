//! Interactive tool routing, point-capture classification and `status.tool`.

use super::super::*;
use super::test_support::*;

/// The 14 draw selectors mapped to their `cad-app` machine keys.
const DRAW_SELECTORS: &[(&str, &str)] = &[
    ("LINE", "draw.line"),
    ("CIRCLE", "draw.circle"),
    ("ARC", "draw.arc"),
    ("PLINE", "draw.polyline"),
    ("RECTANG", "draw.rectang"),
    ("ELLIPSE", "draw.ellipse"),
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
fn rectang_selector_is_admitted_and_starts_the_rectangle_kind() {
    // ADR 0015 P3a: RECTANG is a real `COMMAND_TABLE` draw selector that routes to
    // `BeginDraw(Rectang)`, and it is disjoint from the interaction-only selector
    // families (a measurement/zoom name would otherwise silently win the lookup).
    assert!(admissible_names().contains(&"RECTANG"));
    assert_eq!(canonical_command_name("RECTANG"), Some("RECTANG"));
    assert_eq!(
        normalize_command("rectang"),
        Some(CommandId::CreateRectangle)
    );
    assert_eq!(measurement_selector_kind("RECTANG"), None);
    assert!(zoom_step_factor("RECTANG").is_none());
    assert!(
        !COMMAND_ALIASES.iter().any(|(alias, _)| *alias == "RECTANG"),
        "RECTANG must not be an alias"
    );

    // The session really starts the rectangle kind (the typed request reaches the
    // host), with no captured point and not ready.
    let session = fresh_session();
    session.command("RECTANG").expect("RECTANG is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("RECTANG starts a tool");
    assert_eq!(tool.key, "draw.rectang");
    assert_eq!(tool.points, 0);
    assert!(!tool.ready);

    // Two clicks make it ready: the router-started tool is the real arity-2 capture.
    let capture = |pixel: [f64; 2], sequence: u64| {
        session
            .handle_input(&pointer_at(InputKind::PointerDown, sequence, pixel, None))
            .expect("the press is admitted");
        session
            .handle_input(&pointer_at(InputKind::PointerUp, sequence + 1, pixel, None))
            .expect("the release is admitted");
    };
    capture([400.0, 300.0], 1);
    assert_eq!(session.snapshot(None, None, 0).tool.unwrap().points, 1);
    capture([460.0, 340.0], 3);
    let tool = session.snapshot(None, None, 0).tool.unwrap();
    assert_eq!(tool.key, "draw.rectang");
    assert_eq!(tool.points, 2);
    assert!(tool.ready, "two corners define a rectangle");
}

#[test]
fn ellipse_selector_is_admitted_and_starts_the_ellipse_kind() {
    // ADR 0015 P3a: ELLIPSE is a real `COMMAND_TABLE` draw selector that routes to
    // `BeginDraw(Ellipse)`, and it is disjoint from the interaction-only selector
    // families (a measurement/zoom name would otherwise silently win the lookup).
    assert!(admissible_names().contains(&"ELLIPSE"));
    assert_eq!(canonical_command_name("ELLIPSE"), Some("ELLIPSE"));
    assert_eq!(normalize_command("ellipse"), Some(CommandId::CreateEllipse));
    assert_eq!(measurement_selector_kind("ELLIPSE"), None);
    assert!(zoom_step_factor("ELLIPSE").is_none());
    assert!(
        !COMMAND_ALIASES.iter().any(|(alias, _)| *alias == "ELLIPSE"),
        "ELLIPSE must not be an alias"
    );

    // The session really starts the ellipse kind (the typed request reaches the
    // host), with no captured point and not ready.
    let session = fresh_session();
    session.command("ELLIPSE").expect("ELLIPSE is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("ELLIPSE starts a tool");
    assert_eq!(tool.key, "draw.ellipse");
    assert_eq!(tool.points, 0);
    assert!(!tool.ready);

    // Three clicks make it ready: the router-started tool is the real arity-3
    // axis-endpoint capture (ready exactly at the third pick, not before).
    let capture = |pixel: [f64; 2], sequence: u64| {
        session
            .handle_input(&pointer_at(InputKind::PointerDown, sequence, pixel, None))
            .expect("the press is admitted");
        session
            .handle_input(&pointer_at(InputKind::PointerUp, sequence + 1, pixel, None))
            .expect("the release is admitted");
    };
    capture([400.0, 300.0], 1);
    assert_eq!(session.snapshot(None, None, 0).tool.unwrap().points, 1);
    capture([500.0, 300.0], 3);
    let tool = session.snapshot(None, None, 0).tool.unwrap();
    assert_eq!(tool.points, 2);
    assert!(!tool.ready, "two picks are not an ellipse");
    capture([450.0, 360.0], 5);
    let tool = session.snapshot(None, None, 0).tool.unwrap();
    assert_eq!(tool.key, "draw.ellipse");
    assert_eq!(tool.points, 3);
    assert!(tool.ready, "three picks define an ellipse");
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
fn measurement_selectors_start_distinct_kinds() {
    // The four measurement selectors dispatch `BeginMeasurement(<kind>)`; the
    // four kinds are distinct, so no button silently collapses onto Distance.
    let cases: &[(&str, cad_app::MeasurementToolKind, &str)] = &[
        (
            "DIST",
            cad_app::MeasurementToolKind::Distance,
            "measure.distance",
        ),
        (
            "MEASUREPOLYLINE",
            cad_app::MeasurementToolKind::PolylineLength,
            "measure.polyline",
        ),
        (
            "MEASUREANGLE",
            cad_app::MeasurementToolKind::Angle,
            "measure.angle",
        ),
        ("AREA", cad_app::MeasurementToolKind::Area, "measure.area"),
    ];
    // Every selector is router-admitted; the UI enables it from this one
    // vocabulary (never a second list).
    let admitted = admissible_names();
    for (selector, kind, _) in cases {
        assert!(admitted.contains(selector), "{selector} not advertised");
        assert_eq!(canonical_command_name(selector), Some(*selector));
        assert_eq!(
            measurement_selector_kind(selector),
            Some(*kind),
            "{selector} maps to the wrong kind"
        );
    }
    let distinct: std::collections::BTreeSet<&str> = cases.iter().map(|(_, _, key)| *key).collect();
    assert_eq!(
        distinct.len(),
        4,
        "the four measurement kinds must be distinct"
    );
    // The session really starts each kind (the typed request reaches the host).
    for (selector, _, key) in cases {
        let session = fresh_session();
        session
            .command(selector)
            .unwrap_or_else(|error| panic!("{selector} refused: {error}"));
        let tool = session
            .snapshot(None, None, 0)
            .tool
            .unwrap_or_else(|| panic!("{selector} started no tool"));
        assert_eq!(&tool.key, key, "{selector} started the wrong kind");
    }
    // `MEASURE` remains the default Distance selector.
    let session = fresh_session();
    session.command("MEASURE").expect("MEASURE is admitted");
    assert_eq!(
        session.snapshot(None, None, 0).tool.unwrap().key,
        "measure.distance"
    );
}

#[test]
fn pan_selector_enters_modal_pan_without_a_host_error() {
    let session = fresh_session();
    // The bare `PAN` selector must not reach the payload-requiring
    // `CommandId::Pan` command (which needs a `Points` world delta and would
    // error). It starts modal pan mode exactly like the other interactive tools.
    session.command("PAN").expect("PAN is admitted");
    assert!(
        matches!(session.core.tool_snapshot(), cad_app::ToolSnapshot::Panning),
        "PAN must enter modal pan mode"
    );
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.error, None, "PAN must not produce a host error");
}

#[test]
fn zoom_step_selectors_are_opposite_and_centred() {
    let session = fresh_session();
    let base = session
        .snapshot(None, None, 0)
        .camera
        .expect("the demo view is a 2D plan");
    // The two selectors are router-admitted and carry opposite factors.
    for selector in ["ZOOMIN", "ZOOMOUT"] {
        assert!(admissible_names().contains(&selector), "{selector}");
        assert_eq!(canonical_command_name(selector), Some(selector));
    }
    let zoom_in_factor = zoom_step_factor("ZOOMIN").expect("ZOOMIN has a factor");
    let zoom_out_factor = zoom_step_factor("ZOOMOUT").expect("ZOOMOUT has a factor");
    assert_eq!(zoom_in_factor, ZOOM_IN_FACTOR);
    assert_eq!(zoom_out_factor, ZOOM_OUT_FACTOR);
    assert!(zoom_in_factor > 1.0, "zoom in must enlarge");
    assert!(zoom_out_factor < 1.0, "zoom out must shrink");
    assert!(
        (zoom_in_factor * zoom_out_factor - 1.0).abs() < 1e-12,
        "the two steps are exact opposites"
    );

    session.command("ZOOMIN").expect("ZOOMIN is admitted");
    let zoomed_in = session
        .snapshot(None, None, 0)
        .camera
        .expect("still a 2D plan");
    // Zooming in reduces world-units-per-pixel; the centre is unchanged because
    // the step is anchored at the viewport centre.
    assert!(
        zoomed_in.scale < base.scale,
        "zoom in must reduce the scale"
    );
    assert_eq!(zoomed_in.center, base.center, "zoom is about the centre");

    session.command("ZOOMOUT").expect("ZOOMOUT is admitted");
    let zoomed_out = session
        .snapshot(None, None, 0)
        .camera
        .expect("still a 2D plan");
    assert!(
        zoomed_out.scale > zoomed_in.scale,
        "zoom out must increase the scale"
    );
    assert_eq!(zoomed_out.center, base.center, "zoom is about the centre");
    assert!(
        (zoomed_out.scale - base.scale).abs() < 1e-9,
        "opposite steps round trip"
    );
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
fn status_projects_the_best_snap_candidate_kind_or_none() {
    let session = fresh_session();
    // Idle: no point prompt, so no candidate kind is projected, never a
    // placeholder.
    assert_eq!(session.snapshot(None, None, 0).snap_kind, None);

    session.command("LINE").expect("LINE is admitted");
    // A live draw prompt with no cursor yet: still no candidate.
    assert_eq!(session.snapshot(None, None, 0).snap_kind, None);

    // The demo circle's 0° rim is a real quadrant candidate; the projection
    // reports the same best-candidate kind the snap overlay resolves.
    let rim = demo_circle_rim(&session);
    session
        .core
        .controller
        .borrow_mut()
        .move_tool_cursor(rim)
        .expect("the cursor move is admitted");
    assert_eq!(
        session.snapshot(None, None, 0).snap_kind.as_deref(),
        Some("quadrant")
    );

    // Far from any geometry the honest `None` is reported, never a fabricated
    // kind.
    session
        .core
        .controller
        .borrow_mut()
        .move_tool_cursor(Point3 {
            x: 1_000_000.0,
            y: 1_000_000.0,
            z: 0.0,
        })
        .expect("a far cursor move is admitted");
    assert_eq!(session.snapshot(None, None, 0).snap_kind, None);
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
