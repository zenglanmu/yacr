use super::*;

// ---------------------------------------------------------------------------
// 2D navigation fallback (base-only frames while overlays wait).
//
// The qualifier below is exercised two ways:
//   * pure-state tests (no GPU) model the resident base explicitly and are
//     labeled as such — they prove the gating logic, not GPU behavior;
//   * `#[ignore]` hardware tests drive a real headless adapter (P620/Vulkan on
//     this machine) for the end-to-end suppression/restore proof. Run them with
//     `cargo test -p cad-engine --locked -- --ignored`.
// ---------------------------------------------------------------------------

/// PURE-STATE (no GPU): the qualifier holds only for a valid 2D view whose base
/// input identity matches the last publication while the GPU base is resident.
#[test]
fn base_only_navigation_qualifier_requires_same_resident_2d_base() {
    let session = session_with_published_base();
    let mut state = session.0.borrow_mut();
    let base_pub = state.base_publication_revision;
    state
        .runtime
        .force_resident_base_for_test(base_pub, /* base_batches */ 1);
    assert!(
        base_only_navigation_eligible(&state),
        "resident base + valid 2D view + unchanged base identity is eligible"
    );

    // A 3D view never uses the 2D fallback.
    let two_d = state.view.clone();
    let mut three_d = two_d.clone();
    three_d.mode = ProjectionKind::ThreeD;
    state.view = three_d;
    assert!(!base_only_navigation_eligible(&state));
    state.view = two_d;

    // An unuploaded/stale base (applied revision behind the publication) blocks.
    state.runtime.force_resident_base_for_test(base_pub + 1, 1);
    assert!(
        !base_only_navigation_eligible(&state),
        "a base whose applied revision is not the published one must block"
    );

    // A detached runtime blocks even with a matching publication.
    state.runtime.detach();
    state.runtime.force_resident_base_for_test(base_pub, 1);
    assert!(base_only_navigation_eligible(&state));
    state.runtime.detach();
    assert!(
        !base_only_navigation_eligible(&state),
        "no attached device must block the fallback"
    );
}

/// PURE-STATE (no GPU): a changed base input identity (new drawing, document,
/// layers, fonts or images) blocks the fallback even if the previous base is
/// still modeled as resident.
#[test]
fn base_only_navigation_qualifier_blocks_changed_base_inputs() {
    // New drawing (different Arc, so a different authoritative base).
    let session = session_with_published_base();
    let other = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    session.sync_drawing(Some(other));
    {
        let mut state = session.0.borrow_mut();
        let base_pub = state.base_publication_revision;
        state.runtime.force_resident_base_for_test(base_pub, 1);
        assert!(
            !base_only_navigation_eligible(&state),
            "a new drawing must block the fallback"
        );
    }

    // Document switch.
    let session = session_with_published_base();
    {
        let mut view = session.view_snapshot();
        view.document = DocumentId(2);
        session.apply_view_snapshot(view).unwrap();
        let mut state = session.0.borrow_mut();
        let base_pub = state.base_publication_revision;
        state.runtime.force_resident_base_for_test(base_pub, 1);
        assert!(
            !base_only_navigation_eligible(&state),
            "a document switch must block the fallback"
        );
    }

    // Layer override.
    let session = session_with_published_base();
    let mut layers = LayerOverrideSet::new();
    layers.set(LayerId(0), false);
    session.set_layers(layers);
    {
        let mut state = session.0.borrow_mut();
        let base_pub = state.base_publication_revision;
        state.runtime.force_resident_base_for_test(base_pub, 1);
        assert!(
            !base_only_navigation_eligible(&state),
            "a layer change must block the fallback"
        );
    }

    // Font resource.
    let session = session_with_published_base();
    session.set_fonts(Some(Arc::new(FontEngine::new())));
    {
        let mut state = session.0.borrow_mut();
        let base_pub = state.base_publication_revision;
        state.runtime.force_resident_base_for_test(base_pub, 1);
        assert!(
            !base_only_navigation_eligible(&state),
            "a font change must block the fallback"
        );
    }

    // Image resource.
    let session = session_with_published_base();
    session.set_images(Some(Arc::new(DecodedImageCache::new())));
    {
        let mut state = session.0.borrow_mut();
        let base_pub = state.base_publication_revision;
        state.runtime.force_resident_base_for_test(base_pub, 1);
        assert!(
            !base_only_navigation_eligible(&state),
            "an image change must block the fallback"
        );
    }
}

/// PURE-STATE (no GPU): the authoritative database revision is part of the base
/// input identity, so a pointer-equal drawing whose revision moved is a
/// different base and the fallback stays blocked until re-published. (The
/// database is mutable in place behind a shared `Arc`, so the bare pointer
/// alone would hide the edit.)
#[test]
fn base_only_navigation_qualifier_identity_includes_authoritative_revision() {
    let base = BaseInputIdentity {
        drawing: 0x1234,
        revision: Some(Revision(1)),
        document: DocumentId(0),
        space: SpaceSelection::Model,
        fonts_generation: 0,
        images_generation: 0,
        layers: LayerOverrideSet::new(),
    };
    let edited = BaseInputIdentity {
        revision: Some(Revision(2)),
        ..base.clone()
    };
    assert_ne!(
        base, edited,
        "an in-place revision bump must change the base identity"
    );
    // The same pointer and revision is the same base.
    assert_eq!(base, base.clone());
}

// ---------------------------------------------------------------------------
// Camera-only navigation must not invalidate the published scene.
//
// Pan/zoom advance `view_revision` and the camera-derived reference-grid bounds,
// but every overlay batch is world-space geometry that renders correctly under
// any camera. The published scene therefore stays current (no base-only/full
// frame alternation, no suffix churn) while the reference grid still refines in
// the background. Real scene-identity changes keep invalidating as before.
// ---------------------------------------------------------------------------

/// PURE-STATE (no GPU): a pan keeps the published scene current and never enters
/// the base-only fallback or republishes per step.
#[test]
fn camera_only_navigation_keeps_published_scene_and_does_not_churn_overlays() {
    let session = session_with_published_base();
    assert!(session.preparation_current());
    let baseline = session.current_prepared_scene().expect("published scene");
    let input_before = session.input_revision();
    let overlay_before = session.overlay_revision();
    let view_before = session.view_revision();
    let suffix_before = session.overlay_suffix_commits();

    for step in 1..=3 {
        let mut view = session.view_snapshot();
        view.camera.target.x += 250.0 * step as f64;
        assert!(session.apply_view_snapshot(view).unwrap());

        assert!(
            session.preparation_current(),
            "pan step {step} must keep the ready scene current"
        );
        let ready = session.current_prepared_scene().expect("ready scene");
        assert_eq!(
            ready.revision, baseline.revision,
            "a camera-only step must not republish the scene"
        );
        assert!(
            !session.overlays_suppressed(),
            "camera-only navigation never suppresses the overlay suffix"
        );
        assert!(
            session.cad_frame_dirty(),
            "the new camera still needs a frame"
        );

        // Nothing new was published, so there is nothing to re-upload. No device
        // is attached here, so this is a CPU-level churn proof: the runtime has no
        // suffix commit to make and the counter stays flat.
        session.apply_ready_updates().unwrap();
        assert_eq!(
            session.overlay_suffix_commits(),
            suffix_before,
            "a camera-only step must not churn the overlay suffix"
        );
    }

    assert_eq!(
        session.input_revision(),
        input_before,
        "camera-only changes must not bump the scene-identity token"
    );
    assert!(session.view_revision() > view_before, "the view advanced");
    assert!(
        session.overlay_revision() > overlay_before,
        "the camera-derived viewport bounds advanced the reference geometry"
    );
}

/// PURE-STATE (no GPU): the viewport-bounds change still schedules a background
/// re-prepare that republishes the refined grid while reusing the base.
#[test]
fn camera_only_viewport_change_reprepares_grid_without_invalidating_scene() {
    use std::time::{Duration, Instant};

    let session = session_with_published_base();
    let before = session.current_prepared_scene().expect("published scene");
    let input_before = session.input_revision();

    let mut view = session.view_snapshot();
    view.camera.target.x += 400.0;
    session.apply_view_snapshot(view).unwrap();

    assert!(
        session.preparation_current(),
        "the scene stays current through the camera-only change"
    );
    assert_eq!(session.input_revision(), input_before);
    assert!(!session.overlays_suppressed());

    // The background re-prepare runs for the new viewport bounds.
    session.request_preparation().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let refined = loop {
        session.poll_preparation().unwrap();
        let ready = session.current_prepared_scene().expect("ready scene");
        if ready.revision > before.revision {
            break ready;
        }
        assert!(
            Instant::now() < deadline,
            "grid-extent refinement timed out"
        );
        std::thread::yield_now();
    };

    assert_eq!(
        refined.base_revision, before.base_revision,
        "the refinement must not replace the published base"
    );
    assert_ne!(
        refined.highlight.added, before.highlight.added,
        "the reference grid must reflect the new viewport bounds"
    );
    assert!(session.overlays().viewport_bounds.is_some());
    assert!(session.preparation_current());

    // A second camera step re-prepares again through the same background worker,
    // which must reuse its base `Arc` across camera-only refinements.
    let mut view = session.view_snapshot();
    view.camera.target.x += 400.0;
    session.apply_view_snapshot(view).unwrap();
    session.request_preparation().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let refined_again = loop {
        session.poll_preparation().unwrap();
        let ready = session.current_prepared_scene().expect("ready scene");
        if ready.revision > refined.revision {
            break ready;
        }
        assert!(Instant::now() < deadline, "second refinement timed out");
        std::thread::yield_now();
    };
    assert_eq!(refined_again.base_revision, refined.base_revision);
    assert!(
        Arc::ptr_eq(&refined_again.base, &refined.base),
        "camera-only refinements must reuse the worker's resident base"
    );
    assert!(session.preparation_current());
}
