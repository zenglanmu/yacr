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

/// DETERMINISTIC HARDWARE: N consecutive hover changes upload **only** entity
/// flags. The published scene identity is untouched, so no base-only frame is
/// ever produced, no geometry stream/chunk or overlay suffix is re-uploaded, and
/// the current scene stays committed and presented throughout.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn hover_changes_push_only_flags_and_never_drop_the_current_scene() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();
    session.sync_drawing(Some(Arc::new(db_with_two_quads())));
    session.set_overlay_visibility(OverlayVisibility {
        axes: false,
        grid: false,
        ..OverlayVisibility::default()
    });
    session.prepare().unwrap();
    drive_uploads(&session);
    session.render([200.0, 100.0], 1.0).unwrap().expect("frame");

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
    let scene = session.current_prepared_scene().unwrap();
    let scene_revision = scene.revision;
    let flags_revision = scene.flags_revision;
    let (flags_before, streams_before, meshes_before) = renderer_counters(&session);
    let batches_before = renderer_batch_count(&session);
    let uploads_before = session.diagnostics().runtime.uploads_started;
    let suffix_before = session.overlay_suffix_commits();

    for step in 0..8 {
        let mut overlays = session.overlays();
        overlays.hovered = if step % 2 == 0 {
            SelectionSet::from_refs([reference.clone()])
        } else {
            SelectionSet::new()
        };
        assert!(session.set_overlays(overlays));
        // Flags-only: no worker preparation, the scene stays current.
        assert!(
            session.preparation_current(),
            "hover step {step} must keep the published scene current"
        );
        session.apply_ready_updates().unwrap();
        assert!(
            uploads_committed(&session),
            "hover step {step} must keep the scene committed"
        );
        let frame = session.render([200.0, 100.0], 1.0).unwrap().expect("frame");
        assert_eq!(
            frame.completeness,
            FrameCompleteness::CurrentScene,
            "hover step {step} must present the full scene, never the fallback"
        );
        assert!(
            !session.overlays_suppressed(),
            "hover step {step} must not suppress overlays"
        );
        assert!(
            session.current_drawing_presented(),
            "hover step {step} must present the current drawing"
        );
        assert_eq!(
            session.current_prepared_scene().unwrap().revision,
            scene_revision,
            "hover step {step} must not republish the scene"
        );
        let (_, streams, meshes) = renderer_counters(&session);
        assert_eq!(
            streams, streams_before,
            "hover step {step} must not re-upload an instance stream"
        );
        assert_eq!(
            meshes, meshes_before,
            "hover step {step} must not re-upload a mesh chunk"
        );
        assert_eq!(
            renderer_batch_count(&session),
            batches_before,
            "hover step {step} must not change the resident batch set"
        );
        assert_eq!(
            session.overlay_suffix_commits(),
            suffix_before,
            "hover step {step} must not churn the overlay suffix"
        );
        assert_eq!(
            session.diagnostics().runtime.uploads_started,
            uploads_before,
            "hover step {step} must not start a geometry upload"
        );
    }
    assert!(
        renderer_counters(&session).0 > flags_before,
        "the hover tint must reach the renderer through the flags path"
    );
    assert!(
        session.current_prepared_scene().unwrap().flags_revision > flags_revision,
        "each distinct hover must advance the flags revision"
    );
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
