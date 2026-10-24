use super::*;

// ---------------------------------------------------------------------------
// Camera-settle deferral (PURE-STATE, no GPU): a camera-only navigation gesture
// (pan/zoom/rotate) defers the reference-grid re-prepare and commits **once** when
// it settles. Non-camera inputs (selection/visibility/undo/geometry/document
// switch/resize) keep today's immediate path and clear the pending refresh.
// ---------------------------------------------------------------------------

#[test]
fn camera_gesture_defers_the_grid_refresh_and_commits_once_on_settle() {
    let session = session_with_published_base();
    session.set_grid_settle_timeout(NO_SETTLE);
    let before = session.diagnostics();
    let scene_before = session.current_prepared_scene().expect("published scene");
    let suffix_before = session.overlay_suffix_commits();
    let view_before = session.view_revision();

    // A pan + zoom gesture: several camera-only steps, all deferred.
    for step in 1..=4 {
        pan_step(&session, 60.0 * step as f64);
        assert!(
            session.grid_refresh_pending(),
            "pan step {step} arms the deferred refresh"
        );
        zoom_step(&session, 1.0 / 1.1);
    }

    assert!(
        session.diagnostics().requested_token == before.requested_token,
        "0 in-gesture content advances"
    );
    assert_eq!(
        session.diagnostics().input_revision,
        before.input_revision,
        "camera-only navigation must not bump the scene identity"
    );
    assert_eq!(
        session.overlay_suffix_commits(),
        suffix_before,
        "no overlay-suffix churn during the deferred gesture"
    );
    assert!(
        session.view_revision() > view_before,
        "every camera step still advances the view"
    );
    assert!(
        session.preparation_current(),
        "the published scene keeps its grid extent while deferred"
    );
    assert_eq!(
        session.current_prepared_scene().expect("ready").revision,
        scene_before.revision,
        "nothing is republished during the gesture"
    );

    settle_camera_refresh(&session);
    assert_eq!(
        session.diagnostics().requested_token,
        before.requested_token + 1,
        "settling advances the content revision exactly once"
    );
    assert_eq!(
        session.diagnostics().input_revision,
        before.input_revision,
        "still camera-only: the identity never moved"
    );
}

#[test]
fn single_camera_change_still_commits_within_the_settle_timeout() {
    let session = session_with_published_base();
    let timeout = Duration::from_millis(20);
    session.set_grid_settle_timeout(timeout);
    assert_eq!(
        session.grid_settle_timeout(),
        timeout,
        "the settle timeout is the one that was set"
    );
    let before = session.diagnostics();

    pan_step(&session, 25.0);
    assert!(session.grid_refresh_pending());

    let deadline = Instant::now() + Duration::from_secs(5);
    while session.grid_refresh_pending() {
        assert!(
            Instant::now() < deadline,
            "a single camera change must commit once the timeout elapses"
        );
        session.poll_preparation().unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        session.diagnostics().requested_token,
        before.requested_token + 1,
        "a single camera change commits exactly once"
    );
    assert!(!session.grid_refresh_pending());
}

#[test]
fn zero_settle_timeout_is_the_kill_switch_for_the_immediate_path() {
    let session = session_with_published_base();
    session.set_grid_settle_timeout(Duration::ZERO);
    let before = session.diagnostics();

    pan_step(&session, 30.0);
    assert!(
        !session.grid_refresh_pending(),
        "the zero timeout disables deferral entirely"
    );
    assert_eq!(
        session.diagnostics().requested_token,
        before.requested_token + 1,
        "the kill-switch keeps today's immediate content advance"
    );
}

#[test]
fn resize_takes_the_immediate_path_and_clears_the_pending_refresh() {
    let session = session_with_published_base();
    session.set_grid_settle_timeout(NO_SETTLE);
    pan_step(&session, 100.0);
    assert!(session.grid_refresh_pending());
    let armed = session.diagnostics();

    let mut resized = session.view_snapshot();
    resized.canvas_size = [resized.canvas_size[0] * 0.5, resized.canvas_size[1] * 0.5];
    session.apply_view_snapshot(resized).unwrap();

    assert!(
        !session.grid_refresh_pending(),
        "a resize must clear the deferred refresh, never swallow it"
    );
    assert_eq!(
        session.diagnostics().requested_token,
        armed.requested_token + 1,
        "a resize advances the content revision immediately"
    );
    assert_eq!(
        session.diagnostics().input_revision,
        armed.input_revision,
        "a resize does not invalidate the published base identity"
    );
    assert_eq!(
        session.diagnostics().ready_base_revision,
        armed.ready_base_revision,
        "the published base is untouched by a resize"
    );
}

/// A whole-entity ref used for flags-only assertions (works on wasm too, unlike
/// the GPU-only `model_entity`).
fn flags_ref(id: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(0),
        entity: EntityId(id),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

#[test]
fn flags_only_selection_hover_and_hidden_never_defer_or_invalidate() {
    let session = session_with_published_base();
    session.set_grid_settle_timeout(NO_SETTLE);
    pan_step(&session, 80.0);
    assert!(session.grid_refresh_pending());
    let armed = session.diagnostics();
    let scene_before = session.current_prepared_scene().expect("published scene");
    let suffix_before = session.overlay_suffix_commits();
    assert!(
        session.preparation_current(),
        "an armed camera settle keeps the published scene current"
    );

    // Whole-entity selection is the flags path: it changes no overlay batch, so
    // it must not advance the scene identity or the preparation token, must not
    // drop the pending camera settle, and must not touch the published scene.
    assert!(session.set_selection(SelectionSet::from_refs([flags_ref(1)])));
    let after_selection = session.diagnostics();
    assert_eq!(
        after_selection.input_revision, armed.input_revision,
        "a flags-only selection must not bump the scene identity"
    );
    assert_eq!(
        after_selection.requested_token, armed.requested_token,
        "a flags-only selection must not schedule a worker preparation"
    );
    assert_eq!(
        after_selection.view_revision, armed.view_revision,
        "a flags-only selection does not touch the view"
    );
    assert!(
        session.grid_refresh_pending(),
        "a flags-only selection must not drop a pending camera settle"
    );
    assert_eq!(
        session.current_prepared_scene().expect("scene").revision,
        scene_before.revision,
        "a flags-only selection must not republish the scene"
    );
    assert!(
        Arc::ptr_eq(
            &scene_before.highlight,
            &session.current_prepared_scene().expect("scene").highlight
        ),
        "a flags-only selection must reuse the published overlay geometry, not rebuild it"
    );
    assert_eq!(
        session.overlay_suffix_commits(),
        suffix_before,
        "a flags-only selection must not truncate the overlay suffix"
    );

    // Hover and hidden are the same flags-only path.
    let mut hovered = session.overlays();
    hovered.hovered = SelectionSet::from_refs([flags_ref(2)]);
    assert!(session.set_overlays(hovered));
    assert_eq!(session.diagnostics().input_revision, armed.input_revision);
    assert_eq!(session.diagnostics().requested_token, armed.requested_token);
    assert!(session.grid_refresh_pending());
    let mut hidden = session.overlays();
    hidden.hidden = SelectionSet::from_refs([flags_ref(3)]);
    assert!(session.set_overlays(hidden));
    assert_eq!(session.diagnostics().input_revision, armed.input_revision);
    assert_eq!(session.diagnostics().requested_token, armed.requested_token);
    assert!(session.grid_refresh_pending());

    // A visible reference-grid toggle is a real overlay-batch change: today's
    // immediate path, which clears the deferred refresh.
    let mut grid = session.overlays();
    grid.visibility.grid = !grid.visibility.grid;
    assert!(session.set_overlays(grid));
    assert!(
        !session.grid_refresh_pending(),
        "a visibility change clears the deferred refresh"
    );
    assert_eq!(
        session.diagnostics().input_revision,
        armed.input_revision + 1,
        "a visibility change bumps the scene identity immediately"
    );
    assert_eq!(
        session.diagnostics().requested_token,
        armed.requested_token + 1,
        "a visibility change schedules a preparation immediately"
    );
}

#[test]
fn box_selection_is_a_visual_overlay_change_not_bounds_or_flags_only() {
    let session = session_with_published_base();
    session.set_grid_settle_timeout(NO_SETTLE);
    pan_step(&session, 80.0);
    assert!(session.grid_refresh_pending());
    let armed = session.diagnostics();
    let base_before = session
        .current_prepared_scene()
        .expect("published scene")
        .base
        .clone();

    // A box-only change: same camera/bounds, same entity flags, only the live
    // rubber band moves. Its fingerprint is a visual-overlay input, so it must
    // take the immediate `advance_input` path and never the bounds-only one.
    let mut overlays = session.overlays();
    overlays.selection_box = Some(cad_app::render_scene::BoxSelectionPreview {
        segments: vec![
            [p(0.0, 0.0), p(10.0, 0.0)],
            [p(10.0, 0.0), p(10.0, 10.0)],
            [p(10.0, 10.0), p(0.0, 10.0)],
            [p(0.0, 10.0), p(0.0, 0.0)],
        ],
        crossing: false,
    });
    assert!(session.set_overlays(overlays));
    let after = session.diagnostics();
    assert_eq!(
        after.input_revision,
        armed.input_revision + 1,
        "a box-only change is a visual (scene-identity) change, not flags-only"
    );
    assert_eq!(
        after.requested_token,
        armed.requested_token + 1,
        "a visual change schedules a preparation immediately"
    );
    assert!(
        !session.grid_refresh_pending(),
        "the immediate visual path clears the deferred refresh"
    );
    assert!(
        session.current_prepared_scene().is_none(),
        "the bounds-only path keeps the published scene current; a visual change must not"
    );

    // The rebuild touches only the overlay: the base `Arc` is reused and the box
    // is the new highlight batch at its own draw order.
    session.prepare().unwrap();
    let after_prepare = session.controller_ready().expect("prepared scene");
    assert!(Arc::ptr_eq(&base_before, &after_prepare.base));
    let box_batches: Vec<_> = after_prepare
        .highlight
        .added
        .iter()
        .filter(|batch| batch.draw_order == cad_app::render_scene::BOX_SELECT_DRAW_ORDER)
        .collect();
    assert_eq!(box_batches.len(), 1, "one box-selection batch");
    assert_eq!(
        box_batches[0].color,
        cad_app::render_scene::BOX_SELECT_WINDOW_COLOR
    );
}

#[test]
fn hover_after_submission_is_not_regressed_by_the_published_scene() {
    let session = session_with_published_base();
    // A live ref resolved from the published base, so the flags actually resolve.
    let reference = session
        .current_prepared_scene()
        .expect("published scene")
        .base
        .added
        .iter()
        .flat_map(|batch| batch.sources.iter())
        .find(|reference| reference.sub_element.is_none())
        .cloned()
        .expect("a resident whole-entity ref");

    // Submit a preparation, then move the hover before it lands (the worker
    // result carries the pre-hover flags).
    session.set_grid_settle_timeout(NO_SETTLE);
    pan_step(&session, 120.0);
    session.set_grid_settle_timeout(Duration::ZERO);
    session.poll_preparation().unwrap();
    let mut overlays = session.overlays();
    overlays.hovered = SelectionSet::from_refs([reference.clone()]);
    assert!(session.set_overlays(overlays));

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        session.poll_preparation().unwrap();
        let diagnostics = session.diagnostics();
        if diagnostics.prepared_token == Some(diagnostics.requested_token) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the publication after the hover change timed out"
        );
        std::thread::yield_now();
    }

    let ready = session.current_prepared_scene().expect("published scene");
    assert!(
        ready
            .entity_flags
            .iter()
            .any(|(flagged, bits)| { flagged.entity == reference.entity && *bits != 0 }),
        "the publication boundary must use the LATEST hover, not the submitted one"
    );
}

#[test]
fn document_switch_and_geometry_edits_are_immediate_and_clear_pending() {
    let session = session_with_published_base();
    session.set_grid_settle_timeout(NO_SETTLE);
    pan_step(&session, 90.0);
    assert!(session.grid_refresh_pending());
    let armed = session.diagnostics();

    // A geometry edit (a new drawing `Arc`, as an undo/edit produces) is immediate.
    let other = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    assert!(session.sync_drawing(Some(other)));
    assert!(
        !session.grid_refresh_pending(),
        "a geometry/document change clears the deferred refresh"
    );
    assert!(
        session.diagnostics().input_revision > armed.input_revision,
        "a geometry edit bumps the scene identity immediately"
    );

    // A document switch through the view snapshot is the same immediate path.
    pan_step(&session, 90.0);
    assert!(session.grid_refresh_pending());
    let armed = session.diagnostics();
    let mut switched = session.view_snapshot();
    switched.document = DocumentId(99);
    assert!(session.apply_view_snapshot(switched).unwrap());
    assert!(
        !session.grid_refresh_pending(),
        "a document switch clears the deferred refresh"
    );
    assert!(
        session.diagnostics().input_revision > armed.input_revision,
        "a document switch bumps the scene identity immediately"
    );
}

#[test]
fn settled_camera_refresh_publishes_the_final_viewport_bounds() {
    let session = session_with_published_base();
    session.set_grid_settle_timeout(NO_SETTLE);
    let before = session.current_prepared_scene().expect("published scene");

    for step in 1..=3 {
        pan_step(&session, 200.0 * step as f64);
    }
    let final_bounds = viewport_world_bounds(&session.view_snapshot());
    session.set_grid_settle_timeout(Duration::ZERO);

    let deadline = Instant::now() + Duration::from_secs(5);
    let refined = loop {
        session.poll_preparation().unwrap();
        let ready = session.current_prepared_scene().expect("ready scene");
        if ready.revision > before.revision {
            break ready;
        }
        assert!(
            Instant::now() < deadline,
            "the settled grid refinement timed out"
        );
        std::thread::yield_now();
    };

    assert_eq!(
        session.overlays().viewport_bounds,
        final_bounds,
        "the settle commit must use the FINAL viewport bounds, never an intermediate one"
    );
    assert_eq!(
        refined.base_revision, before.base_revision,
        "the settle must reuse the published base"
    );
    assert_ne!(
        refined.highlight.added, before.highlight.added,
        "the reference grid must reflect the settled viewport"
    );
    assert!(!session.grid_refresh_pending());
}

#[test]
fn settle_during_an_inflight_prepare_publishes_exactly_once_at_final_bounds() {
    let session = session_with_published_base();
    let base_before = session.diagnostics().ready_base_revision;
    let publications_before = session.diagnostics().scene_publication_revision;

    // Gesture 1: arm, then settle and start the preparation for token T1.
    session.set_grid_settle_timeout(NO_SETTLE);
    pan_step(&session, 120.0);
    session.set_grid_settle_timeout(Duration::ZERO);
    session.request_preparation().unwrap();

    // Gesture 2 lands *before polling*: re-arm and settle to token T2 while the T1
    // preparation is still in flight. The existing token check must drop the stale
    // completion, so only T2 is ever published.
    session.set_grid_settle_timeout(NO_SETTLE);
    pan_step(&session, 240.0);
    session.set_grid_settle_timeout(Duration::ZERO);
    let final_bounds = viewport_world_bounds(&session.view_snapshot());

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        session.poll_preparation().unwrap();
        let diagnostics = session.diagnostics();
        if diagnostics.prepared_token == Some(diagnostics.requested_token) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the overlapped settle never published the final input"
        );
        std::thread::yield_now();
    }

    let after = session.diagnostics();
    assert_eq!(
        after.scene_publication_revision,
        publications_before + 1,
        "an overlapped settle publishes exactly one scene (the final input)"
    );
    assert_eq!(
        session.overlays().viewport_bounds,
        final_bounds,
        "the single publication carries the final viewport bounds"
    );
    assert_eq!(
        after.ready_base_revision, base_before,
        "the base is reused across the overlapped settle"
    );
}

#[test]
fn grid_refresh_diagnostics_are_honest_and_never_fake_a_commit() {
    let session = session_with_published_base();
    assert!(!session.grid_refresh_pending());
    assert_eq!(session.grid_refresh_diagnostic(), "grid-refresh-idle");

    session.set_grid_settle_timeout(NO_SETTLE);
    let before = session.diagnostics();
    pan_step(&session, 45.0);

    assert!(session.grid_refresh_pending());
    assert_eq!(session.grid_refresh_diagnostic(), "grid-refresh-pending");
    // Honest: while the refresh is deferred, nothing claims the final grid is in.
    assert_eq!(
        session.diagnostics().requested_token,
        before.requested_token,
        "a pending refresh must not report a content advance"
    );
    assert_eq!(
        session.diagnostics().prepared_token,
        before.prepared_token,
        "a pending refresh must not report a publication"
    );

    settle_camera_refresh(&session);
    assert_eq!(session.grid_refresh_diagnostic(), "grid-refresh-idle");
    assert_eq!(
        session.diagnostics().requested_token,
        before.requested_token + 1,
        "the settle commit is the one that advances the content revision"
    );
}

#[test]
fn bounds_preserving_view_change_covers_a_pending_camera_refresh() {
    let session = session_with_published_base();
    session.set_grid_settle_timeout(NO_SETTLE);
    let before = session.diagnostics();

    pan_step(&session, 75.0);
    assert!(
        session.grid_refresh_pending(),
        "the pan arms the deferred refresh"
    );

    // A fit-on-resize: the canvas halves while the camera zooms in by 2x, so the
    // world extent is *identical* and `bounds_changed` is false, while
    // `canvas_size` differs so `camera_only` is false. The pending refresh covers
    // these very bounds and must not be dropped without a prepare.
    let mut view = session.view_snapshot();
    let scale = match view.camera.projection {
        cad_app::Projection::Orthographic { scale } => scale,
        _ => panic!("the 2D fixture must use an orthographic camera"),
    };
    view.camera.projection = cad_app::Projection::Orthographic { scale: scale * 2.0 };
    view.canvas_size = [view.canvas_size[0] * 0.5, view.canvas_size[1] * 0.5];
    assert!(session.apply_view_snapshot(view).unwrap());

    assert!(
        !session.grid_refresh_pending(),
        "the deferred bounds must be covered, never dropped"
    );
    assert_eq!(
        session.diagnostics().requested_token,
        before.requested_token + 1,
        "the pending refresh becomes exactly one content advance"
    );
    assert_eq!(
        session.diagnostics().input_revision,
        before.input_revision,
        "a bounds-preserving view change is still not an identity change"
    );
}
