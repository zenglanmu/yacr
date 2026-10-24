use super::*;

// ---------------------------------------------------------------------------
// Hardware GPU proof (ignored; real adapter).
// ---------------------------------------------------------------------------

/// DETERMINISTIC HARDWARE: while newer overlay inputs are still preparing, the
/// session presents the whole resident scene (suffix included, never truncated)
/// over the latest camera and labels the frame `PendingOverlayInputs`. The suffix
/// batch set, the uploaded bytes and the suffix-commit counter all stay flat
/// through the pending window; the publication that follows is one real suffix
/// commit over the same resident base.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn pending_overlay_inputs_keep_the_resident_suffix() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();

    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    // A single chunked pass is not guaranteed to commit under load (the real
    // 8 ms staging budget can break it early); drive the upload to completion so
    // the committed-scene precondition is deterministic.
    drive_uploads(&session);

    let base_pub = session.0.borrow().base_publication_revision;
    let base_batches = session.0.borrow().runtime.base_batches_for_test();
    let full_batches = renderer_batch_count(&session);
    assert!(
        full_batches > base_batches,
        "the demo drawing has an axes/grid overlay suffix ({full_batches} vs {base_batches})"
    );
    let uploaded_full = cumulative_upload_bytes(&session);
    let suffix_commits_full = session.overlay_suffix_commits();

    // The full current scene is presented and honestly tagged.
    let full_frame = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("full-scene frame");
    assert_eq!(full_frame.completeness, FrameCompleteness::CurrentScene);
    assert!(session.current_drawing_presented());
    let view_rev_0 = full_frame.view_revision;
    assert_eq!(session.overlay_suffix_commits(), suffix_commits_full);

    // A camera drag alone no longer makes preparation stale: camera-only changes
    // keep the published scene current. The pending window is entered by a real
    // overlay input change (which rebuilds the reference overlay) while the base
    // stays unchanged and resident. The visibility flag toggled here produces no
    // batches for this drawing, so the batch-count comparisons stay valid.
    let mut view = session.view_snapshot();
    view.camera.target.x += 250.0;
    session.apply_view_snapshot(view.clone()).unwrap();
    assert!(
        session.preparation_current(),
        "a camera-only change must keep the published scene current"
    );
    let mut overlays = session.overlays();
    overlays.visibility.snap_hints = !overlays.visibility.snap_hints;
    assert!(session.set_overlays(overlays));
    assert!(!session.preparation_current());

    let pending_frame = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("pending-overlay frame");
    assert_eq!(
        pending_frame.completeness,
        FrameCompleteness::PendingOverlayInputs,
        "the newest overlay inputs are not presented yet"
    );
    assert!(
        pending_frame.view_revision > view_rev_0,
        "latest view is tagged"
    );
    assert!(
        !session.current_drawing_presented(),
        "a pending-overlay frame is not the presented current drawing"
    );
    assert_eq!(
        renderer_batch_count(&session),
        full_batches,
        "the resident overlay suffix is kept, never truncated"
    );
    assert_eq!(runtime_applied_base(&session), Some(base_pub));
    assert_eq!(
        cumulative_upload_bytes(&session),
        uploaded_full,
        "entering the pending window uploads zero bytes"
    );
    assert_eq!(
        session.overlay_suffix_commits(),
        suffix_commits_full,
        "entering the pending window commits no overlay suffix"
    );

    // Further camera updates keep presenting the latest view over the same
    // resident scene: still no truncation, no upload and no suffix commit.
    let mut view2 = session.view_snapshot();
    view2.camera.target.x += 250.0;
    session.apply_view_snapshot(view2).unwrap();
    let pending_frame2 = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("second pending-overlay frame");
    assert_eq!(
        pending_frame2.completeness,
        FrameCompleteness::PendingOverlayInputs
    );
    assert!(pending_frame2.view_revision > pending_frame.view_revision);
    assert_eq!(renderer_batch_count(&session), full_batches);
    assert_eq!(runtime_applied_base(&session), Some(base_pub));
    assert_eq!(
        cumulative_upload_bytes(&session),
        uploaded_full,
        "the pending window never re-uploads the scene it already holds"
    );
    assert_eq!(
        session.overlay_suffix_commits(),
        suffix_commits_full,
        "the pending window never commits a suffix"
    );

    // Preparation catches up: one real suffix commit over the same resident base,
    // with no truncate/restore churn.
    session.prepare().unwrap();
    assert!(session.preparation_current());
    session.apply_ready_updates().unwrap();
    assert_eq!(
        session.overlay_suffix_commits(),
        suffix_commits_full + 1,
        "the publication is exactly one suffix commit, never truncate + restore"
    );
    assert_eq!(
        renderer_batch_count(&session),
        full_batches,
        "the suffix stays resident through the publication"
    );
    assert_eq!(
        runtime_applied_base(&session),
        Some(base_pub),
        "the publication reused the resident base"
    );
    let uploaded_published = cumulative_upload_bytes(&session);
    assert!(
        uploaded_published > uploaded_full,
        "the changed overlay suffix is uploaded"
    );
    assert!(
        uploaded_published - uploaded_full < uploaded_full,
        "the suffix commit must not re-upload the whole base scene"
    );

    let published_frame = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("published frame");
    assert_eq!(
        published_frame.completeness,
        FrameCompleteness::CurrentScene
    );
    assert!(session.current_drawing_presented());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn resident_scene_presentation_blocked_after_device_change_until_reuploaded() {
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
    assert!(session.render([800.0, 600.0], 1.0).unwrap().is_some());

    // A new device resets the resident base; presenting the resident scene must
    // not draw it.
    session.detach_device();
    let gpu2 = headless_gpu();
    session.attach_device(gpu2.device, gpu2.queue).unwrap();
    assert_eq!(runtime_applied_base(&session), None);
    let mut overlays = session.overlays();
    overlays.visibility.snap_hints = !overlays.visibility.snap_hints;
    assert!(session.set_overlays(overlays));
    assert!(!session.preparation_current());
    assert!(
        session.render([800.0, 600.0], 1.0).unwrap().is_none(),
        "no resident base: strict waiting, never a blank/stale frame"
    );

    // Re-preparing and re-uploading the same base restores rendering.
    session.prepare().unwrap();
    drive_uploads(&session);
    assert_eq!(
        runtime_applied_base(&session),
        Some(session.0.borrow().base_publication_revision)
    );
    assert!(session.render([800.0, 600.0], 1.0).unwrap().is_some());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn pending_overlay_progressive_page_restarts_on_camera_change() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();
    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    // Commit the base deterministically: one chunked pass can leave the real
    // upload pending under load, and the pending-overlay frame needs a resident
    // base.
    drive_uploads(&session);

    // Force the base batch set to need more than one progressive page.
    let scene = session.current_prepared_scene().unwrap();
    let total: usize = scene.base.added.iter().map(|b| b.vertices.len()).sum();
    let max_batch: usize = scene
        .base
        .added
        .iter()
        .map(|b| b.vertices.len())
        .max()
        .unwrap_or(0);
    if total <= max_batch {
        eprintln!("skipping: base scene cannot be split into two honest pages");
        return;
    }
    {
        let mut state = session.0.borrow_mut();
        state
            .runtime
            .renderer
            .as_mut()
            .unwrap()
            .frame_budget
            .max_vertices = total - 1;
    }

    // Enter the pending-overlay window through a real overlay input change (a
    // camera-only change no longer invalidates the published scene), then move the
    // camera so the next progressive page must restart under the new camera.
    let mut overlays = session.overlays();
    overlays.visibility.snap_hints = !overlays.visibility.snap_hints;
    assert!(session.set_overlays(overlays));
    let mut view = session.view_snapshot();
    view.camera.target.x += 10.0;
    session.apply_view_snapshot(view).unwrap();
    session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("pending-overlay frame A");
    let (cursor_a, len_a) = session
        .0
        .borrow()
        .runtime
        .drawing_progress()
        .expect("progressive frame A");
    assert!(cursor_a < len_a, "page A must leave the frame pending");

    let mut view2 = session.view_snapshot();
    view2.camera.target.x += 10.0;
    session.apply_view_snapshot(view2).unwrap();
    session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("pending-overlay frame B");
    let (cursor_b, _) = session
        .0
        .borrow()
        .runtime
        .drawing_progress()
        .expect("progressive frame B");
    assert_eq!(
        cursor_b, cursor_a,
        "a camera change restarts the progressive page instead of continuing the old one"
    );
}

/// DETERMINISTIC HARDWARE: with a fully committed current scene, camera-only pan
/// steps keep the published scene current, present full `CurrentScene` frames,
/// and never re-upload or truncate the overlay suffix.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn camera_only_navigation_does_not_churn_the_overlay_suffix() {
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

    let full_batches = renderer_batch_count(&session);
    let uploaded = cumulative_upload_bytes(&session);
    let suffix_commits = session.overlay_suffix_commits();
    let ready_revision = session.current_prepared_scene().unwrap().revision;

    let first = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("full-scene frame");
    assert_eq!(first.completeness, FrameCompleteness::CurrentScene);

    for step in 1..=4 {
        let mut view = session.view_snapshot();
        view.camera.target.x += 60.0 * step as f64;
        session.apply_view_snapshot(view).unwrap();

        assert!(
            session.preparation_current(),
            "camera-only step {step} must keep the published scene current"
        );
        let frame = session
            .render([800.0, 600.0], 1.0)
            .unwrap()
            .expect("full-scene frame");
        assert_eq!(
            frame.completeness,
            FrameCompleteness::CurrentScene,
            "camera-only step {step} must present the full scene, never a pending-overlay frame"
        );
        assert_eq!(
            session.current_prepared_scene().unwrap().revision,
            ready_revision,
            "camera-only step {step} must not republish the scene"
        );
        assert_eq!(
            renderer_batch_count(&session),
            full_batches,
            "camera-only step {step} must keep the overlay suffix resident"
        );
        assert_eq!(
            cumulative_upload_bytes(&session),
            uploaded,
            "camera-only step {step} must upload zero bytes"
        );
        assert_eq!(
            session.overlay_suffix_commits(),
            suffix_commits,
            "camera-only step {step} must not churn the overlay suffix"
        );
    }
    assert!(session.current_drawing_presented());
}

/// DETERMINISTIC HARDWARE: a CPU-ready new base whose GPU upload is still
/// pending must never be presented as the resident old base (or any stale
/// frame). The real chunked upload leaves `pending_upload` set after one pass
/// because the base needs more than the 4 slices / 256 batches per pass.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn current_cpu_scene_with_uncommitted_new_base_never_presents_old_base() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();

    // A small, fully committed old scene.
    let small = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    session.sync_drawing(Some(small));
    session.prepare().unwrap();
    drive_uploads(&session);
    let old_frame = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("old committed frame");
    assert_eq!(old_frame.completeness, FrameCompleteness::CurrentScene);
    let old_batches = renderer_batch_count(&session);

    // A large new drawing: CPU-ready immediately, GPU upload chunked and pending.
    session.sync_drawing(Some(Arc::new(db_with_lines(1500))));
    session.prepare().unwrap();
    assert!(session.preparation_current());
    session.apply_ready_updates().unwrap();
    {
        let state = session.0.borrow();
        let ready = state.controller.ready.as_ref().unwrap();
        assert!(
            !state
                .runtime
                .scene_committed(ready.revision, ready.base_revision),
            "the new base must still be uncommitted after one chunked pass"
        );
        assert!(
            state.runtime.uploading(),
            "a real (non-mocked) upload must be pending"
        );
    }
    assert!(
        session.render([800.0, 600.0], 1.0).unwrap().is_none(),
        "an uncommitted new base must strict-wait, never the resident old base"
    );
    assert!(!session.current_drawing_presented());

    // Only once the upload commits may the new scene be labeled CurrentScene.
    drive_uploads(&session);
    let new_frame = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("new committed frame");
    assert_eq!(new_frame.completeness, FrameCompleteness::CurrentScene);
    assert!(session.current_drawing_presented());
    assert!(renderer_batch_count(&session) > old_batches);
}

/// `count` separated filled quads, each carrying one stable face source on both
/// of its triangles, so selecting every face as a sub-element produces one
/// overlay batch per entity. More than 1024 of them need more than the four
/// 256-batch staging slices of one chunked pass: the real suffix upload stays
/// pending after the first `apply_ready_updates`.
#[cfg(not(target_arch = "wasm32"))]
fn db_with_face_quads(count: usize) -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for i in 0..count {
        let id = i as u128 + 1;
        let face = cad_domain::SubElementId {
            source_key: format!("face-{id}"),
            topology_revision: Revision(0),
        };
        let mut entity = quad_entity(id, i as f64 * 4.0, 0.0, 1.0);
        match &mut entity.geometry {
            SemanticGeometry::Mesh(mesh) => {
                mesh.face_sources = vec![Some(face); mesh.triangles.len()];
            }
            other => panic!("the quad fixture must be a mesh, got {other:?}"),
        }
        b.insert_entity(entity).unwrap();
    }
    b.finish().unwrap()
}

/// DETERMINISTIC HARDWARE: while an overlay-suffix upload is still pending on an
/// unchanged resident base, the frame must be labeled `PendingOverlayInputs` and
/// must show the **stale suffix that is still resident** — never a truncated
/// base-only scene and never `CurrentScene`. Selecting every mesh face as a
/// sub-element makes the suffix span several chunked passes with the real limits.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn pending_overlay_upload_never_labels_stale_suffix_current() {
    // More than 4 staging slices x 256 batches: one `apply_ready_updates` pass
    // cannot commit this suffix, so the pending state below is deterministic.
    const FACES: usize = 1100;

    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();

    let drawing = Arc::new(db_with_face_quads(FACES));
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    drive_uploads(&session);
    let base_pub = session.0.borrow().base_publication_revision;
    let document = session
        .current_prepared_scene()
        .expect("published scene")
        .base
        .stamp
        .document;
    let committed_batches = renderer_batch_count(&session);
    let base_batches = session.0.borrow().runtime.base_batches_for_test();
    let suffix_commits_before = session.overlay_suffix_commits();
    assert_ne!(
        committed_batches, base_batches,
        "this fixture must have a real overlay suffix, otherwise the check below is vacuous"
    );
    assert_eq!(
        session
            .render([800.0, 600.0], 1.0)
            .unwrap()
            .unwrap()
            .completeness,
        FrameCompleteness::CurrentScene
    );

    // Same base, but selecting every face makes the overlay suffix span several
    // chunked passes: the renderer still holds the stale previous suffix.
    let selection = SelectionSet::from_refs((0..FACES).map(|i| {
        let id = i as u128 + 1;
        SelectionRef {
            document,
            entity: EntityId(id),
            instance: InstancePath::default(),
            sub_element: Some(cad_domain::SubElementId {
                source_key: format!("face-{id}"),
                topology_revision: Revision(0),
            }),
        }
    }));
    assert!(session.set_selection(selection));
    session.prepare().unwrap();
    session.apply_ready_updates().unwrap();
    {
        let state = session.0.borrow();
        let ready = state.controller.ready.as_ref().unwrap();
        assert_eq!(state.runtime.applied_base_for_test(), Some(base_pub));
        assert!(
            !state
                .runtime
                .scene_committed(ready.revision, ready.base_revision),
            "the overlay suffix must still be uncommitted"
        );
        assert!(
            state.runtime.uploading(),
            "a real overlay upload is pending"
        );
    }
    let during = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("pending-overlay frame during the overlay upload");
    assert_eq!(
        during.completeness,
        FrameCompleteness::PendingOverlayInputs,
        "a stale overlay suffix must never be labeled CurrentScene"
    );
    assert_eq!(
        renderer_batch_count(&session),
        committed_batches,
        "the pending frame shows the stale suffix still resident, never a truncated base"
    );
    assert_eq!(
        session.overlay_suffix_commits(),
        suffix_commits_before,
        "a staged (uncommitted) suffix upload must not count as a suffix commit"
    );
    assert!(!session.current_drawing_presented());

    drive_uploads(&session);
    let restored = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("restored frame");
    assert_eq!(restored.completeness, FrameCompleteness::CurrentScene);
    assert!(session.current_drawing_presented());
    assert_eq!(
        session.overlay_suffix_commits(),
        suffix_commits_before + 1,
        "the pending upload commits exactly once"
    );
    assert!(renderer_batch_count(&session) > committed_batches);
}
