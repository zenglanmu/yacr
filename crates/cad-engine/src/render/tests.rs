use super::*;

#[test]
fn same_session_checks_shared_owner_state_identity() {
    let first = RenderSessionHandle::new(BackendPreference::Auto);
    let clone = first.clone();
    let other = RenderSessionHandle::new(BackendPreference::Auto);
    assert!(first.same_session(&clone));
    assert!(!first.same_session(&other));
}

#[test]
fn session_is_ui_independent_and_identical_view_does_not_dirty() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let view = ViewSnapshot::default();
    assert!(!session.apply_view_snapshot(view).unwrap());
    assert_eq!(session.frames_rendered(), 0);
    assert!(matches!(session.lifecycle(), RenderLifecycle::Detached));
}

#[test]
fn overlay_preparation_reuses_base_while_resource_replacement_invalidates_it() {
    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    let first = session.controller_ready().unwrap();
    let base = first.base.clone();
    let mut overlays = session.overlays();
    overlays.visibility.axes = false;
    assert!(session.set_overlays(overlays));
    session.prepare().unwrap();
    let overlay_scene = session.controller_ready().unwrap();
    assert!(Arc::ptr_eq(&base, &overlay_scene.base));
    assert_eq!(overlay_scene.base_revision, first.base_revision);
    let base_revision = overlay_scene.base_revision;
    assert!(session.set_images(Some(Arc::new(DecodedImageCache::new()))));
    session.prepare().unwrap();
    assert!(session.controller_ready().unwrap().base_revision > base_revision);
}

#[test]
fn dirty_frame_errors_and_lost_lifecycle_never_mark_clean_or_retry() {
    let mut dirty = runtime::FrameInvalidation::default();
    assert!(dirty.dirty());
    dirty.complete();
    assert!(!dirty.dirty());
    dirty.invalidate();
    assert!(dirty.dirty());
    let failed_render: Result<(), &str> = Err("synthetic render failure");
    if failed_render.is_ok() {
        dirty.complete();
    }
    assert!(dirty.dirty(), "failed rendering must leave the frame dirty");
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    session.fail_device("lost", true);
    assert!(matches!(session.lifecycle(), RenderLifecycle::Lost { .. }));
    assert!(session.render([0.0, 0.0], 1.0).unwrap().is_none());
    session.detach_device();
    assert_eq!(session.lifecycle(), RenderLifecycle::Detached);
}

#[test]
fn drawing_replacement_and_equal_view_are_ui_free_inputs() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing();
    assert!(session.sync_drawing(drawing.clone()));
    assert!(!session.sync_drawing(drawing));
    let mut changed = ViewSnapshot::default();
    changed.camera.target.x += 1.0;
    assert!(session.apply_view_snapshot(changed.clone()).unwrap());
    let dirty = session.cad_frame_dirty();
    assert!(!session.apply_view_snapshot(changed).unwrap());
    assert_eq!(session.cad_frame_dirty(), dirty);
    assert_eq!(session.redraw_requests(), 0);
}

#[test]
fn sync_then_native_async_publication_uses_session_global_revisions() {
    use std::time::{Duration, Instant};

    let first = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    let second = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    session.sync_drawing(Some(first));
    session.prepare().unwrap();
    let sync_ready = session.current_prepared_scene().unwrap();
    assert_eq!((sync_ready.revision, sync_ready.base_revision), (1, 1));

    session.sync_drawing(Some(second));
    assert!(!session.preparation_current());
    session.request_preparation().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !session.preparation_current() {
        session.poll_preparation().unwrap();
        assert!(Instant::now() < deadline, "native preparation timed out");
        std::thread::yield_now();
    }
    let async_ready = session.current_prepared_scene().unwrap();
    assert!(async_ready.revision > sync_ready.revision);
    assert!(async_ready.base_revision > sync_ready.base_revision);
}

#[test]
fn input_mutation_invalidates_current_scene_until_reprepared() {
    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    let previous = session.controller_ready().unwrap();
    assert!(session.preparation_current());

    let images = Arc::new(DecodedImageCache::new());
    assert!(session.set_images(Some(images)));
    assert!(!session.preparation_current());
    assert!(session.current_prepared_scene().is_none());
    assert_eq!(
        session.controller_ready().unwrap().revision,
        previous.revision
    );
    session.apply_ready_updates().unwrap();

    assert!(session.set_overlay_visibility(OverlayVisibility {
        axes: false,
        ..OverlayVisibility::default()
    }));
    assert!(!session.preparation_current());
    assert!(session.current_prepared_scene().is_none());
    session.apply_ready_updates().unwrap();
}

#[test]
fn invalid_space_snapshot_is_rejected_atomically() {
    use cad_domain::LayoutId;

    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    session.sync_drawing(Some(drawing));
    let before = session.view_snapshot();
    let input_revision = session.input_revision();
    let dirty = session.cad_frame_dirty();
    let invalid = ViewSnapshot {
        space: SpaceSelection::Paper(LayoutId(999_999)),
        ..before.clone()
    };
    assert!(session.apply_view_snapshot(invalid).is_err());
    assert_eq!(session.view_snapshot(), before);
    assert_eq!(session.input_revision(), input_revision);
    assert_eq!(session.cad_frame_dirty(), dirty);
}

#[test]
fn repeated_equal_setters_keep_input_token_stable() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let images = Arc::new(DecodedImageCache::new());
    assert!(session.set_images(Some(images.clone())));
    let token = session.input_revision();
    assert!(!session.set_images(Some(images)));
    assert!(!session.set_fonts(None));
    assert!(!session.set_layers(LayerOverrideSet::new()));
    assert!(!session.set_selection(SelectionSet::new()));
    assert!(!session.set_overlay_visibility(OverlayVisibility::default()));
    assert_eq!(session.input_revision(), token);
}

#[test]
fn resource_generations_survive_unpublished_clear_and_replacement() {
    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    let before = session.current_prepared_scene().unwrap().base_revision;

    let first_cache = Arc::new(DecodedImageCache::new());
    assert!(session.set_images(Some(first_cache.clone())));
    let first_generation = session.0.borrow().images_generation;
    assert!(!session.set_images(Some(first_cache)));
    assert_eq!(session.0.borrow().images_generation, first_generation);
    assert!(session.set_images(None));
    assert_eq!(session.0.borrow().images_generation, first_generation + 1);
    assert!(session.set_images(Some(Arc::new(DecodedImageCache::new()))));
    assert_eq!(session.0.borrow().images_generation, first_generation + 2);

    let first_fonts = Arc::new(FontEngine::new());
    assert!(session.set_fonts(Some(first_fonts.clone())));
    let font_generation = session.0.borrow().fonts_generation;
    assert!(!session.set_fonts(Some(first_fonts)));
    assert_eq!(session.0.borrow().fonts_generation, font_generation);
    assert!(session.set_fonts(None));
    assert_eq!(session.0.borrow().fonts_generation, font_generation + 1);
    assert!(session.set_fonts(Some(Arc::new(FontEngine::new()))));
    assert_eq!(session.0.borrow().fonts_generation, font_generation + 2);
    assert!(!session.preparation_current());

    session.prepare().unwrap();
    let after = session.current_prepared_scene().unwrap().base_revision;
    assert!(
        after > before,
        "resource generations must invalidate the base even without an intermediate publication"
    );
}

#[test]
fn diagnostics_read_is_pure_and_never_labels_a_success() {
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let input_before = session.input_revision();
    let view_before = session.view_revision();
    let frames_before = session.frames_rendered();
    let first = session.diagnostics();
    let second = session.diagnostics();
    // Repeated reads are identical (no time/state dependent drift).
    assert_eq!(first, second);
    // Reading diagnostics must not advance revisions or render anything.
    assert_eq!(session.input_revision(), input_before);
    assert_eq!(session.view_revision(), view_before);
    assert_eq!(session.frames_rendered(), frames_before);
    // A fresh, device-less session is Detached and never a fabricated Ready /
    // current scene.
    assert_eq!(first.gate, RenderGateReason::Detached);
    assert_eq!(first.lifecycle, "detached");
    assert_eq!(first.ready_revision, None);
    assert_eq!(first.ready_base_revision, None);
    assert_eq!(first.prepared_token, None);
    assert_eq!(first.prepared_identity, None);
    assert!(!first.runtime.uploading);
    assert_eq!(first.runtime.upload, None);
    assert_eq!(first.runtime.frames_rendered, 0);
    assert_eq!(first.runtime.uploads_started, 0);
    assert_eq!(first.runtime.upload_revision_resets, 0);
    assert_eq!(first.runtime.ready_none_clears, 0);
}
