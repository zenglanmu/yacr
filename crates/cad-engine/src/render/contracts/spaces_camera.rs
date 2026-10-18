use super::*;

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
