use super::*;

// --- S1 click-select: SelectAt → pick_at_screen → flags/selection ------------

#[test]
fn a_click_selects_through_select_at_mode_and_reaches_the_entity_flags_input() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let expected = picked_ref_at(&core, pixel);
    let camera_before = active_camera(&core);

    let selected = click_select_at(&core, pixel);
    assert_eq!(
        selected, expected,
        "the click must select the pick_at_screen result, never an invented one"
    );
    assert_eq!(
        active_camera(&core),
        camera_before,
        "a press/release click must not navigate"
    );
    assert!(
        core.render.overlays().selection.contains(&expected),
        "the selection must reach the overlay input that yields FLAG_SELECTED"
    );
    assert!(
        matches!(core.controller.borrow().session.tool, ToolState::Selecting),
        "a successful select enters the selecting tool state"
    );

    // The published host state reports the same selection for the properties
    // panel: one object, with its real identity rows.
    let full = EngineClient::take_updates(&core)
        .unwrap()
        .full
        .expect("the select marks the full state dirty");
    assert_eq!(full.selection_properties.count, 1);
    assert!(
        full.selection_properties
            .rows
            .iter()
            .any(|row| row.key == "id"),
        "the published rows are the real property rows"
    );
    assert!(!full.selection_properties.empty);
}

#[test]
fn a_click_on_empty_space_clears_the_selection_without_navigating() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let selected = click_select_at(&core, pixel);
    assert!(core.render.overlays().selection.contains(&selected));

    // A corner far from every entity really picks nothing.
    let corner = [1.0, 1.0];
    assert!(
        picks_nothing(&core, corner),
        "the chosen corner must really be empty space"
    );
    let revision = core.request_context().metrics_revision.unwrap();
    dispatch_pointer(
        &core,
        InteractionRequest::SelectAt {
            point: PointInput::CanvasLocal {
                logical_position: corner,
                metrics_revision: revision,
            },
            mode: SelectionMode::Replace,
        },
    );
    assert!(
        core.controller.borrow().session.selection.is_empty(),
        "a replace-select on empty space clears the selection"
    );
    assert!(
        core.render.overlays().selection.is_empty(),
        "the flags input must be cleared, not stale"
    );
}

#[test]
fn a_primary_drag_beyond_the_threshold_boxes_and_never_pans() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let start = screen_of(&core, demo_circle_rim(&core));
    let end = [
        start[0] + cad_app::DRAG_THRESHOLD_LOGICAL_PX + 30.0,
        start[1] + cad_app::DRAG_THRESHOLD_LOGICAL_PX + 30.0,
    ];
    let camera_before = active_camera(&core);
    let revision = core.request_context().metrics_revision.unwrap();

    // Exactly the sequence the session emits for a drag: press, move past the
    // threshold, release — with no SelectAt.
    dispatch_pointer(
        &core,
        mouse(1, cad_app::interaction::PointerPhase::Down, start, revision),
    );
    dispatch_pointer(
        &core,
        mouse(1, cad_app::interaction::PointerPhase::Move, end, revision),
    );
    // A 2D primary drag is a box selection (ADR 0014 B.2), never a pan.
    assert!(
        core.navigation.box_gesture.borrow().is_some(),
        "a primary drag beyond the threshold starts a box gesture"
    );
    dispatch_pointer(
        &core,
        mouse(1, cad_app::interaction::PointerPhase::Up, end, revision),
    );

    assert!(
        core.render.overlays().selection_box.is_none(),
        "the rubber band is cleared on release"
    );
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "the box gesture is finished on release"
    );
    assert_eq!(
        active_camera(&core),
        camera_before,
        "a primary drag must not pan the view any more"
    );
    assert!(
        !matches!(core.controller.borrow().session.tool, ToolState::Panning),
        "a box gesture never enters the pan tool state"
    );
}

#[test]
fn a_middle_button_drag_pans_and_never_selects() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let start = screen_of(&core, demo_circle_rim(&core));
    let end = [start[0], start[1] + 40.0];
    let camera_before = active_camera(&core);
    let revision = core.request_context().metrics_revision.unwrap();

    dispatch_pointer(
        &core,
        pointer_with_button(
            1,
            cad_app::interaction::PointerPhase::Down,
            start,
            revision,
            PointerButton::Middle,
        ),
    );
    dispatch_pointer(
        &core,
        pointer_with_button(
            1,
            cad_app::interaction::PointerPhase::Move,
            end,
            revision,
            PointerButton::Middle,
        ),
    );
    dispatch_pointer(
        &core,
        pointer_with_button(
            1,
            cad_app::interaction::PointerPhase::Up,
            end,
            revision,
            PointerButton::Middle,
        ),
    );

    assert!(
        core.controller.borrow().session.selection.is_empty(),
        "the middle button is a pan gesture, never a selection"
    );
    assert_ne!(
        active_camera(&core),
        camera_before,
        "a middle-button drag pans the view"
    );
}

#[test]
fn a_select_at_with_a_stale_metrics_revision_is_refused_explicitly() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let revision = client.request_context().metrics_revision.unwrap();
    // The canvas really changed: the revision a coordinate was tagged with is no
    // longer the host's current one.
    core.set_canvas_metrics(CanvasMetrics::new([0.0; 2], [1000.0, 800.0], 1.0))
        .unwrap();
    let current = client.request_context().metrics_revision.unwrap();
    assert!(
        current > revision,
        "the metrics change advances the revision"
    );
    let selection_before = core.controller.borrow().session.selection.refs().to_vec();

    let request = |metrics_revision: u64| {
        EngineRequest::Interaction(InteractionRequest::SelectAt {
            point: PointInput::CanvasLocal {
                logical_position: [100.0, 100.0],
                metrics_revision,
            },
            mode: SelectionMode::Replace,
        })
    };

    // The synchronous path refuses it with its stable, explicit reason.
    let error = client.dispatch(request(revision)).unwrap_err();
    assert!(
        matches!(
            error,
            RequestError::Execution(CadError::InvalidInput(ref reason))
                if reason == "stale canvas metrics revision"
        ),
        "a stale canvas coordinate must be an explicit refusal, got {error:?}"
    );
    assert_eq!(
        core.controller.borrow().session.selection.refs(),
        selection_before,
        "a refused selection must change nothing"
    );

    // The queued path grades it stale as well: never a silent success.
    client
        .submit(client.request_context(), request(revision))
        .unwrap();
    client.pump().unwrap();
    let completions = client.poll_completions(4);
    assert_eq!(completions.len(), 1);
    assert!(
        matches!(completions[0].result, Err(RequestError::StaleContext)),
        "a queued stale select must be graded stale, got {:?}",
        completions[0].result
    );
    assert_eq!(
        core.controller.borrow().session.selection.refs(),
        selection_before,
        "neither path may mutate the selection"
    );
}
