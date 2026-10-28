//! `ZOOM PREVIOUS`: the per-viewport bounded discrete-view stack.
//!
//! The contract pinned here: the pre-change snapshot of every **discrete** view
//! command (FIT, a zoom factor, `ZOOM WINDOW`, `STANDARDVIEW`, `SWITCH2D3D`,
//! `PROJECTION`, `RESETVIEW`), the exact restore (camera + 2D/3D mode + plan work
//! plane), the explicit divergence that pans / wheel zooms / orbit drags record
//! nothing, the cap of 16 with the oldest dropped, the non-error
//! `zoom.previous.empty` diagnostic, and that no view command ever creates a
//! document undo step.

use super::*;

fn viewport(app: &Application) -> &Viewport {
    &app.workspace.viewports[&ViewportId(1)]
}

fn steps(app: &Application) -> usize {
    viewport(app).zoom_history_len()
}

fn camera(app: &Application) -> Camera {
    viewport(app).camera
}

fn run(app: &mut Application, session: &mut SessionState, id: CommandId) {
    app.execute(session, command(id, CommandPayload::None))
        .unwrap_or_else(|error| panic!("{id:?} refused: {error:?}"));
}

fn zoom_by(app: &mut Application, session: &mut SessionState, factor: f64) {
    app.execute(
        session,
        command(
            CommandId::Zoom,
            CommandPayload::ZoomAt {
                factor,
                cursor: [400.0, 300.0],
            },
        ),
    )
    .unwrap();
}

fn zoom_previous(app: &mut Application, session: &mut SessionState) -> CommandOutcome {
    app.execute(
        session,
        command(CommandId::Zoom, CommandPayload::ZoomPrevious),
    )
    .expect("ZOOM PREVIOUS is never an error")
}

fn codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

/// A document spanning `(-100,-50)-(100,50)`, so FIT has a real extent.
fn app_with_extent() -> (Application, SessionState) {
    application_with_bounds(
        Point3 {
            x: -100.0,
            y: -50.0,
            z: 0.0,
        },
        Point3 {
            x: 100.0,
            y: 50.0,
            z: 0.0,
        },
    )
}

#[test]
fn a_fit_and_a_zoom_walk_back_step_by_step() {
    let (mut app, mut session) = app_with_extent();
    let start = camera(&app);

    run(&mut app, &mut session, CommandId::FitDrawing);
    let fitted = camera(&app);
    assert_eq!(steps(&app), 1, "the fit recorded the pre-fit view");

    zoom_by(&mut app, &mut session, 2.0);
    let zoomed = camera(&app);
    assert_eq!(steps(&app), 2, "the factor recorded the pre-zoom view");

    // First PREVIOUS: exactly the fitted view, and the 2D mode tracks it.
    assert_eq!(
        codes(&zoom_previous(&mut app, &mut session)),
        ["zoom.previous"]
    );
    assert_eq!(camera(&app), fitted);
    assert_eq!(view_mode(&app), ViewMode2d3d::TwoD { saved: fitted });
    assert_eq!(steps(&app), 1);

    // Second PREVIOUS: the untouched start view (bit-for-bit).
    zoom_previous(&mut app, &mut session);
    assert_eq!(camera(&app), start);
    assert_eq!(view_mode(&app), ViewMode2d3d::TwoD { saved: start });
    assert_eq!(steps(&app), 0);
    assert_ne!(camera(&app), zoomed);
}

fn view_mode(app: &Application) -> ViewMode2d3d {
    viewport(app).view_mode
}

#[test]
fn an_empty_history_is_a_named_diagnostic_and_changes_nothing() {
    let (mut app, mut session) = application_with_document();
    assert_eq!(steps(&app), 0);
    let before = camera(&app);
    let generation = session.generation;

    let outcome = zoom_previous(&mut app, &mut session);
    assert_eq!(codes(&outcome), ["zoom.previous.empty"]);
    assert_eq!(camera(&app), before, "an empty stack restores nothing");
    assert_eq!(session.generation, generation);
    assert_eq!(steps(&app), 0);
    assert_eq!(
        app.history_availability(&DocumentId(1)),
        HistoryAvailability::default(),
        "an empty PREVIOUS is not an undo step"
    );
}

#[test]
fn pans_and_orbit_drags_record_no_step() {
    let (mut app, mut session) = app_with_extent();
    run(&mut app, &mut session, CommandId::FitDrawing);
    let recorded = steps(&app);
    assert_eq!(recorded, 1);

    // A pan, a modal-pan replacement and a 2D/3D round trip are navigation, but
    // only the *discrete* switch records: the pans are the divergence.
    for delta in [
        Point3 {
            x: 15.0,
            y: -7.0,
            z: 0.0,
        },
        Point3 {
            x: -3.0,
            y: 2.0,
            z: 0.0,
        },
    ] {
        app.execute(
            &mut session,
            command(CommandId::Pan, CommandPayload::Points(vec![delta])),
        )
        .unwrap();
        assert_eq!(steps(&app), recorded, "a pan must not record a step");
    }

    // Enter 3D (one discrete step), then orbit: the orbit is a drag, not a view.
    run(&mut app, &mut session, CommandId::Switch2d3d);
    let after_switch = steps(&app);
    assert_eq!(after_switch, recorded + 1);
    for (yaw, pitch) in [(0.1, 0.05), (0.2, -0.1), (-0.3, 0.2)] {
        app.execute(
            &mut session,
            command(CommandId::Orbit, CommandPayload::Orbit { yaw, pitch }),
        )
        .unwrap();
        assert_eq!(steps(&app), after_switch, "an orbit must not record a step");
    }
}

#[test]
fn every_discrete_view_command_records_one_step() {
    let (mut app, mut session) = app_with_extent();

    let mut expected = 0;
    let check = |app: &Application, expected: usize, what: &str| {
        assert_eq!(steps(app), expected, "{what} must record exactly one step");
    };

    run(&mut app, &mut session, CommandId::FitDrawing);
    expected += 1;
    check(&app, expected, "FIT");

    zoom_by(&mut app, &mut session, 1.25);
    expected += 1;
    check(&app, expected, "ZOOM <factor>");

    app.execute(
        &mut session,
        command(
            CommandId::Zoom,
            CommandPayload::ZoomWindow {
                rect_logical: [[100.0, 100.0], [700.0, 500.0]],
            },
        ),
    )
    .unwrap();
    expected += 1;
    check(&app, expected, "ZOOM WINDOW");

    app.execute(
        &mut session,
        command(
            CommandId::StandardView,
            CommandPayload::StandardView(StandardView::Front),
        ),
    )
    .unwrap();
    expected += 1;
    check(&app, expected, "STANDARDVIEW (3D)");

    // The plan view while already in 3D is a real change (it restores the
    // snapshot), so it counts; the plan view while already in 2D does not.
    app.execute(
        &mut session,
        command(
            CommandId::StandardView,
            CommandPayload::StandardView(StandardView::Top),
        ),
    )
    .unwrap();
    expected += 1;
    check(&app, expected, "STANDARDVIEW (plan from 3D)");
    app.execute(
        &mut session,
        command(
            CommandId::StandardView,
            CommandPayload::StandardView(StandardView::Top),
        ),
    )
    .unwrap();
    check(&app, expected, "STANDARDVIEW (plan, already 2D: no-op)");

    run(&mut app, &mut session, CommandId::SwitchProjection);
    expected += 1;
    check(&app, expected, "PROJECTION");

    run(&mut app, &mut session, CommandId::ResetView);
    expected += 1;
    check(&app, expected, "RESETVIEW");

    run(&mut app, &mut session, CommandId::Switch2d3d);
    expected += 1;
    check(&app, expected, "SWITCH2D3D");
}

#[test]
fn a_three_d_switch_round_trip_is_restorable() {
    let (mut app, mut session) = app_with_extent();
    run(&mut app, &mut session, CommandId::FitDrawing);
    let plan = camera(&app);
    run(&mut app, &mut session, CommandId::Switch2d3d);
    assert_eq!(view_mode(&app).kind(), ProjectionKind::ThreeD);
    let three_d = camera(&app);

    zoom_previous(&mut app, &mut session);
    assert_eq!(camera(&app), plan, "the plan camera is restored exactly");
    assert_eq!(view_mode(&app), ViewMode2d3d::TwoD { saved: plan });
    assert_eq!(steps(&app), 1);

    // And the previous step is the pre-fit start view.
    zoom_previous(&mut app, &mut session);
    assert_eq!(view_mode(&app).kind(), ProjectionKind::TwoD);
    assert_ne!(camera(&app), three_d);
}

#[test]
fn the_history_is_capped_at_sixteen_and_drops_the_oldest() {
    let (mut app, mut session) = application_with_document();
    // A pre-scale so every factor is admissible.
    let mut cameras = vec![camera(&app)];
    for _ in 0..20 {
        zoom_by(&mut app, &mut session, 1.01);
        cameras.push(camera(&app));
    }
    assert_eq!(ZOOM_HISTORY_CAP, 16);
    assert_eq!(steps(&app), ZOOM_HISTORY_CAP, "the stack is bounded");

    // Pushing happens before change i, so after 20 changes the retained
    // snapshots are cameras[4..=19] (the four oldest were dropped).
    let retained: Vec<Camera> = viewport(&app)
        .zoom_history
        .snapshots()
        .copied()
        .map(|snapshot| snapshot.camera)
        .collect();
    assert_eq!(retained.len(), ZOOM_HISTORY_CAP);
    assert_eq!(retained.first(), Some(&cameras[4]));
    assert_eq!(retained.last(), Some(&cameras[19]));

    // Walking back consumes exactly the retained ones, newest first. The newest
    // snapshot is the pre-change view of the last zoom (`cameras[19]`), not the
    // view it produced (`cameras[20]`).
    for expected in cameras.iter().rev().skip(1).take(ZOOM_HISTORY_CAP) {
        zoom_previous(&mut app, &mut session);
        assert_eq!(&camera(&app), expected);
    }
    assert_eq!(steps(&app), 0);
    assert_eq!(
        codes(&zoom_previous(&mut app, &mut session)),
        ["zoom.previous.empty"]
    );
}

#[test]
fn view_commands_never_create_a_document_undo_step() {
    let (mut app, mut session) = app_with_extent();
    let revision = app.workspace.documents[&DocumentId(1)].drawing.revision();

    run(&mut app, &mut session, CommandId::FitDrawing);
    zoom_by(&mut app, &mut session, 2.0);
    app.execute(
        &mut session,
        command(
            CommandId::Zoom,
            CommandPayload::ZoomWindow {
                rect_logical: [[100.0, 100.0], [700.0, 500.0]],
            },
        ),
    )
    .unwrap();
    zoom_previous(&mut app, &mut session);
    zoom_previous(&mut app, &mut session);
    zoom_previous(&mut app, &mut session);
    run(&mut app, &mut session, CommandId::ResetView);

    assert_eq!(
        app.history_availability(&DocumentId(1)),
        HistoryAvailability::default(),
        "no view command may record a document undo/redo step"
    );
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        revision,
        "no view command may bump the database revision"
    );
}

#[test]
fn the_work_plane_is_restored_with_the_view() {
    let (mut app, mut session) = app_with_extent();
    let mut plane = viewport(&app).work_plane;
    plane.origin.z = 42.0;
    app.workspace
        .viewports
        .get_mut(&ViewportId(1))
        .unwrap()
        .work_plane = plane;

    run(&mut app, &mut session, CommandId::ResetView);
    assert_eq!(viewport(&app).work_plane.origin.z, 0.0);
    zoom_previous(&mut app, &mut session);
    assert_eq!(
        viewport(&app).work_plane.origin.z,
        42.0,
        "PREVIOUS restores the plan work plane RESETVIEW replaced"
    );
}
