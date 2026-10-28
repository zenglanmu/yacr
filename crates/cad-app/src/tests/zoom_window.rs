//! `ZOOM WINDOW`: the exact two-corner fit, 2D orthographic only.
//!
//! The contract pinned here: the framing math (centre and `max(ex/w, ey/h)`
//! scale with margin 1.0, compared against an independent expectation), the
//! 2D-only refusal by name, the degenerate/non-finite refusals that change
//! nothing, and the fact that a window is session view state only — no database
//! write, no revision bump, no `cad-history` entry, no generation bump.

use super::*;

/// The active viewport.
fn viewport(app: &Application) -> &Viewport {
    &app.workspace.viewports[&ViewportId(1)]
}

/// The orthographic world-units-per-pixel scale of the active viewport.
fn scale(app: &Application) -> f64 {
    match viewport(app).camera.projection {
        Projection::Orthographic { scale } => scale,
        Projection::Perspective { .. } => panic!("expected the 2D plan projection"),
    }
}

fn zoom_window(
    app: &mut Application,
    session: &mut SessionState,
    rect: [[f64; 2]; 2],
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(
            CommandId::Zoom,
            CommandPayload::ZoomWindow { rect_logical: rect },
        ),
    )
}

fn codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

#[test]
fn a_known_logical_rect_is_framed_exactly() {
    // 800x600 canvas, plan camera at the origin with scale 1.0: one logical
    // pixel is one world unit, so the logical rect `(200,150)-(600,450)` is the
    // world rect `(-200,150)-(200,-150)`.
    let (mut app, mut session) = application_with_document();
    let before = viewport(&app).camera;
    assert_eq!(scale(&app), 1.0);

    zoom_window(&mut app, &mut session, [[200.0, 150.0], [600.0, 450.0]])
        .expect("a 2D window is admitted");

    let after = viewport(&app);
    // Independent expectation: the rect spans half the canvas in both axes
    // (400/800 and 300/600), so `max(ex/w, ey/h) * 1.0` is 0.5.
    assert!(
        (scale(&app) - 0.5).abs() < 1e-12,
        "scale {} != 0.5",
        scale(&app)
    );
    assert_eq!(after.camera.target.x, 0.0);
    assert_eq!(after.camera.target.y, 0.0);
    assert_eq!(after.camera.target.z, 0.0);
    // The plan eye keeps its height above the plane (a window changes the
    // framing, not the eye height).
    assert_eq!(after.camera.eye.z, before.eye.z);
    assert_eq!(after.camera.eye.x, after.camera.target.x);
    assert_eq!(after.camera.eye.y, after.camera.target.y);

    // Exactness: the picked world corners project back onto the canvas corners.
    let bottom_left = Point3 {
        x: -200.0,
        y: -150.0,
        z: 0.0,
    };
    let top_right = Point3 {
        x: 200.0,
        y: 150.0,
        z: 0.0,
    };
    let bl = after
        .camera
        .world_to_screen(bottom_left, after.logical_size)
        .expect("the plan projects the corner");
    let tr = after
        .camera
        .world_to_screen(top_right, after.logical_size)
        .expect("the plan projects the corner");
    assert!((bl[0] - 0.0).abs() < 1e-9 && (bl[1] - 600.0).abs() < 1e-9);
    assert!((tr[0] - 800.0).abs() < 1e-9 && (tr[1] - 0.0).abs() < 1e-9);
}

#[test]
fn the_tighter_axis_decides_the_scale() {
    // A wide, short rect: 800 x 60 logical pixels -> ex/w = 1, ey/h = 0.1.
    let (mut app, mut session) = application_with_document();
    zoom_window(&mut app, &mut session, [[0.0, 270.0], [800.0, 330.0]]).unwrap();
    assert!(
        (scale(&app) - 1.0).abs() < 1e-12,
        "wide rect: {}",
        scale(&app)
    );

    // A tall, narrow rect: 60 x 600 logical pixels -> ex/w = 0.075, ey/h = 1.
    let (mut app, mut session) = application_with_document();
    zoom_window(&mut app, &mut session, [[370.0, 0.0], [430.0, 600.0]]).unwrap();
    assert!(
        (scale(&app) - 1.0).abs() < 1e-12,
        "tall rect: {}",
        scale(&app)
    );

    // A square rect of 200 x 200 px -> the tighter axis is the vertical one
    // (200/600 = 1/3 beats 200/800 = 0.25), so the height decides.
    let (mut app, mut session) = application_with_document();
    zoom_window(&mut app, &mut session, [[300.0, 200.0], [500.0, 400.0]]).unwrap();
    assert!(
        (scale(&app) - 1.0 / 3.0).abs() < 1e-12,
        "square rect: {}",
        scale(&app)
    );
}

#[test]
fn a_window_is_exact_while_fit_keeps_its_margin() {
    let min = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let max = Point3 {
        x: 400.0,
        y: 300.0,
        z: 0.0,
    };
    let (mut app, mut session) = application_with_bounds(min, max);

    app.execute(
        &mut session,
        command(CommandId::FitDrawing, CommandPayload::None),
    )
    .unwrap();
    // FIT frames the same bounds with 5 % breathing room.
    assert!(
        (scale(&app) - 0.5 * FIT_MARGIN).abs() < 1e-12,
        "FIT scale {} != 0.5 * {FIT_MARGIN}",
        scale(&app)
    );
    let fitted = viewport(&app).camera;

    // Now window exactly the document corners, read from the fitted view.
    let first = fitted
        .world_to_screen(min, viewport(&app).logical_size)
        .unwrap();
    let second = fitted
        .world_to_screen(max, viewport(&app).logical_size)
        .unwrap();
    zoom_window(&mut app, &mut session, [first, second]).unwrap();
    assert!(
        (scale(&app) - 0.5 * WINDOW_MARGIN).abs() < 1e-12,
        "window scale {} != 0.5",
        scale(&app)
    );
    assert!((viewport(&app).camera.target.x - 200.0).abs() < 1e-9);
    assert!((viewport(&app).camera.target.y - 150.0).abs() < 1e-9);
}

#[test]
fn a_window_stays_in_2d_and_records_no_document_history() {
    let (mut app, mut session) = application_with_document();
    let revision = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let drawing = app.workspace.documents[&DocumentId(1)].drawing.clone();
    let generation = session.generation;

    zoom_window(&mut app, &mut session, [[100.0, 100.0], [700.0, 500.0]]).unwrap();

    let vp = viewport(&app);
    assert_eq!(vp.view_mode.kind(), ProjectionKind::TwoD);
    assert_eq!(
        vp.view_mode.saved_2d_camera(),
        vp.camera,
        "the saved 2D camera tracks the window like every other 2D zoom"
    );
    // A plain zoom is not document/task work: the generation stamp is untouched.
    assert_eq!(session.generation, generation);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        revision
    );
    assert!(Arc::ptr_eq(
        &drawing,
        &app.workspace.documents[&DocumentId(1)].drawing
    ));
    assert_eq!(
        app.history_availability(&DocumentId(1)),
        HistoryAvailability::default(),
        "a window must never record an undo/redo step"
    );
}

#[test]
fn a_three_d_view_refuses_the_window_by_name() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(CommandId::Switch2d3d, CommandPayload::None),
    )
    .unwrap();
    let camera = viewport(&app).camera;
    let steps = viewport(&app).zoom_history_len();

    let error = zoom_window(&mut app, &mut session, [[100.0, 100.0], [700.0, 500.0]])
        .expect_err("a window in 3D is refused");
    assert!(
        error.to_string().contains("zoom.window.requires_2d"),
        "the refusal is named: {error}"
    );
    assert_eq!(viewport(&app).camera, camera, "nothing moved");
    assert_eq!(viewport(&app).view_mode.kind(), ProjectionKind::ThreeD);
    assert_eq!(
        viewport(&app).zoom_history_len(),
        steps,
        "a refusal plants no ZOOM PREVIOUS step"
    );
}

#[test]
fn degenerate_and_non_finite_rects_are_refused_and_change_nothing() {
    let (mut app, mut session) = application_with_document();
    let camera = viewport(&app).camera;
    let steps = viewport(&app).zoom_history_len();

    // Baseline: the same app/camera *would* move for an admissible rect, so the
    // assertions below are not vacuous.
    {
        let (mut probe_app, mut probe_session) = application_with_document();
        zoom_window(
            &mut probe_app,
            &mut probe_session,
            [[100.0, 100.0], [600.0, 400.0]],
        )
        .unwrap();
        assert_ne!(viewport(&probe_app).camera, camera);
    }

    for rect in [
        [[100.0, 100.0], [100.0, 400.0]],
        [[100.0, 100.0], [600.0, 100.0]],
    ] {
        let error = zoom_window(&mut app, &mut session, rect)
            .expect_err(&format!("{rect:?} must be refused"));
        assert!(
            error.to_string().contains("zoom.window.degenerate_rect"),
            "{rect:?} must carry the named code: {error}"
        );
    }
    for rect in [
        [[f64::NAN, 100.0], [600.0, 400.0]],
        [[100.0, 100.0], [f64::INFINITY, 400.0]],
    ] {
        let error = zoom_window(&mut app, &mut session, rect)
            .expect_err(&format!("{rect:?} must be refused"));
        assert!(
            error.to_string().contains("zoom.window.invalid_rect"),
            "{rect:?} must carry the named code: {error}"
        );
    }
    assert_eq!(viewport(&app).camera, camera, "nothing moved");
    assert_eq!(
        viewport(&app).zoom_history_len(),
        steps,
        "a refusal plants no ZOOM PREVIOUS step"
    );
    assert_eq!(
        app.history_availability(&DocumentId(1)),
        HistoryAvailability::default()
    );
}

#[test]
fn the_corner_order_does_not_matter() {
    let rect = [[200.0, 150.0], [600.0, 450.0]];
    let (mut app, mut session) = application_with_document();
    zoom_window(&mut app, &mut session, rect).unwrap();
    let expected = viewport(&app).camera;

    for flipped in [
        [rect[1], rect[0]],
        [[rect[0][0], rect[1][1]], [rect[1][0], rect[0][1]]],
        [[rect[1][0], rect[1][1]], [rect[0][0], rect[0][1]]],
    ] {
        let (mut app, mut session) = application_with_document();
        zoom_window(&mut app, &mut session, flipped).unwrap();
        assert_eq!(
            viewport(&app).camera,
            expected,
            "corners {flipped:?} must frame the same rect"
        );
    }
}

#[test]
fn canonical_logical_rect_is_one_rule() {
    assert_eq!(
        canonical_logical_rect([[600.0, 450.0], [200.0, 150.0]]),
        Ok([[200.0, 150.0], [600.0, 450.0]])
    );
    assert_eq!(
        canonical_logical_rect([[200.0, 150.0], [600.0, 450.0]]),
        Ok([[200.0, 150.0], [600.0, 450.0]])
    );
    for refused in [
        [[0.0, 0.0], [0.0, 10.0]],
        [[0.0, 0.0], [10.0, 0.0]],
        [[0.0, 0.0], [0.0, 0.0]],
        [[f64::NAN, 0.0], [10.0, 10.0]],
        [[0.0, 0.0], [f64::NEG_INFINITY, 10.0]],
    ] {
        assert!(
            canonical_logical_rect(refused).is_err(),
            "{refused:?} must be refused"
        );
    }
    // `codes` also sees the success outcome shape (no diagnostics on a window).
    let (mut app, mut session) = application_with_document();
    let outcome = zoom_window(&mut app, &mut session, [[10.0, 10.0], [20.0, 20.0]]).unwrap();
    assert!(codes(&outcome).is_empty(), "{:?}", codes(&outcome));
}
