use super::*;

// ---------------------------------------------------------------------------
// A camera-only operation is not a scene mutation: a selection must survive a
// zoom and the camera settle that follows it. The tint lives in the renderer's
// entity-flag table (whole-entity refs are the in-stream flags path, never an
// overlay batch), so the fences below read the retained flag request, the
// resolved keys and the rendered pixels.
// ---------------------------------------------------------------------------

/// DETERMINISTIC HARDWARE: a selection survives a camera-only zoom gesture and
/// the settle commit that follows it. The retained flag request and its resolved
/// keys are unchanged, the selection state is unchanged, no `flags.unresolved` /
/// `highlight.unresolved` is reported, and the selected pixel is still tinted.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn selection_survives_a_camera_zoom_and_settle() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();
    session.sync_drawing(Some(Arc::new(db_with_one_quad())));
    // No axes/grid, so the sampled centre pixel is the quad's own pixel.
    session.set_overlay_visibility(OverlayVisibility {
        axes: false,
        grid: false,
        ..OverlayVisibility::default()
    });
    session.prepare().unwrap();
    drive_uploads(&session);

    let size = [200.0, 100.0];
    let (cx, cy) = world_to_pixel_2d(200, 100, [0.0, 0.0]);
    let centre = (cy as usize * 200 + cx as usize) * 4;

    assert!(session.set_selection(SelectionSet::from_refs([model_entity(1)])));
    session.prepare().unwrap();
    drive_uploads(&session);
    let keys_before = entity_flag_keys(&session);
    assert!(
        !keys_before.is_empty(),
        "the selected entity must resolve to a resident whole-entity key"
    );
    let selected_frame = session_frame(&session, size);
    let selected_pixel = selected_frame[centre..centre + 3].to_vec();

    // The camera-only gesture: N zoom steps, then the settle commit.
    session.set_grid_settle_timeout(NO_SETTLE);
    for _ in 0..4 {
        zoom_step(&session, 1.0 / 1.1);
    }
    assert_eq!(
        session.selection().refs().len(),
        1,
        "a zoom must not clear the selection state"
    );
    settle_camera_refresh(&session);
    drive_uploads(&session);

    assert_eq!(
        session.selection().refs().len(),
        1,
        "the settle must not clear the selection state"
    );
    assert_eq!(
        entity_flag_keys(&session),
        keys_before,
        "a camera-only operation must not change the resolved entity-flag keys"
    );
    let diagnostic = session.diagnostic().unwrap_or_default();
    assert!(
        !diagnostic.contains("unresolved"),
        "a camera-only operation must not degrade the highlight, got {diagnostic:?}"
    );
    assert!(session.current_drawing_presented());

    // The tint itself: clear the selection (a flags-only change, same camera and
    // same overlay suffix) and re-sample. The selected pixel must differ; a pixel
    // outside the quad must not.
    let after_zoom = session_frame(&session, size);
    let after_pixel = after_zoom[centre..centre + 3].to_vec();
    assert_eq!(
        after_pixel, selected_pixel,
        "the tint must survive the zoom and settle unchanged"
    );
    assert!(session.set_selection(SelectionSet::new()));
    session.prepare().unwrap();
    session.apply_ready_updates().unwrap();
    let unselected = session_frame(&session, size);
    assert_ne!(
        after_pixel,
        unselected[centre..centre + 3].to_vec(),
        "the selected pixel must be tinted relative to the unselected scene"
    );
}

/// DETERMINISTIC HARDWARE: the 2D navigation fallback re-asserts the retained
/// entity flags after its truncating commit. The renderer's retained request is
/// cleared directly (the transient empty-resident reset), so only a re-push from
/// the runtime can restore the tint when the fallback is entered.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn camera_settle_fallback_reasserts_the_retained_entity_flags() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();
    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    drive_uploads(&session);

    // A ref that is resident in the base (the demo document's first base entity).
    let reference = {
        let state = session.0.borrow();
        let ready = state.controller.ready.as_ref().expect("ready scene");
        let document = ready.base.stamp.document;
        let entity = ready
            .base
            .added
            .iter()
            .find_map(|batch| batch.sources.first().map(|source| source.entity))
            .expect("a resident base entity");
        SelectionRef {
            document,
            entity,
            instance: InstancePath::default(),
            sub_element: None,
        }
    };
    assert!(session.set_selection(SelectionSet::from_refs([reference])));
    session.prepare().unwrap();
    drive_uploads(&session);
    let keys_before = entity_flag_keys(&session);
    assert!(!keys_before.is_empty());

    // Simulate the renderer-side loss of the retained request.
    {
        let mut state = session.0.borrow_mut();
        state
            .runtime
            .renderer
            .as_mut()
            .expect("attached renderer")
            .set_entity_flags(&[])
            .unwrap();
    }
    assert!(
        entity_flag_keys(&session).is_empty(),
        "the simulated loss must clear the tint"
    );

    // Enter the fallback: an overlay input change that produces no batches.
    let mut overlays = session.overlays();
    overlays.visibility.snap_hints = !overlays.visibility.snap_hints;
    assert!(session.set_overlays(overlays));
    assert!(!session.preparation_current());
    let frame = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("base-only frame");
    assert_eq!(
        frame.completeness,
        FrameCompleteness::BaseOnlyPendingOverlays
    );
    assert!(session.overlays_suppressed());
    assert_eq!(
        entity_flag_keys(&session),
        keys_before,
        "the fallback must re-assert the retained entity flags after its commit"
    );

    // The full-scene restore keeps the tint.
    session.prepare().unwrap();
    drive_uploads(&session);
    assert!(!session.overlays_suppressed());
    assert_eq!(entity_flag_keys(&session), keys_before);
}

/// DETERMINISTIC HARDWARE: the `flags.unresolved` state an unmatched selection
/// re-resolution produces reaches a probe-visible place — `RenderDiagnostics`
/// `result_status`, which the host logs as `[probe] pan-wait … status=…` under
/// `YACR_PROBE_CAD_LOG`. No new truth is invented: it is the same runtime
/// diagnostic `RenderSessionHandle::diagnostic` exposes.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn unmatched_selection_reaches_the_probe_status() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();
    session.sync_drawing(Some(Arc::new(db_with_two_quads())));

    // Hide the layer both entities live on, so the selection resolves in the
    // database but has no resident key.
    let mut layers = LayerOverrideSet::new();
    layers.set(LayerId(0), false);
    session.set_layers(layers);
    session.prepare().unwrap();
    drive_uploads(&session);

    assert!(session.set_selection(SelectionSet::from_refs([model_entity(1)])));
    session.prepare().unwrap();
    session.apply_ready_updates().unwrap();

    let status = session
        .diagnostics()
        .result_status
        .expect("the unmatched selection must surface a status");
    assert!(
        status.contains("flags.unresolved"),
        "the probe status must carry the unmatched selection, got {status:?}"
    );
}
