use super::*;
use crate::{host::HostController, Command, CommandId, CommandPayload, RawCommandTrace};
use cad_domain::ViewportId;

fn transition() -> (
    Arc<DrawingDatabase>,
    Arc<DrawingDatabase>,
    Vec<RawCommandTrace>,
) {
    let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let before = host.drawing().unwrap();
    let traced = host
        .execute_traced(Command {
            schema_version: 1,
            id: CommandId::CreateLine,
            document: DocumentId(1),
            viewport: ViewportId(1),
            payload: CommandPayload::Points(vec![
                Point3 {
                    x: 2.0,
                    y: 3.0,
                    z: 0.0,
                },
                Point3 {
                    x: 4.0,
                    y: 5.0,
                    z: 0.0,
                },
            ]),
        })
        .unwrap();
    traced.result.unwrap();
    (before, host.drawing().unwrap(), traced.commands)
}

fn prepare(
    controller: &mut CadSceneController,
    db: Arc<DrawingDatabase>,
    edit: Option<&DrawingEditReceipt>,
) {
    controller
        .prepare_shared_with_overlays_and_images_and_edit(
            Some(db),
            DocumentId(1),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
            &OverlayInputs::default(),
            None,
            edit,
        )
        .unwrap();
}

#[test]
fn line_append_matches_full_assembly_and_retains_predecessor_across_overlay() {
    let (before, after, trace) = transition();
    let receipt =
        DrawingEditReceipt::from_create_line_trace(before.clone(), after.clone(), &trace).unwrap();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before, None);
    let predecessor = controller.ready.as_ref().unwrap().base.clone();
    prepare(&mut controller, after.clone(), Some(&receipt));
    let ready = controller.ready.as_ref().unwrap();
    let append = ready.base_append.as_ref().expect("authorized append");
    assert!(Arc::ptr_eq(&append.predecessor, &predecessor));
    assert_eq!(append.keep_batches, predecessor.added.len());
    let full = build_scene(&after, ready.base.stamp.clone()).unwrap();
    assert_eq!(ready.base.added, full.added);
    assert_eq!(ready.base.images, full.images);
    assert_eq!(ready.base.masks, full.masks);
    let mut overlays = OverlayInputs::default();
    overlays.visibility.axes = !overlays.visibility.axes;
    controller
        .prepare_shared_with_overlays_and_images_and_edit(
            Some(after),
            DocumentId(1),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
            &overlays,
            None,
            Some(&receipt),
        )
        .unwrap();
    assert!(Arc::ptr_eq(
        &controller
            .ready
            .as_ref()
            .unwrap()
            .base_append
            .as_ref()
            .unwrap()
            .predecessor,
        &predecessor
    ));
}

#[test]
fn line_append_rejects_wrong_predecessor_and_font_changes() {
    for font_change in [false, true] {
        let (before, after, trace) = transition();
        let receipt =
            DrawingEditReceipt::from_create_line_trace(before.clone(), after.clone(), &trace)
                .unwrap();
        let mut controller = CadSceneController::default();
        let adopted = if font_change {
            before
        } else {
            Arc::new(before.as_ref().clone())
        };
        prepare(&mut controller, adopted, None);
        if font_change {
            controller.fonts_changed();
        }
        prepare(&mut controller, after.clone(), Some(&receipt));
        let ready = controller.ready.as_ref().unwrap();
        assert!(ready.base_append.is_none());
        assert_eq!(
            ready.base.added,
            build_scene(&after, ready.base.stamp.clone()).unwrap().added
        );
    }
}

#[test]
fn receipt_requires_actual_single_create_line_trace_and_revision() {
    let (before, after, trace) = transition();
    assert!(
        DrawingEditReceipt::from_create_line_trace(before.clone(), after.clone(), &[]).is_none()
    );
    assert!(DrawingEditReceipt::from_create_line_trace(
        before.clone(),
        after.clone(),
        &[trace[0].clone(), trace[0].clone()]
    )
    .is_none());
    let mut wrong = trace.clone();
    wrong[0].command = CommandId::CreateCircle;
    assert!(
        DrawingEditReceipt::from_create_line_trace(before.clone(), after.clone(), &wrong).is_none()
    );
    assert!(
        DrawingEditReceipt::from_create_line_trace(before.clone(), before.clone(), &trace)
            .is_none()
    );
    assert!(DrawingEditReceipt::from_create_line_trace(after.clone(), after, &trace).is_none());
}

fn issued_edit(before: Arc<DrawingDatabase>) -> (Arc<DrawingDatabase>, DrawingEditReceipt) {
    let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    host.application
        .workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = before;
    let (trace, receipt) = host
        .execute_traced_with_edit(Command {
            schema_version: 1,
            id: CommandId::CreateLine,
            document: DocumentId(1),
            viewport: ViewportId(1),
            payload: CommandPayload::Points(vec![
                Point3 {
                    x: 2.0,
                    y: 3.0,
                    z: 0.0,
                },
                Point3 {
                    x: 4.0,
                    y: 5.0,
                    z: 0.0,
                },
            ]),
        })
        .unwrap();
    trace.result.unwrap();
    (host.drawing().unwrap(), receipt.unwrap())
}

#[test]
fn published_base_not_last_adopted_document_authorizes_append() {
    let (before, after, trace) = transition();
    let receipt =
        DrawingEditReceipt::from_create_line_trace(before.clone(), after.clone(), &trace).unwrap();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before, None);
    // A borrowed preparation replaces the base without adopting a shared Arc.
    controller
        .prepare(
            Some(&after),
            DocumentId(1),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
        )
        .unwrap();
    prepare(&mut controller, after.clone(), Some(&receipt));
    let ready = controller.ready.as_ref().unwrap();
    assert!(ready.base_append.is_none());
    assert_eq!(
        ready.base.added,
        build_scene(&after, ready.base.stamp.clone()).unwrap().added
    );

    let (next, receipt) = issued_edit(after.clone());
    let mut controller = CadSceneController::default();
    prepare(&mut controller, receipt.before.clone(), None);
    // Adoption of B fails: the published base still belongs to A.
    assert!(controller
        .prepare_shared(
            Some(next.clone()),
            DocumentId(1),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Paper(cad_domain::LayoutId(99999))
        )
        .is_err());
    // Use a different B -> C receipt: A must not be treated as B's base.
    let (third, third_receipt) = issued_edit(next);
    prepare(&mut controller, third.clone(), Some(&third_receipt));
    assert!(controller.ready.as_ref().unwrap().base_append.is_none());
    assert_eq!(
        controller.ready.as_ref().unwrap().base.added,
        build_scene(
            &third,
            controller.ready.as_ref().unwrap().base.stamp.clone()
        )
        .unwrap()
        .added
    );
}

#[test]
fn sequential_appends_bind_each_immediate_predecessor() {
    let (before, _, _) = transition();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before.clone(), None);
    let mut previous = controller.ready.as_ref().unwrap().base.clone();
    let mut drawing = before;
    for _ in 0..2 {
        let (after, receipt) = issued_edit(drawing);
        assert!(Arc::ptr_eq(receipt.after(), &after));
        prepare(&mut controller, after.clone(), Some(&receipt));
        let ready = controller.ready.as_ref().unwrap();
        assert!(Arc::ptr_eq(
            &ready.base_append.as_ref().unwrap().predecessor,
            &previous
        ));
        assert_eq!(
            ready.base.added,
            build_scene(&after, ready.base.stamp.clone()).unwrap().added
        );
        assert_eq!(ready.base.stamp.object_revision, after.revision());
        previous = ready.base.clone();
        drawing = after;
    }
}

#[test]
fn hidden_line_appends_zero_batches_and_resource_changes_fall_back() {
    let (before, after, trace) = transition();
    let receipt =
        DrawingEditReceipt::from_create_line_trace(before.clone(), after.clone(), &trace).unwrap();
    let layer = after.entity(receipt.inserted).unwrap().layer;
    let mut hidden = LayerOverrideSet::new();
    hidden.set(layer, false);
    let mut controller = CadSceneController::default();
    controller
        .prepare_shared(
            Some(before.clone()),
            DocumentId(1),
            None,
            &hidden,
            SpaceSelection::Model,
        )
        .unwrap();
    let count = controller.ready.as_ref().unwrap().base.added.len();
    controller
        .prepare_shared_with_overlays_and_images_and_edit(
            Some(after.clone()),
            DocumentId(1),
            None,
            &hidden,
            SpaceSelection::Model,
            &OverlayInputs::default(),
            None,
            Some(&receipt),
        )
        .unwrap();
    let ready = controller.ready.as_ref().unwrap();
    assert!(ready.base_append.is_some());
    assert_eq!(ready.base.added.len(), count);
    assert_eq!(
        ready.base.added,
        build_scene_with_overrides(&after, ready.base.stamp.clone(), None, &hidden)
            .unwrap()
            .added
    );

    for change_font in [false, true] {
        let mut controller = CadSceneController::default();
        controller
            .prepare_shared(
                Some(before.clone()),
                DocumentId(1),
                Some(Arc::new(FontEngine::new())),
                &hidden,
                SpaceSelection::Model,
            )
            .unwrap();
        let fonts = if change_font {
            Some(Arc::new(FontEngine::new()))
        } else {
            None
        };
        controller
            .prepare_shared_with_overlays_and_images_and_edit(
                Some(after.clone()),
                DocumentId(1),
                fonts,
                &hidden,
                SpaceSelection::Model,
                &OverlayInputs::default(),
                None,
                Some(&receipt),
            )
            .unwrap();
        assert!(controller.ready.as_ref().unwrap().base_append.is_none());
    }
}

#[test]
fn failed_full_build_does_not_poison_points_for_subsequent_valid_append() {
    let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    host.create_point(Point3 {
        x: 5.0,
        y: 6.0,
        z: 0.0,
    })
    .unwrap();
    let before = host.drawing().unwrap();
    host.create_point(Point3 {
        x: 90.0,
        y: 60.0,
        z: 0.0,
    })
    .unwrap();
    let rejected = host.drawing().unwrap();
    let (after, receipt) = issued_edit(before.clone());
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before, None);
    let points = controller.point_markers.clone();
    let style = controller.point_display_style;
    let base = controller.ready.as_ref().unwrap().base.clone();
    controller.fail_after_point_staging = true;
    assert!(controller
        .prepare_shared(
            Some(rejected),
            DocumentId(1),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model
        )
        .is_err());
    assert_eq!(controller.point_markers, points);
    assert_eq!(controller.point_display_style, style);
    assert!(Arc::ptr_eq(&controller.ready.as_ref().unwrap().base, &base));
    prepare(&mut controller, after.clone(), Some(&receipt));
    assert!(controller.ready.as_ref().unwrap().base_append.is_some());
    let mut full = CadSceneController::default();
    prepare(&mut full, after, None);
    assert_eq!(controller.point_markers, full.point_markers);
    assert_eq!(controller.point_display_style, full.point_display_style);
    assert_eq!(
        controller.ready.as_ref().unwrap().highlight.added,
        full.ready.as_ref().unwrap().highlight.added
    );
}

#[test]
fn styled_dashed_line_uses_the_full_provider_and_compaction_path() {
    use cad_db::{DrawingDatabaseBuilder, Layer, LayerAttributes, LineType, LinetypePattern};
    use cad_domain::{DatabaseId, LayerId, LinetypeId};
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_linetype(LineType {
            id: LinetypeId(1),
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.25, -0.125]),
            complex: false,
        })
        .unwrap();
    builder
        .set_layer_attributes(
            LayerId(0),
            LayerAttributes {
                linetype: "DASHED".into(),
                opacity: 0.5,
                ..LayerAttributes::default()
            },
        )
        .unwrap();
    let before = Arc::new(builder.finish().unwrap());
    let (after, receipt) = issued_edit(before.clone());
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before, None);
    prepare(&mut controller, after.clone(), Some(&receipt));
    let ready = controller.ready.as_ref().unwrap();
    assert!(ready.base_append.is_some());
    assert!(!ready.base.added.is_empty());
    assert!(ready.base.added.iter().all(|batch| batch.alpha == 0.5));
    assert!(ready
        .base
        .added
        .iter()
        .all(|batch| !batch.linetype.elements.is_empty()));
    assert_eq!(
        ready.base.added,
        build_scene(&after, ready.base.stamp.clone()).unwrap().added
    );
}

#[test]
fn changed_images_layers_and_vendor_snapshot_identity_force_full_build() {
    let (before, after, trace) = transition();
    let receipt =
        DrawingEditReceipt::from_create_line_trace(before.clone(), after.clone(), &trace).unwrap();
    for mode in 0..3 {
        let mut controller = CadSceneController::default();
        controller.set_vendor_representations(Some(Arc::new(VendorRepresentationStore::empty(7))));
        let images = Arc::new(DecodedImageCache::new());
        let layers = LayerOverrideSet::new();
        controller
            .prepare_shared_with_overlays_and_images(
                Some(before.clone()),
                DocumentId(1),
                None,
                &layers,
                SpaceSelection::Model,
                &OverlayInputs::default(),
                Some(images.clone()),
            )
            .unwrap();
        let mut layers = layers;
        let images = if mode == 0 {
            Arc::new(DecodedImageCache::new())
        } else {
            images
        };
        if mode == 1 {
            layers.set(cad_domain::LayerId(0), false);
        }
        if mode == 2 {
            controller
                .set_vendor_representations(Some(Arc::new(VendorRepresentationStore::empty(7))));
        }
        controller
            .prepare_shared_with_overlays_and_images_and_edit(
                Some(after.clone()),
                DocumentId(1),
                None,
                &layers,
                SpaceSelection::Model,
                &OverlayInputs::default(),
                Some(images),
                Some(&receipt),
            )
            .unwrap();
        assert!(
            controller.ready.as_ref().unwrap().base_append.is_none(),
            "mode={mode}"
        );
    }
}
