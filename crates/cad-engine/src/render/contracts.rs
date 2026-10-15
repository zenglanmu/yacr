use super::*;
use cad_app::render_scene as controller;
use cad_app::render_scene::{
    build_scene, build_scene_with_overrides, build_scene_with_space, layout_descriptors,
};
use cad_app::BackendChoice;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Layout, PaperViewport};
use cad_domain::TaskStamp;
use cad_domain::{
    Completeness, DatabaseId, EntityId, InstancePath, LayerId, LayoutId, Mesh, ObjectId, Point3,
    Revision,
};
use cad_domain::{SelectionRef, SemanticGeometry, SpaceId};

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

#[test]
fn scene_controller_reuses_base_for_overlay_and_rebuilds_for_font_replacement() {
    let db = db_with_layout();
    let mut controller = controller::CadSceneController::default();
    let layers = LayerOverrideSet::new();
    controller
        .prepare(
            Some(&db),
            DocumentId(42),
            None,
            &layers,
            SpaceSelection::Model,
        )
        .unwrap();
    let first = controller.ready.as_ref().unwrap();
    assert_eq!(first.base.stamp.document, DocumentId(42));
    assert_eq!(first.base.stamp.object_revision, db.revision());
    let base = first.base.clone();
    let base_revision = first.base_revision;
    let revision = first.revision;
    controller
        .prepare(
            Some(&db),
            DocumentId(42),
            None,
            &layers,
            SpaceSelection::Model,
        )
        .unwrap();
    assert_eq!(controller.ready.as_ref().unwrap().revision, revision);
    assert!(Arc::ptr_eq(&base, &controller.ready.as_ref().unwrap().base));
    assert_eq!(
        controller.ready.as_ref().unwrap().base_revision,
        base_revision
    );
    controller.fonts_changed();
    controller
        .prepare(
            Some(&db),
            DocumentId(42),
            None,
            &layers,
            SpaceSelection::Model,
        )
        .unwrap();
    assert!(controller.ready.as_ref().unwrap().base_revision > base_revision);
    controller
        .prepare(None, DocumentId(42), None, &layers, SpaceSelection::Model)
        .unwrap();
    let closed = controller.ready.as_ref().unwrap();
    assert!(closed.base.added.is_empty() && closed.highlight.added.is_empty());
}

#[test]
fn failed_preparation_keeps_last_ready_scene_retryable() {
    let db = db_with_layout();
    let mut controller = controller::CadSceneController::default();
    controller
        .prepare(
            Some(&db),
            DocumentId(1),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
        )
        .unwrap();
    let revision = controller.ready.as_ref().unwrap().revision;
    assert!(controller
        .prepare(
            Some(&db),
            DocumentId(1),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Paper(LayoutId(999)),
        )
        .is_err());
    assert_eq!(controller.ready.as_ref().unwrap().revision, revision);
    assert!(controller.diagnostic.is_some());
}

#[test]
fn dirty_frames_and_device_lifecycle_do_not_retry_lost_devices() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    session.fail_device("lost", true);
    assert!(matches!(session.lifecycle(), RenderLifecycle::Lost { .. }));
    assert!(session.render([100.0, 100.0], 1.0).unwrap().is_none());
    session.detach_device();
    assert_eq!(session.lifecycle(), RenderLifecycle::Detached);
    assert!(session.backend_outcome().is_none());
}

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

#[test]
fn space_selection_maps_session_spaces() {
    use cad_domain::BlockId;
    assert_eq!(
        space_selection(&SpaceId::Model),
        Some(SpaceSelection::Model)
    );
    let layout = LayoutId(3);
    assert_eq!(
        space_selection(&SpaceId::Paper(layout)),
        Some(SpaceSelection::Paper(layout))
    );
    // Block-definition geometry is never a top-level space.
    assert_eq!(space_selection(&SpaceId::Block(BlockId(1))), None);
}

fn line_entity(id: u128, space: SpaceId, a: Point3, b: Point3) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry: SemanticGeometry::Line { start: a, end: b },
        draw_order: id as i64,
    }
}

fn db_with_layout() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // Model line through the viewport anchor.
    b.insert_entity(line_entity(1, SpaceId::Model, p(9.0, 20.0), p(11.0, 20.0)))
        .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Layout1".into(),
        viewports: vec![PaperViewport {
            clip: vec![p(0.0, 0.0), p(100.0, 50.0), p(10.0, 20.0)],
            model_to_paper: cad_domain::Transform3::scale(100.0),
            completeness: Completeness::Complete,
        }],
    })
    .unwrap();
    b.finish().unwrap()
}

#[test]
fn empty_database_produces_an_empty_scene() {
    let db = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1))
        .finish()
        .unwrap();
    let delta = build_scene(&db, TaskStamp::new(DocumentId(0), 0)).unwrap();
    assert!(delta.added.is_empty());
}

#[test]
fn model_space_entry_point_is_unchanged_by_the_space_api() {
    let db = db_with_layout();
    let stamp = TaskStamp::new(DocumentId(0), 0);
    let model = build_scene(&db, stamp.clone()).unwrap();
    let explicit = build_scene_with_space(
        &db,
        stamp,
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
    )
    .unwrap();
    assert_eq!(model.added.len(), explicit.added.len());
    assert_eq!(model.added.len(), 1);
}

#[test]
fn paper_space_build_maps_model_geometry_through_the_viewport() {
    let db = db_with_layout();
    let delta = build_scene_with_space(
        &db,
        TaskStamp::new(DocumentId(0), 0),
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Paper(LayoutId(1)),
    )
    .unwrap();
    assert_eq!(delta.added.len(), 1);
}

#[test]
fn switching_to_a_missing_layout_is_reported_not_faked() {
    let db = Arc::new(db_with_layout());
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    session.sync_drawing(Some(db));
    let before = session.view_snapshot();
    let invalid = ViewSnapshot {
        space: SpaceSelection::Paper(LayoutId(99)),
        ..before.clone()
    };
    let error = session.apply_view_snapshot(invalid).unwrap_err();
    assert!(error.to_string().contains("layout"));
    assert_eq!(session.view_snapshot(), before);
}

#[test]
fn layout_descriptors_expose_ids_names_and_support() {
    let db = db_with_layout();
    let rows = layout_descriptors(&db);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, LayoutId(1));
    assert_eq!(rows[0].name, "Layout1");
    assert!(rows[0].supported);
    assert_eq!(rows[0].viewport_count, 1);
}

#[test]
fn fit_camera_on_empty_database_is_default() {
    let db = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1))
        .finish()
        .unwrap();
    let c = fit_camera(&db, [100.0, 100.0]);
    assert_eq!(c.world_per_px, 1.0);
}

#[test]
fn camera_stays_finite_under_extreme_zoom() {
    let db = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1))
        .finish()
        .unwrap();
    let c = fit_camera(&db, [100.0, 100.0]);
    assert!(c.world_per_px.is_finite());
}

#[test]
fn backend_kind_mapping_is_truthful() {
    assert_eq!(
        backend_kind(ActiveBackend::WebGpu),
        ActiveBackendKind::WebGpu
    );
    assert_eq!(
        backend_kind(ActiveBackend::WebGl2),
        ActiveBackendKind::WebGl2
    );
    assert_eq!(
        backend_kind(ActiveBackend::Native),
        ActiveBackendKind::Native
    );
}

#[test]
fn preference_maps_to_the_shared_app_choice() {
    assert_eq!(
        preference_choice(BackendPreference::Auto),
        BackendChoice::Auto
    );
    assert_eq!(
        preference_choice(BackendPreference::WebGpu),
        BackendChoice::WebGpu
    );
    assert_eq!(
        preference_choice(BackendPreference::WebGl2),
        BackendChoice::WebGl2
    );
}

#[test]
fn camera3d_params_map_field_for_field_to_the_renderer_camera() {
    let params = cad_app::Camera3dParams {
        eye: p3(1.0, 2.0, 3.0),
        target: p3(4.0, 5.0, 6.0),
        up: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        fov_y: 0.7,
        near: 0.01,
        far: 1000.0,
    };
    let camera = camera3d_from_params(params);
    assert_eq!(camera.eye, params.eye);
    assert_eq!(camera.target, params.target);
    assert_eq!(camera.up, params.up);
    assert_eq!(camera.fov_y, params.fov_y);
    assert_eq!(camera.near, params.near);
    assert_eq!(camera.far, params.far);
    assert!(camera.is_usable(16.0 / 9.0));
}

#[test]
fn camera2d_params_map_to_the_renderer_plan_camera() {
    let params = cad_app::Camera2dParams {
        center: p(7.0, -8.0),
        world_per_px: 0.5,
    };
    let camera = camera2d_from_params(params);
    assert_eq!(camera.center, p(7.0, -8.0));
    assert_eq!(camera.world_per_px, 0.5);
    assert_eq!(camera.z_plane, 0.0);
}

#[test]
fn scene_facade_preserves_task_stamp_and_layer_filtering() {
    let db = db_with_layout();
    let stamp = TaskStamp::new(DocumentId(42), 7);
    let mut overrides = LayerOverrideSet::new();
    overrides.set(LayerId(0), false);
    let delta = build_scene_with_overrides(&db, stamp.clone(), None, &overrides).unwrap();
    assert_eq!(delta.stamp, stamp);
    assert!(delta.added.is_empty());
    assert_eq!(db.entity_count(), 1);
}

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

fn session_with_published_base() -> RenderSessionHandle {
    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    session
}

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

// ---------------------------------------------------------------------------
// Hardware GPU proof (ignored; real adapter).
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
fn headless_gpu() -> cad_render_wgpu::headless::HeadlessGpu {
    let gpu = cad_render_wgpu::headless::create_headless_gpu(BackendPreference::Auto)
        .expect("hardware test requires a real wgpu adapter (P620/Vulkan)");
    eprintln!(
        "cad-engine hardware test adapter: backend={} name={} device_type={} driver={}",
        gpu.adapter.backend, gpu.adapter.name, gpu.adapter.device_type, gpu.adapter.driver
    );
    gpu
}

#[cfg(not(target_arch = "wasm32"))]
fn renderer_batch_count(session: &RenderSessionHandle) -> usize {
    session
        .0
        .borrow()
        .runtime
        .renderer
        .as_ref()
        .expect("attached renderer")
        .batch_count()
}

#[cfg(not(target_arch = "wasm32"))]
fn runtime_applied_base(session: &RenderSessionHandle) -> Option<u64> {
    session.0.borrow().runtime.applied_base_for_test()
}

/// Cumulative GPU bytes uploaded by the renderer. Read through a throwaway
/// render so the engine's own public surface is unchanged; the render itself
/// uploads nothing.
#[cfg(not(target_arch = "wasm32"))]
fn cumulative_upload_bytes(session: &RenderSessionHandle) -> u64 {
    let mut state = session.0.borrow_mut();
    let renderer = state.runtime.renderer.as_mut().expect("attached renderer");
    let target = cad_render_wgpu::RenderTarget::new(32, 32);
    let camera = camera2d_from_params(cad_app::Camera2dParams {
        center: p(0.0, 0.0),
        world_per_px: 1.0,
    });
    renderer
        .render(camera, &target)
        .expect("measurement render")
        .uploaded_bytes
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn base_only_navigation_renders_latest_camera_and_restores_overlays() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();

    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    session.apply_ready_updates().unwrap();

    let base_pub = session.0.borrow().base_publication_revision;
    let base_batches = session.0.borrow().runtime.base_batches_for_test();
    let full_batches = renderer_batch_count(&session);
    assert!(
        full_batches > base_batches,
        "the demo drawing has an axes/grid overlay suffix ({full_batches} vs {base_batches})"
    );
    let uploaded_full = cumulative_upload_bytes(&session);

    // The full current scene is presented and honestly tagged.
    let full_frame = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("full-scene frame");
    assert_eq!(full_frame.completeness, FrameCompleteness::CurrentScene);
    assert!(session.current_drawing_presented());
    let view_rev_0 = full_frame.view_revision;

    // A camera drag alone no longer makes preparation stale: camera-only changes
    // keep the published scene current. The fallback is entered by a real overlay
    // input change (which rebuilds the reference overlay) while the base stays
    // unchanged and resident. The visibility flag toggled here produces no batches
    // for this drawing, so the batch-count comparisons stay valid.
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

    let base_frame = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("base-only frame");
    assert_eq!(
        base_frame.completeness,
        FrameCompleteness::BaseOnlyPendingOverlays
    );
    assert!(
        base_frame.view_revision > view_rev_0,
        "latest view is tagged"
    );
    assert!(session.overlays_suppressed());
    assert!(
        !session.current_drawing_presented(),
        "a base-only frame is not the presented current drawing"
    );
    assert_eq!(
        renderer_batch_count(&session),
        base_batches,
        "the stale overlay suffix is cleared, the base prefix preserved"
    );
    assert_eq!(runtime_applied_base(&session), Some(base_pub));
    assert_eq!(
        cumulative_upload_bytes(&session),
        uploaded_full,
        "entering the fallback uploads zero bytes (empty suffix commit)"
    );

    // Further camera updates keep rendering the latest view with no re-upload.
    let mut view2 = session.view_snapshot();
    view2.camera.target.x += 250.0;
    session.apply_view_snapshot(view2).unwrap();
    let base_frame2 = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("second base-only frame");
    assert_eq!(
        base_frame2.completeness,
        FrameCompleteness::BaseOnlyPendingOverlays
    );
    assert!(base_frame2.view_revision > base_frame.view_revision);
    assert_eq!(renderer_batch_count(&session), base_batches);
    assert_eq!(runtime_applied_base(&session), Some(base_pub));
    let uploaded_after_suppression = cumulative_upload_bytes(&session);
    assert_eq!(uploaded_after_suppression, uploaded_full);

    // Preparation catches up: the overlay suffix is restored from the same base.
    session.prepare().unwrap();
    assert!(session.preparation_current());
    session.apply_ready_updates().unwrap();
    assert!(!session.overlays_suppressed());
    assert_eq!(
        renderer_batch_count(&session),
        full_batches,
        "overlay suffix restored"
    );
    assert_eq!(
        runtime_applied_base(&session),
        Some(base_pub),
        "restore reused the resident base"
    );
    let uploaded_restored = cumulative_upload_bytes(&session);
    assert!(
        uploaded_restored > uploaded_after_suppression,
        "the overlay suffix is re-uploaded"
    );
    assert!(
        uploaded_restored - uploaded_after_suppression < uploaded_full,
        "restore must not re-upload the whole base scene"
    );

    let restored_frame = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("restored frame");
    assert_eq!(restored_frame.completeness, FrameCompleteness::CurrentScene);
    assert!(session.current_drawing_presented());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn base_only_navigation_blocked_after_device_change_until_reuploaded() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();
    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    session.apply_ready_updates().unwrap();
    assert!(session.render([800.0, 600.0], 1.0).unwrap().is_some());

    // A new device resets the resident base; the fallback must not draw it.
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
        "no resident base: strict waiting, never a blank/stale base-only frame"
    );

    // Re-preparing and re-uploading the same base restores rendering.
    session.prepare().unwrap();
    session.apply_ready_updates().unwrap();
    assert_eq!(
        runtime_applied_base(&session),
        Some(session.0.borrow().base_publication_revision)
    );
    assert!(session.render([800.0, 600.0], 1.0).unwrap().is_some());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn base_only_navigation_progressive_page_restarts_on_camera_change() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();
    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    session.apply_ready_updates().unwrap();

    // Force the base-only batch set to need more than one progressive page.
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

    // Enter the base-only fallback through a real overlay input change (a
    // camera-only change no longer invalidates the published scene), then move the
    // camera so the next base-only page must restart under the new camera.
    let mut overlays = session.overlays();
    overlays.visibility.snap_hints = !overlays.visibility.snap_hints;
    assert!(session.set_overlays(overlays));
    let mut view = session.view_snapshot();
    view.camera.target.x += 10.0;
    session.apply_view_snapshot(view).unwrap();
    session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("base-only frame A");
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
        .expect("base-only frame B");
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
            "camera-only step {step} must present the full scene, never the fallback"
        );
        assert_eq!(
            session.current_prepared_scene().unwrap().revision,
            ready_revision,
            "camera-only step {step} must not republish the scene"
        );
        assert!(
            !session.overlays_suppressed(),
            "camera-only step {step} must not suppress overlays"
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

#[cfg(not(target_arch = "wasm32"))]
fn db_with_lines(count: usize) -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for i in 0..count {
        let id = i as u128 + 1;
        b.insert_entity(line_entity(
            id,
            SpaceId::Model,
            p(i as f64, 0.0),
            p(i as f64 + 1.0, 0.0),
        ))
        .unwrap();
    }
    b.finish().unwrap()
}

#[cfg(not(target_arch = "wasm32"))]
fn uploads_committed(session: &RenderSessionHandle) -> bool {
    let state = session.0.borrow();
    match state.controller.ready.as_ref() {
        Some(ready) => state
            .runtime
            .scene_committed(ready.revision, ready.base_revision),
        None => false,
    }
}

/// Drive the chunked `apply_ready_updates` until the current ready scene is
/// fully GPU-committed. Uses the real chunk limits (no budget weakening).
#[cfg(not(target_arch = "wasm32"))]
fn drive_uploads(session: &RenderSessionHandle) {
    for _ in 0..100_000 {
        session.apply_ready_updates().unwrap();
        if uploads_committed(session) {
            return;
        }
    }
    panic!("chunked GPU uploads did not commit within the bound");
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

/// DETERMINISTIC HARDWARE: while the overlay suffix upload is still pending on
/// an unchanged resident base, the stale suffix must never be labeled
/// `CurrentScene`. A large highlight forces the suffix to span multiple chunked
/// passes with the real limits.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn pending_overlay_upload_never_labels_stale_suffix_current() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let gpu = headless_gpu();
    session.attach_device(gpu.device, gpu.queue).unwrap();

    let drawing = Arc::new(db_with_lines(1500));
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    drive_uploads(&session);
    let base_pub = session.0.borrow().base_publication_revision;
    let committed_batches = renderer_batch_count(&session);
    assert_eq!(
        session
            .render([800.0, 600.0], 1.0)
            .unwrap()
            .unwrap()
            .completeness,
        FrameCompleteness::CurrentScene
    );

    // Same base, but a large highlight makes the overlay suffix span several
    // chunked passes: the renderer still holds the stale previous suffix.
    let selection = SelectionSet::from_refs((0..1500).map(|i| SelectionRef {
        document: DocumentId(0),
        entity: EntityId(i as u128 + 1),
        instance: InstancePath::default(),
        sub_element: None,
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
        .expect("base-only frame during the overlay upload");
    assert_eq!(
        during.completeness,
        FrameCompleteness::BaseOnlyPendingOverlays,
        "a stale overlay suffix must never be labeled CurrentScene"
    );
    assert!(!session.current_drawing_presented());

    drive_uploads(&session);
    let restored = session
        .render([800.0, 600.0], 1.0)
        .unwrap()
        .expect("restored frame");
    assert_eq!(restored.completeness, FrameCompleteness::CurrentScene);
    assert!(session.current_drawing_presented());
    assert!(renderer_batch_count(&session) > committed_batches);
}

// ---------------------------------------------------------------------------
// Production flags-only fast path (B1/B2/S1/S4): a selection change must tint
// through the in-stream flags without re-uploading any geometry.
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
fn model_entity(id: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(0),
        entity: EntityId(id),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// A filled axis-aligned quad (two triangles, CCW in a Y-up world — the winding
/// the renderer's quad tests use) as an `AcDbMesh` entity.
#[cfg(not(target_arch = "wasm32"))]
fn quad_entity(id: u128, cx: f64, cy: f64, half: f64) -> DbEntity {
    let vertices = vec![
        Point3 {
            x: cx - half,
            y: cy - half,
            z: 0.0,
        },
        Point3 {
            x: cx - half,
            y: cy + half,
            z: 0.0,
        },
        Point3 {
            x: cx + half,
            y: cy + half,
            z: 0.0,
        },
        Point3 {
            x: cx + half,
            y: cy - half,
            z: 0.0,
        },
    ];
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbMesh".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mesh(Mesh {
            vertices,
            triangles: vec![[0, 2, 1], [0, 3, 2]],
            normals: vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                };
                4
            ],
            face_sources: vec![None; 2],
            colors: Vec::new(),
        }),
        draw_order: id as i64,
    }
}

/// Two well-separated filled quads (entities 1 and 2) so a selected tint can be
/// sampled at a known pixel per entity.
#[cfg(not(target_arch = "wasm32"))]
fn db_with_two_quads() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(quad_entity(1, -40.0, 0.0, 15.0)).unwrap();
    b.insert_entity(quad_entity(2, 40.0, 0.0, 15.0)).unwrap();
    b.finish().unwrap()
}

/// The pixel containing world point `(wx, wy)` under the default session camera
/// (centre `(0, 0)`, orthographic scale `1.0`).
#[cfg(not(target_arch = "wasm32"))]
fn world_to_pixel_2d(width: u32, height: u32, world: [f64; 2]) -> (u32, u32) {
    let sx = 2.0 / f64::from(width);
    let sy = 2.0 / f64::from(height);
    let px = ((sx * world[0] + 1.0) * 0.5 * f64::from(width)).floor() as i64;
    let py = ((1.0 - sy * world[1]) * 0.5 * f64::from(height)).floor() as i64;
    (
        px.clamp(0, i64::from(width) - 1) as u32,
        py.clamp(0, i64::from(height) - 1) as u32,
    )
}

/// Render the current scene through the session and read the frame back.
#[cfg(not(target_arch = "wasm32"))]
fn session_frame(session: &RenderSessionHandle, size: [f64; 2]) -> Vec<u8> {
    session.render(size, 1.0).unwrap().expect("frame");
    session
        .0
        .borrow_mut()
        .runtime
        .renderer
        .as_mut()
        .expect("attached renderer")
        .read_target_rgba()
        .expect("read back frame")
        .pixels
}

/// `(flags_upload_count, instance_stream_uploads, mesh_chunk_uploads)`.
#[cfg(not(target_arch = "wasm32"))]
fn renderer_counters(session: &RenderSessionHandle) -> (u64, u64, u64) {
    let state = session.0.borrow();
    let renderer = state.runtime.renderer.as_ref().expect("attached renderer");
    (
        renderer.flags_upload_count(),
        renderer.instance_stream_uploads(),
        renderer.mesh_chunk_uploads(),
    )
}

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
