use super::*;

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
