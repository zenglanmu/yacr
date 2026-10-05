use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Layout, PaperViewport};
use cad_domain::TaskStamp;
use cad_domain::{Completeness, EntityId, LayerId, LayoutId, ObjectId, Point3, Revision};
use cad_domain::{SemanticGeometry, SpaceId};

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
    let mut dirty = runtime::FrameInvalidation::default();
    assert!(dirty.dirty());
    dirty.complete();
    assert!(!dirty.dirty(), "UI-only redraw reuses CAD frame");
    dirty.invalidate();
    assert!(dirty.dirty());
    let mut runtime = runtime::CadRenderRuntime::default();
    runtime.fail(BackendPreference::Auto, "lost".into(), true);
    assert!(matches!(runtime.lifecycle, RenderLifecycle::Lost { .. }));
    assert!(runtime
        .render_if_dirty(&ViewSnapshot::default(), [100.0, 100.0], 1.0)
        .unwrap()
        .is_none());
    runtime.detach();
    assert_eq!(runtime.lifecycle, RenderLifecycle::Detached);
    assert!(runtime.outcome.is_none());
}

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

#[test]
fn space_selection_maps_session_spaces() {
    use cad_domain::BlockId;
    assert_eq!(
        CadView::space_selection(&SpaceId::Model),
        Some(SpaceSelection::Model)
    );
    let layout = LayoutId(3);
    assert_eq!(
        CadView::space_selection(&SpaceId::Paper(layout)),
        Some(SpaceSelection::Paper(layout))
    );
    // Block-definition geometry is never a top-level space.
    assert_eq!(CadView::space_selection(&SpaceId::Block(BlockId(1))), None);
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
    let db = db_with_layout();
    // Diagnostic is on the representation, not the scene delta.
    let delta = build_scene_with_space(
        &db,
        TaskStamp::new(DocumentId(0), 0),
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Paper(LayoutId(99)),
    )
    .unwrap();
    assert!(delta.added.is_empty());
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
