use super::*;

// ---------------------------------------------------------------------------
// Production flags-only fast path (B1/B2/S1/S4): a selection change must tint
// through the in-stream flags without re-uploading any geometry.
// ---------------------------------------------------------------------------

/// DETERMINISTIC HARDWARE: an entity-only selection change goes through the
/// production flags-only fast path — it tints the selected entity, uploads only
/// flags data, and leaves the instance streams, mesh chunks and overlay suffix
/// untouched. Switching the selection tints the new entity and untints the old.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn selection_flags_only_fast_path_tints_without_reuploading_geometry() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();
    session.sync_drawing(Some(Arc::new(db_with_two_quads())));
    // Turn off the reference axes/grid so the sampled pixels are the quads' own
    // pixels, not a white axis/grid line drawn over the quad centre.
    session.set_overlay_visibility(OverlayVisibility {
        axes: false,
        grid: false,
        ..OverlayVisibility::default()
    });
    session.prepare().unwrap();
    drive_uploads(&session);

    let size = [200.0, 100.0];
    let baseline = session_frame(&session, size);
    let (ax, ay) = world_to_pixel_2d(200, 100, [-40.0, 0.0]);
    let (bx, by) = world_to_pixel_2d(200, 100, [40.0, 0.0]);
    let ai = (ay as usize * 200 + ax as usize) * 4;
    let bi = (by as usize * 200 + bx as usize) * 4;
    for (label, i) in [("A", ai), ("B", bi)] {
        assert!(
            baseline[i] > 8 || baseline[i + 1] > 8 || baseline[i + 2] > 8,
            "quad {label} must be drawn at its sample pixel"
        );
    }

    let (flags0, streams0, chunks0) = renderer_counters(&session);
    let suffix0 = session.overlay_suffix_commits();

    // Select A: a flags-only change (the controller reuses the overlay `Arc`).
    assert!(session.set_selection(SelectionSet::from_refs([model_entity(1)])));
    session.prepare().unwrap();
    session.apply_ready_updates().unwrap();
    {
        // B1 guard: the runtime retains the applied overlay `Arc` by identity, so
        // a freed-and-reallocated overlay cannot alias the fast-path comparison.
        let state = session.0.borrow();
        let applied = state
            .runtime
            .applied_highlight_for_test()
            .expect("the applied overlay is retained");
        let ready = state.controller.ready.as_ref().expect("ready scene");
        assert!(
            Arc::ptr_eq(&applied, &ready.highlight),
            "the retained overlay must be the published one"
        );
    }
    let frame_a = session_frame(&session, size);
    assert_ne!(
        &frame_a[ai..ai + 3],
        &baseline[ai..ai + 3],
        "selecting A must tint A"
    );
    assert_eq!(
        &frame_a[bi..bi + 3],
        &baseline[bi..bi + 3],
        "selecting A must not tint B"
    );
    let (flags1, streams1, chunks1) = renderer_counters(&session);
    assert!(flags1 > flags0, "a selection change must upload flags");
    assert_eq!(
        streams1, streams0,
        "a selection change must not re-upload instance streams"
    );
    assert_eq!(
        chunks1, chunks0,
        "a selection change must not re-upload mesh chunks"
    );
    assert_eq!(
        session.overlay_suffix_commits(),
        suffix0,
        "a selection change must not commit the overlay suffix"
    );

    // Switch to B: A untinted, B tinted, counters still flat.
    assert!(session.set_selection(SelectionSet::from_refs([model_entity(2)])));
    session.prepare().unwrap();
    session.apply_ready_updates().unwrap();
    let frame_b = session_frame(&session, size);
    assert_eq!(
        &frame_b[ai..ai + 3],
        &baseline[ai..ai + 3],
        "switching to B must untint A"
    );
    assert_ne!(
        &frame_b[bi..bi + 3],
        &baseline[bi..bi + 3],
        "switching to B must tint B"
    );
    let (flags2, streams2, chunks2) = renderer_counters(&session);
    assert!(
        flags2 > flags1,
        "the second selection change must upload flags"
    );
    assert_eq!(streams2, streams1);
    assert_eq!(chunks2, chunks1);
    assert_eq!(session.overlay_suffix_commits(), suffix0);
}

/// DETERMINISTIC HARDWARE: a selected entity the base build excluded (a hidden
/// layer) resolves in the database but has no resident pick key. The runtime must
/// surface `flags.unresolved` instead of treating the update as a silent success.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn a_selected_entity_excluded_from_the_base_reports_a_diagnostic() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();
    session.sync_drawing(Some(Arc::new(db_with_two_quads())));

    // Hide the layer both entities live on, so neither is resident.
    let mut layers = LayerOverrideSet::new();
    layers.set(LayerId(0), false);
    session.set_layers(layers);
    session.prepare().unwrap();
    drive_uploads(&session);

    // Entity 1 still resolves in the database, so it yields a flag request; but it
    // has no resident key, so the apply must report it.
    assert!(session.set_selection(SelectionSet::from_refs([model_entity(1)])));
    session.prepare().unwrap();
    session.apply_ready_updates().unwrap();

    let diagnostic = session.diagnostic().expect("a diagnostic is surfaced");
    assert!(
        diagnostic.contains("flags.unresolved"),
        "the unmatched selection must be reported, got {diagnostic:?}"
    );
    assert!(diagnostic.contains('1'));
}
