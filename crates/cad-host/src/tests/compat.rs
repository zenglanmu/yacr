use super::*;

// --- P2-B compatibility follow-up contracts --------------------------------

#[test]
fn clear_user_preference_empties_the_one_authority_store() {
    let store = Rc::new(RecordingConfig::default());
    let core = core_with_config(store.clone());
    let client: &dyn EngineClient = &core;

    // Apply an allowed preference through the single authority; it flips a
    // default-on flag and persists a non-empty projection.
    core.apply_user_preference_json(
        r#"{"ui":{"components":{"layerPanel":{"initiallyOpen":false}}}}"#,
    )
    .unwrap();
    let with_preference = core.config_snapshot();
    assert!(
        !store.persisted.borrow().is_empty() && store.persisted.borrow().last().unwrap() != "{}",
        "applying a preference persists its projection"
    );

    let persisted_before = store.persisted.borrow().len();
    client.clear_user_preference().unwrap();

    // The store is really empty: the effective config returns to the host
    // default, and the allowed projection is persisted as an empty object.
    let default_effective = core_with_config(Rc::new(RecordingConfig::default()))
        .config_snapshot()
        .effective;
    let cleared = core.config_snapshot();
    assert_eq!(cleared.effective, default_effective);
    assert_ne!(cleared.effective, with_preference.effective);
    let persisted = store.persisted.borrow();
    assert!(persisted.len() > persisted_before);
    assert_eq!(persisted.last().unwrap(), "{}");
    drop(persisted);

    // Inherent and EngineClient surfaces share the one authority, and the
    // change is published as a config delta (not a full state query).
    assert_eq!(EngineClient::config_snapshot(&core), cleared);
    let updates = client.take_updates().unwrap();
    assert_eq!(updates.config.expect("config delta"), cleared);

    // A clear is idempotent and never inherits the trait's refusing default.
    client.clear_user_preference().unwrap();
    assert!(client.clear_user_preference().is_ok());
}

#[test]
fn backend_lifecycle_delta_is_published_without_a_full_query() {
    use cad_app::recovery::BackendOutcome;

    let core = core();
    let client: &dyn EngineClient = &core;

    // The first drain publishes the current actual backend (no device yet).
    let first = client.take_updates().unwrap();
    assert!(
        first.backend.is_some(),
        "the first drain publishes the actual backend state"
    );
    // Nothing changed: no backend delta, and the whole batch is empty.
    let second = client.take_updates().unwrap();
    assert!(second.backend.is_none());
    assert_eq!(second, HostUpdates::default());

    // A real device failure changes the actual backend lifecycle. The delta is
    // published even though every other dirty flag is clear, and it must not
    // force a full `HostState` derivation.
    core.render.fail_device("synthetic device failure", false);
    let updates = client.take_updates().unwrap();
    assert!(
        updates.full.is_none(),
        "a backend delta must not force a full state query"
    );
    let backend = updates.backend.expect("backend delta");
    match backend.outcome {
        Some(BackendOutcome::Failed { .. }) => {}
        other => panic!("expected a Failed backend outcome, got {other:?}"),
    }
    assert_eq!(backend.api, None);

    // Stable again once published.
    assert!(client.take_updates().unwrap().backend.is_none());
}

#[test]
fn full_state_reports_the_document_unit_context() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let document_units = {
        let c = core.controller.borrow();
        c.application
            .workspace
            .documents
            .get(&c.document_id)
            .unwrap()
            .units
            .clone()
    };
    let full = client.take_updates().unwrap().full.expect("full state");
    assert_eq!(
        full.units,
        Some(document_units),
        "units mirror the document context, never None while a document exists"
    );
}

#[test]
fn host_state_selected_draw_kind_is_none_idle_and_some_during_capture() {
    let core = core();
    let client: &dyn EngineClient = &core;

    assert_eq!(
        client
            .take_updates()
            .unwrap()
            .full
            .unwrap()
            .selected_draw_kind,
        None,
        "idle must not synthesize a Line"
    );
    client
        .dispatch(EngineRequest::Interaction(InteractionRequest::BeginDraw(
            DrawToolKind::Line,
        )))
        .unwrap();
    assert_eq!(
        client
            .take_updates()
            .unwrap()
            .full
            .unwrap()
            .selected_draw_kind,
        Some(DrawToolKind::Line)
    );
    client
        .dispatch(EngineRequest::Interaction(InteractionRequest::Cancel))
        .unwrap();
    assert_eq!(
        client
            .take_updates()
            .unwrap()
            .full
            .unwrap()
            .selected_draw_kind,
        None,
        "after cancel the idle state is None again"
    );
}

#[test]
fn pointer_contact_lifecycle_two_ids_repeat_down_cancel_and_last_release() {
    use cad_app::interaction::PointerPhase;

    let core = core();
    let revision = core.request_context().metrics_revision.unwrap();
    let revision_before = core.controller.borrow().drawing().unwrap().revision();

    // Repeat Down for the same contact id is idempotent: it must not register a
    // second contact and cancel the active capture.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    core.controller
        .borrow_mut()
        .capture_tool_point(p(1.0, 1.0))
        .unwrap();
    dispatch_pointer(&core, touch(1, PointerPhase::Down, [10.0, 10.0], revision));
    dispatch_pointer(&core, touch(1, PointerPhase::Down, [10.0, 10.0], revision));
    assert!(
        matches!(core.controller.borrow().session.tool, ToolState::Drawing(_)),
        "a repeated Down must not count as a second contact"
    );
    assert!(core.controller.borrow().session.draw_preview().is_some());

    // A genuine second contact id enters multi-touch and cancels the capture,
    // never committing it.
    dispatch_pointer(&core, touch(2, PointerPhase::Down, [40.0, 10.0], revision));
    assert!(
        matches!(core.controller.borrow().session.tool, ToolState::Idle),
        "the second contact cancels the active tool"
    );
    assert!(core.controller.borrow().session.draw_preview().is_none());
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        revision_before,
        "multi-touch must never commit a drawing"
    );

    // Releasing one of two contacts keeps the gesture; the last release ends it.
    dispatch_pointer(&core, touch(2, PointerPhase::Up, [40.0, 10.0], revision));
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));
    dispatch_pointer(&core, touch(1, PointerPhase::Up, [10.0, 10.0], revision));
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));

    // Cancel identity: cancelling one contact leaves the other; cancelling the
    // last empties the gesture and cancels any tool.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    dispatch_pointer(&core, touch(3, PointerPhase::Down, [1.0, 1.0], revision));
    dispatch_pointer(&core, touch(4, PointerPhase::Down, [2.0, 2.0], revision));
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));
    dispatch_pointer(&core, touch(3, PointerPhase::Cancel, [1.0, 1.0], revision));
    dispatch_pointer(&core, touch(4, PointerPhase::Cancel, [2.0, 2.0], revision));
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));
    assert!(core.controller.borrow().session.draw_preview().is_none());
}

#[test]
fn sync_view_does_not_churn_render_revisions_on_a_stable_tick() {
    let core = core();
    // Settle the first (changing) sync, then a stable tick must be idempotent:
    // the overlay bounds are built once, so the render input/scene revision
    // cannot grow just because the 50 ms timer ran again.
    core.sync_view().unwrap();
    let input = core.render.input_revision();
    let overlay = core.render.overlay_revision();
    for _ in 0..4 {
        core.sync_view().unwrap();
    }
    assert_eq!(
        core.render.input_revision(),
        input,
        "a stable sync must not advance the render input revision"
    );
    assert_eq!(
        core.render.overlay_revision(),
        overlay,
        "a stable sync must not churn the overlay revision"
    );

    // The same holds for repeated update drains in the stable tick.
    let client: &dyn EngineClient = &core;
    assert!(client.take_updates().unwrap().full.is_some());
    assert_eq!(client.take_updates().unwrap(), HostUpdates::default());
    assert_eq!(client.take_updates().unwrap(), HostUpdates::default());
    assert_eq!(core.render.input_revision(), input);
    assert_eq!(core.render.overlay_revision(), overlay);
}

#[test]
fn missing_and_failed_images_surface_raw_resource_diagnostics() {
    let core = core();
    *core.last_image_report.borrow_mut() = Some(ImageLoadReport {
        requested: vec!["a.png".into(), "b.png".into()],
        loaded: Vec::new(),
        unresolved: vec!["a.png".into()],
        failed: vec!["b.png: decode error".into()],
        bytes: 0,
    });
    core.full_dirty.set(true);

    let full = EngineClient::take_updates(&core)
        .unwrap()
        .full
        .expect("full state");
    let missing: Vec<_> = full
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "resource.missing")
        .collect();
    assert_eq!(
        missing.len(),
        2,
        "one diagnostic per unresolved and per failed image"
    );
    assert!(
        missing.iter().all(|diagnostic| diagnostic.object.is_none()),
        "image diagnostics are document-level"
    );
    assert!(
        missing.iter().all(|diagnostic| diagnostic.severity
            == Some(cad_engine::protocol::DiagnosticSeverity::Error)),
        "missing resources are error-severity, mirroring the pre-decoupling model"
    );
    assert!(
        missing.iter().all(|diagnostic| diagnostic.completeness
            == Some(cad_domain::Completeness::Missing(Vec::new()))),
        "missing resources participate in the Missing completeness summary"
    );
    let keys: Vec<String> = missing
        .iter()
        .filter_map(|diagnostic| match diagnostic.parameters.first() {
            Some(RawDiagnosticParameter::Key(key)) => Some(key.clone()),
            _ => None,
        })
        .collect();
    assert!(
        keys.contains(&"a.png".to_string()),
        "the unresolved key must travel raw for the drawer: {keys:?}"
    );
    assert!(
        keys.contains(&"b.png: decode error".to_string()),
        "the failed key must travel raw for the drawer: {keys:?}"
    );

    // The raw report is still published in full for the resources drawer.
    let images = full.resources.images.expect("image report published");
    assert_eq!(images.unresolved, vec!["a.png".to_string()]);
    assert_eq!(images.failed, vec!["b.png: decode error".to_string()]);
}

#[test]
fn legitimate_three_d_orthographic_view_stays_three_d_in_snapshot_and_navigation() {
    let core = core();
    let (viewport_id, camera) = {
        let c = core.controller.borrow();
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        (c.viewport_id, viewport.camera)
    };
    // A legitimate 3D view may keep an orthographic camera. The reported mode
    // must follow the session view mode, not a guess from the projection variant.
    core.controller
        .borrow_mut()
        .application
        .workspace
        .viewports
        .get_mut(&viewport_id)
        .unwrap()
        .view_mode = cad_app::ViewMode2d3d::ThreeD { saved_2d: camera };

    core.sync_view().unwrap();
    assert_eq!(
        core.render.view_snapshot().mode,
        cad_app::ProjectionKind::ThreeD,
        "the render snapshot must report the real 3D view mode"
    );
    assert!(
        core.render.overlays().viewport_bounds.is_none(),
        "a 3D view must not be given a fabricated 2D overlay extent"
    );
    let navigation = EngineClient::take_updates(&core)
        .unwrap()
        .navigation
        .expect("navigation state");
    assert_eq!(
        navigation.mode,
        cad_app::ProjectionKind::ThreeD,
        "navigation must agree with the snapshot's 3D mode"
    );

    // A typed Primary drag in the 3D (orthographic) view must orbit, not pan:
    // pan translates the camera target, while orbit keeps the target (and the
    // eye-target distance) and only rotates the eye.
    let revision = core.request_context().metrics_revision.unwrap();
    let before = {
        let c = core.controller.borrow();
        c.application.workspace.viewports[&c.viewport_id].camera
    };
    let distance_before = before.distance();
    dispatch_pointer(
        &core,
        mouse(
            1,
            cad_app::interaction::PointerPhase::Down,
            [100.0, 100.0],
            revision,
        ),
    );
    dispatch_pointer(
        &core,
        mouse(
            1,
            cad_app::interaction::PointerPhase::Move,
            [160.0, 120.0],
            revision,
        ),
    );
    dispatch_pointer(
        &core,
        mouse(
            1,
            cad_app::interaction::PointerPhase::Up,
            [160.0, 120.0],
            revision,
        ),
    );
    let after = {
        let c = core.controller.borrow();
        c.application.workspace.viewports[&c.viewport_id].camera
    };
    assert_ne!(after, before, "a 3D Primary drag must move the camera");
    assert_eq!(
        after.target, before.target,
        "orbit preserves the camera target (a pan would translate it)"
    );
    assert!(
        (after.distance() - distance_before).abs() < 1e-9,
        "orbit preserves the eye-target distance"
    );
    assert!(
        matches!(core.controller.borrow().session.tool, ToolState::Idle),
        "a 3D orbit must not enter the panning tool state"
    );
}

#[test]
fn two_d_overlay_bounds_are_ordered_and_contain_the_viewport_corners() {
    let core = core();
    core.sync_view().unwrap();
    let (min, max) = core
        .render
        .overlays()
        .viewport_bounds
        .expect("a 2D plan view has an overlay extent");
    assert!(
        min.x <= max.x && min.y <= max.y && min.z <= max.z,
        "the overlay extent must be axis-ordered, not top-left/bottom-right: {min:?}..{max:?}"
    );
    assert!(
        max.x > min.x && max.y > min.y,
        "the 2D extent must be non-degenerate on both screen axes: {min:?}..{max:?}"
    );

    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    let [width, height] = viewport.logical_size;
    let top_left = viewport
        .screen_to_world([0.0, 0.0], viewport.logical_size)
        .unwrap();
    let bottom_left = viewport
        .screen_to_world([0.0, height], viewport.logical_size)
        .unwrap();
    assert!(
        top_left.y > bottom_left.y,
        "screen Y is inverted; the extent must be normalized, not passed through as-is"
    );
    for corner in [[0.0, 0.0], [width, 0.0], [0.0, height], [width, height]] {
        let point = viewport
            .screen_to_world(corner, viewport.logical_size)
            .unwrap();
        assert!(
            point.x >= min.x - 1e-9 && point.x <= max.x + 1e-9,
            "corner {point:?} x is outside {min:?}..{max:?}"
        );
        assert!(
            point.y >= min.y - 1e-9 && point.y <= max.y + 1e-9,
            "corner {point:?} y is outside {min:?}..{max:?}"
        );
    }
}
