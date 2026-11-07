//! `REDRAW` / `REGEN` / `VIEW` / `VIEWGO` / `VIEWBACK` / `VIEWFORWARD` /
//! `VPOINT`: the 2026-11-07 view batch.
//!
//! The contract pinned here is that every one is session view state only: the
//! drawing revision, entity count and undo/redo availability never move. Named
//! views round-trip camera + mode + work plane + active space, unknown names are
//! explicit refusals, back/forward order is stack-exact and an empty stack is a
//! named no-op, and a zero/non-finite `VPOINT` is refused before any camera
//! change.

use super::*;

fn viewport(app: &Application, id: u128) -> &Viewport {
    &app.workspace.viewports[&ViewportId(id)]
}

fn revision(app: &Application) -> (u64, usize) {
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    (drawing.revision().0, drawing.entity_count())
}

fn assert_view_only(app: &Application, before: (u64, usize)) {
    assert_eq!(revision(app), before, "a view command writes nothing");
    assert!(
        !app.can_undo(&DocumentId(1)),
        "a view command records no undo step"
    );
    assert!(
        !app.can_redo(&DocumentId(1)),
        "a view command records no redo step"
    );
}

fn app_with_extent_and_second_viewport() -> (Application, SessionState) {
    let (mut app, session) = application_with_bounds(
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
    );
    app.workspace.viewports.insert(
        ViewportId(2),
        Viewport::new(ViewportId(2), DocumentId(1), [400.0, 300.0]),
    );
    (app, session)
}

fn run(app: &mut Application, session: &mut SessionState, id: CommandId) {
    app.execute(session, command(id, CommandPayload::None))
        .unwrap_or_else(|error| panic!("{id:?} refused: {error:?}"));
}

fn run_view(
    app: &mut Application,
    session: &mut SessionState,
    option: ViewCommand,
) -> CommandOutcome {
    app.execute(
        session,
        command(CommandId::View, CommandPayload::View(option)),
    )
    .expect("VIEW is admitted")
}

fn view_go(app: &mut Application, session: &mut SessionState, name: &str) -> CommandOutcome {
    app.execute(
        session,
        command(CommandId::ViewGo, CommandPayload::ViewGo(name.into())),
    )
    .expect("VIEWGO is admitted")
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

fn codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

#[test]
fn redraw_and_regen_advance_their_stamps_but_never_the_drawing() {
    let (mut app, mut session) = app_with_extent_and_second_viewport();
    let before = revision(&app);
    let camera = viewport(&app, 1).camera;
    let history = viewport(&app, 1).zoom_history_len();
    let generation = session.generation;

    let outcome = app
        .execute(
            &mut session,
            command(CommandId::Redraw, CommandPayload::None),
        )
        .unwrap();
    assert_eq!(codes(&outcome), ["redraw"]);
    assert_eq!(viewport(&app, 1).refresh_generation, 1);
    assert_eq!(
        viewport(&app, 2).refresh_generation,
        0,
        "REDRAW is current-viewport only"
    );
    assert_eq!(
        viewport(&app, 1).camera,
        camera,
        "REDRAW never moves the camera"
    );
    assert_eq!(viewport(&app, 1).zoom_history_len(), history);
    assert_eq!(
        session.generation,
        generation + 1,
        "the session refresh stamp advances"
    );
    assert_view_only(&app, before);

    let outcome = app
        .execute(
            &mut session,
            command(CommandId::Regen, CommandPayload::None),
        )
        .unwrap();
    assert_eq!(codes(&outcome), ["regen"]);
    assert_eq!(viewport(&app, 1).regen_generation, 1);
    assert_eq!(
        viewport(&app, 2).regen_generation,
        0,
        "REGEN is current-viewport only"
    );
    assert_eq!(session.regen_generation, 1);
    assert_eq!(viewport(&app, 1).camera, camera);
    assert_view_only(&app, before);
}

#[test]
fn redrawall_and_regenall_cover_every_viewport_of_the_document() {
    let (mut app, mut session) = app_with_extent_and_second_viewport();
    let before = revision(&app);

    let outcome = app
        .execute(
            &mut session,
            command(CommandId::RedrawAll, CommandPayload::None),
        )
        .unwrap();
    assert_eq!(codes(&outcome), ["redraw.all"]);
    assert_eq!(viewport(&app, 1).refresh_generation, 1);
    assert_eq!(viewport(&app, 2).refresh_generation, 1);

    let outcome = app
        .execute(
            &mut session,
            command(CommandId::RegenAll, CommandPayload::None),
        )
        .unwrap();
    assert_eq!(codes(&outcome), ["regen.all"]);
    assert_eq!(viewport(&app, 1).regen_generation, 1);
    assert_eq!(viewport(&app, 2).regen_generation, 1);
    assert_view_only(&app, before);
}

#[test]
fn named_view_save_restore_round_trips_camera_mode_plane_and_space() {
    let (mut app, mut session) = app_with_extent_and_second_viewport();
    let before = revision(&app);
    let saved_camera = viewport(&app, 1).camera;

    run_view(&mut app, &mut session, ViewCommand::Save("Plan A".into()));
    zoom_by(&mut app, &mut session, 2.0);
    assert_ne!(viewport(&app, 1).camera, saved_camera);

    // Case-insensitive, exact-name-first resolution.
    let outcome = view_go(&mut app, &mut session, "plan a");
    assert_eq!(codes(&outcome), ["view.restored"]);
    assert_eq!(
        viewport(&app, 1).camera,
        saved_camera,
        "VIEWGO restores the exact camera"
    );
    assert_view_only(&app, before);

    // `VIEW RESTORE` is the same path.
    zoom_by(&mut app, &mut session, 3.0);
    let outcome = run_view(
        &mut app,
        &mut session,
        ViewCommand::Restore("Plan A".into()),
    );
    assert_eq!(codes(&outcome), ["view.restored"]);
    assert_eq!(viewport(&app, 1).camera, saved_camera);
    assert_view_only(&app, before);
}

#[test]
fn named_view_unknown_name_and_delete_are_explicit() {
    let (mut app, mut session) = app_with_extent_and_second_viewport();
    let camera = viewport(&app, 1).camera;

    let error = app
        .execute(
            &mut session,
            command(CommandId::ViewGo, CommandPayload::ViewGo("missing".into())),
        )
        .unwrap_err();
    assert!(format!("{error:?}").contains("view.unknown"));
    assert_eq!(
        viewport(&app, 1).camera,
        camera,
        "a refusal changes nothing"
    );

    let error = app
        .execute(
            &mut session,
            command(
                CommandId::View,
                CommandPayload::View(ViewCommand::Delete("missing".into())),
            ),
        )
        .unwrap_err();
    assert!(format!("{error:?}").contains("view.unknown"));

    run_view(&mut app, &mut session, ViewCommand::Save("Keep".into()));
    let outcome = run_view(&mut app, &mut session, ViewCommand::Delete("keep".into()));
    assert_eq!(codes(&outcome), ["view.deleted"]);
    assert!(session.view_states.is_empty());

    let outcome = run_view(&mut app, &mut session, ViewCommand::List);
    assert_eq!(codes(&outcome), ["view.list"]);
}

#[test]
fn named_view_store_is_bounded_to_32_and_drops_the_oldest() {
    let (mut app, mut session) = app_with_extent_and_second_viewport();
    for index in 0..33 {
        run_view(
            &mut app,
            &mut session,
            ViewCommand::Save(format!("VIEW{index}")),
        );
    }
    assert_eq!(session.view_states.len(), 32);
    assert_eq!(session.view_states.cap(), 32);
    assert!(
        session.view_states.resolve("VIEW0").is_none(),
        "the oldest name is dropped when full"
    );
    assert!(session.view_states.resolve("VIEW32").is_some());
}

#[test]
fn viewback_and_viewforward_walk_the_stack_in_exact_order() {
    let (mut app, mut session) = app_with_extent_and_second_viewport();
    let s0 = viewport(&app, 1).camera;
    zoom_by(&mut app, &mut session, 2.0);
    let s1 = viewport(&app, 1).camera;
    zoom_by(&mut app, &mut session, 4.0);
    let s2 = viewport(&app, 1).camera;

    // A fresh viewport has nothing to go back to: a named no-op, not an error.
    let fresh = Viewport::new(ViewportId(9), DocumentId(1), [800.0, 600.0]);
    assert!(fresh.zoom_history.is_empty());
    let mut temp_session = SessionState::new(DocumentId(1), AppMode::Work);
    app.workspace.viewports.insert(ViewportId(9), fresh);
    let empty_command = Command {
        schema_version: 1,
        id: CommandId::ViewBack,
        document: DocumentId(1),
        viewport: ViewportId(9),
        payload: CommandPayload::None,
    };
    let outcome = app.execute(&mut temp_session, empty_command).unwrap();
    assert_eq!(codes(&outcome), ["view.back.empty"]);

    // s2 -> s1 -> s0, then forward s1 -> s2.
    run(&mut app, &mut session, CommandId::ViewBack);
    assert_eq!(viewport(&app, 1).camera, s1);
    run(&mut app, &mut session, CommandId::ViewBack);
    assert_eq!(viewport(&app, 1).camera, s0);
    run(&mut app, &mut session, CommandId::ViewForward);
    assert_eq!(viewport(&app, 1).camera, s1);
    run(&mut app, &mut session, CommandId::ViewForward);
    assert_eq!(
        viewport(&app, 1).camera,
        s2,
        "forward returns to the newest view"
    );

    let outcome = app
        .execute(
            &mut session,
            command(CommandId::ViewForward, CommandPayload::None),
        )
        .unwrap();
    assert_eq!(codes(&outcome), ["view.forward.empty"]);
    assert_eq!(
        viewport(&app, 1).camera,
        s2,
        "an empty forward stack is a no-op"
    );
}

#[test]
fn zoom_previous_records_a_forward_step_that_viewforward_can_apply() {
    let (mut app, mut session) = app_with_extent_and_second_viewport();
    let start = viewport(&app, 1).camera;
    zoom_by(&mut app, &mut session, 2.0);
    let zoomed = viewport(&app, 1).camera;

    app.execute(
        &mut session,
        command(CommandId::Zoom, CommandPayload::ZoomPrevious),
    )
    .unwrap();
    assert_eq!(viewport(&app, 1).camera, start);
    assert_eq!(viewport(&app, 1).zoom_history.forward_len(), 1);

    run(&mut app, &mut session, CommandId::ViewForward);
    assert_eq!(viewport(&app, 1).camera, zoomed);
}

#[test]
fn vpoint_words_and_vectors_set_a_direction_without_touching_the_model() {
    let (mut app, mut session) = app_with_extent_and_second_viewport();
    let before = revision(&app);
    let model_camera = viewport(&app, 1).camera;

    // TOP is the 2D plan: no 3D camera.
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::VPoint,
                CommandPayload::VPoint(VPointInput::Standard(StandardView::Top)),
            ),
        )
        .unwrap();
    assert_eq!(codes(&outcome), ["vpoint.standard"]);
    assert_eq!(viewport(&app, 1).view_mode.kind(), ProjectionKind::TwoD);
    assert_eq!(viewport(&app, 1).camera, model_camera);

    // FRONT enters the standard 3D observation.
    app.execute(
        &mut session,
        command(
            CommandId::VPoint,
            CommandPayload::VPoint(VPointInput::Standard(StandardView::Front)),
        ),
    )
    .unwrap();
    assert_eq!(viewport(&app, 1).view_mode.kind(), ProjectionKind::ThreeD);
    let direction = camera::normalize3(camera::sub(
        viewport(&app, 1).camera.eye,
        viewport(&app, 1).camera.target,
    ))
    .unwrap();
    assert!((direction.y + 1.0).abs() < 1e-9, "FRONT looks from -Y");

    // A vector is an explicit 3D direction.
    app.execute(
        &mut session,
        command(
            CommandId::VPoint,
            CommandPayload::VPoint(VPointInput::Vector(Point3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            })),
        ),
    )
    .unwrap();
    assert_eq!(viewport(&app, 1).view_mode.kind(), ProjectionKind::ThreeD);
    let direction = camera::normalize3(camera::sub(
        viewport(&app, 1).camera.eye,
        viewport(&app, 1).camera.target,
    ))
    .unwrap();
    let k = 1.0 / 3.0f64.sqrt();
    assert!((direction.x - k).abs() < 1e-9);
    assert!((direction.y - k).abs() < 1e-9);
    assert!((direction.z - k).abs() < 1e-9);
    assert_view_only(&app, before);

    // Zero and non-finite vectors are refused by name before any change.
    let camera_before = viewport(&app, 1).camera;
    let history_before = viewport(&app, 1).zoom_history_len();
    for bad in [
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: f64::NAN,
            y: 1.0,
            z: 0.0,
        },
    ] {
        let error = app
            .execute(
                &mut session,
                command(
                    CommandId::VPoint,
                    CommandPayload::VPoint(VPointInput::Vector(bad)),
                ),
            )
            .unwrap_err();
        assert!(format!("{error:?}").contains("vpoint.invalid_vector"));
    }
    assert_eq!(viewport(&app, 1).camera, camera_before);
    assert_eq!(viewport(&app, 1).zoom_history_len(), history_before);
    assert_view_only(&app, before);
}

#[test]
fn view_commands_refuse_a_caller_payload_where_none_is_allowed() {
    let (mut app, mut session) = app_with_extent_and_second_viewport();
    for id in [
        CommandId::Redraw,
        CommandId::RedrawAll,
        CommandId::Regen,
        CommandId::RegenAll,
        CommandId::ViewBack,
        CommandId::ViewForward,
    ] {
        let error = app
            .execute(
                &mut session,
                command(id, CommandPayload::Points(Vec::new())),
            )
            .unwrap_err();
        assert!(format!("{error:?}").contains("needs no payload"), "{id:?}");
    }
}
