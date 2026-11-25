//! `ResidentScene` release/identity/append tests.

use super::*;
use crate::host::HostController;
use crate::layers::LayerOverrideSet;
use crate::render_scene::{CadSceneController, DrawingEditReceipt, OverlayInputs};
use cad_domain::{DocumentId, Point3};
use cad_representation::SpaceSelection;

fn empty_prefix() -> Arc<SceneDelta> {
    Arc::new(SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), 0),
        added: Vec::new(),
        images: Vec::new(),
        masks: Vec::new(),
    })
}

#[test]
fn release_keeps_counts_and_identity_and_refuses_materialize() {
    let mut prefix = (*empty_prefix()).clone();
    prefix.added = batches(2);
    let mut resident = ResidentScene::from_prefix(Arc::new(prefix));
    resident.push_appended(Arc::new(segment(1, DocumentId(2))));
    let before = resident.clone();
    assert_eq!(resident.batch_len(), 3);
    assert_eq!(resident.payload_state(), PayloadState::Resident);
    assert_eq!(resident.vertex_count(0), Some(4));

    let released = resident.release_payload();
    assert!(released > 0, "the release must report the bytes it dropped");
    assert_eq!(resident.payload_state(), PayloadState::GpuOnly);
    assert!(!resident.has_cpu_payload());
    assert!(!resident.prefix_has_cpu_payload());
    assert_eq!(resident.batch_len(), 3, "counts survive the release");
    assert_eq!(resident.vertex_count(0), Some(4), "counts survive");
    assert!(
        resident.same_as(&before),
        "a payload release must not change scene identity"
    );
    assert_eq!(
        resident.metadata().len(),
        3,
        "metadata-only batches stay readable"
    );
    assert!(resident.resident_batches().is_err());
    assert!(resident.to_delta().is_err());
    assert!(
        resident.materialized(0..3).is_err(),
        "a released scene must refuse materialisation, never return empty"
    );
    assert!(resident.materialized(2..3).is_err());
    assert!(
        resident.materialized(2..2).is_ok(),
        "an empty range is empty"
    );

    assert_eq!(resident.release_payload(), 0, "release is idempotent");
    // A segment appended **after** the release owns its own payload and is
    // the only materialisable range.
    resident.push_appended(Arc::new(segment(1, DocumentId(3))));
    assert_eq!(resident.batch_len(), 4);
    assert_eq!(
        resident
            .materialized(3..4)
            .expect("the new tail owns its payload")
            .len(),
        1
    );
    assert!(resident.materialized(2..4).is_err());
}

#[test]
fn release_is_visible_through_every_clone_and_append_only_stages_the_tail() {
    let mut prefix = (*empty_prefix()).clone();
    prefix.added = batches(2);
    let mut resident = ResidentScene::from_prefix(Arc::new(prefix));
    let clone = resident.clone();
    resident.release_payload();
    assert!(!clone.has_cpu_payload(), "the BatchSet is shared");
    assert!(clone.materialized(0..1).is_err());

    resident.push_appended(Arc::new(segment(1, DocumentId(2))));
    assert_eq!(resident.batch_len(), 3);
    assert_eq!(
        resident.materialized(2..3).expect("the new tail").len(),
        1,
        "an append onto a released prefix stages only its own segment"
    );
    assert!(resident.materialized(1..3).is_err());
}

#[test]
fn a_replaced_batch_after_release_changes_identity_but_not_payload_state() {
    let mut prefix = (*empty_prefix()).clone();
    prefix.added = batches(2);
    let mut resident = ResidentScene::from_prefix(Arc::new(prefix));
    resident.release_payload();
    let before = resident.clone();
    assert!(resident.replace_batch(1, Arc::new(batch(7))));
    assert!(
        !resident.same_as(&before),
        "a replacement advances the BatchSet generation"
    );
    assert_eq!(resident.payload_state(), PayloadState::GpuOnly);
    assert_eq!(
        resident.batch_metadata_at(1).map(|batch| batch.draw_order),
        Some(7)
    );
    assert!(resident.materialized(0..2).is_err());
}

#[test]
fn same_as_is_pointer_identity_not_content_identity() {
    let prefix = empty_prefix();
    let other_prefix = Arc::new((*prefix).clone());

    let resident = ResidentScene::from_prefix(prefix.clone());
    assert!(
        resident.same_as(&resident.clone()),
        "a clone shares the prefix and the segments"
    );
    assert!(
        !resident.same_as(&ResidentScene::from_prefix(other_prefix)),
        "a re-published prefix with identical content is a different base"
    );

    let mut appended = resident.clone();
    appended.push_appended(Arc::new((*prefix).clone()));
    assert!(
        !resident.same_as(&appended),
        "a pushed segment makes it a different resident scene"
    );
    assert!(
        !appended.same_as(&resident),
        "identity is not one-directional"
    );
    assert!(
        appended.same_as(&appended.clone()),
        "the same segment allocation is the same resident scene"
    );

    let mut other_segment = resident.clone();
    other_segment.push_appended(Arc::new((*prefix).clone()));
    assert!(
        !appended.same_as(&other_segment),
        "equal-length segment lists with distinct allocations are different"
    );

    // Replacing one batch swaps one `Arc`: every other batch is shared, but
    // the scene is a different one.
    let mut with_batches = ResidentScene::from_prefix(Arc::new(SceneDelta {
        added: batches(2),
        ..(*prefix).clone()
    }));
    let before = with_batches.clone();
    assert!(
        with_batches.replace_batch(1, Arc::new(batch(9))),
        "index 1 is in range"
    );
    assert!(
        !with_batches.same_as(&before),
        "a replaced per-batch Arc makes it a different resident scene"
    );
    assert!(
        Arc::ptr_eq(
            &with_batches.batch_metadata_at(0).expect("batch 0"),
            &before.batch_metadata_at(0).expect("batch 0")
        ),
        "replace_batch must not clone or replace the other batches"
    );
}

#[test]
fn replace_batch_is_bounded_and_never_creates_a_gap() {
    let mut prefix = (*empty_prefix()).clone();
    prefix.added = batches(2);
    let mut resident = ResidentScene::from_prefix(Arc::new(prefix));
    resident.push_appended(Arc::new(segment(1, DocumentId(2))));
    assert_eq!(resident.batch_len(), 3);

    assert!(
        !resident.replace_batch(3, Arc::new(batch(3))),
        "out of range is a refusal, never a silent append"
    );
    assert_eq!(resident.batch_len(), 3, "a refused replace changes nothing");
    assert!(resident.replace_batch(2, Arc::new(batch(7))));
    assert_eq!(
        resident.batch_len(),
        3,
        "a replace never changes the length"
    );
    assert_eq!(
        resident.batch_metadata_at(2).map(|batch| batch.draw_order),
        Some(7),
        "the replaced batch is the one at that resident index"
    );
}

#[test]
fn materialized_covers_the_range_across_segments() {
    let mut prefix = (*empty_prefix()).clone();
    prefix.added = batches(2);
    let mut resident = ResidentScene::from_prefix(Arc::new(prefix));
    resident.push_appended(Arc::new(segment(1, DocumentId(2))));
    resident.push_appended(Arc::new(segment(2, DocumentId(2))));
    assert_eq!(resident.batch_len(), 5);

    let draws = |range: Range<usize>| -> Vec<i64> {
        resident
            .materialized(range)
            .expect("the test payload is resident")
            .iter()
            .map(|batch| batch.draw_order)
            .collect()
    };
    assert_eq!(
        draws(0..5),
        vec![0, 1, 0, 0, 1],
        "prefix, then each segment"
    );
    assert_eq!(
        draws(3..5),
        vec![0, 1],
        "an append range is exactly the appended segment"
    );
    assert_eq!(
        draws(1..4),
        vec![1, 0, 0],
        "a spanning range is concatenated"
    );
    assert_eq!(draws(2..2), Vec::<i64>::new());
    assert_eq!(
        draws(4..99),
        vec![1],
        "a range past the end is clipped, never fabricated"
    );
    assert_eq!(
        resident.stamp().document,
        DocumentId(2),
        "the last segment's stamp is the newest committed stamp"
    );
}

fn batches(count: usize) -> Vec<RenderBatch> {
    (0..count).map(batch).collect()
}

fn batch(index: usize) -> RenderBatch {
    let x = index as f32 * 0.25;
    RenderBatch {
        uvs: Vec::new(),
        texture: None,
        pbr: None,
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: cad_scene::RenderTopology::Lines,
        vertices: vec![
            [x, 0.0, 0.0],
            [x + 0.1, 0.1, 0.0],
            [x + 0.1, 0.1, 0.0],
            [x, 0.2, 0.0],
        ],
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: Vec::new(),
        draw_order: index as i64,
        text_glyph: false,
    }
}

fn segment(count: usize, document: DocumentId) -> SceneDelta {
    SceneDelta {
        stamp: TaskStamp::new(document, 2),
        added: batches(count),
        images: Vec::new(),
        masks: Vec::new(),
    }
}

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn prepare(
    controller: &mut CadSceneController,
    db: Arc<cad_db::DrawingDatabase>,
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
fn a_forced_full_base_rebuilds_a_released_scene_with_fresh_payload() {
    let host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let drawing = host.drawing().unwrap();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, drawing.clone(), None);
    let before = controller.ready.as_ref().unwrap().base.clone();
    assert!(before.has_cpu_payload());
    before.release_payload();
    assert!(
        !controller.ready.as_ref().unwrap().base.has_cpu_payload(),
        "the controller's ready scene shares the released BatchSet"
    );

    controller.force_full_base_next_prepare();
    prepare(&mut controller, drawing, None);
    let after = &controller.ready.as_ref().unwrap().base;
    assert!(
        after.has_cpu_payload(),
        "the forced rebuild re-materialises"
    );
    assert!(
        !after.same_as(&before),
        "a forced rebuild is a new base identity"
    );
    assert!(after.materialized(0..after.batch_len()).is_ok());
}

#[test]
fn an_append_after_release_keeps_the_prefix_gpu_only_and_stages_only_the_tail() {
    let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let mut drawing = host.drawing().unwrap();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, drawing.clone(), None);
    let prefix_len = controller.ready.as_ref().unwrap().base.batch_len();
    controller.ready.as_ref().unwrap().base.release_payload();
    assert!(!controller
        .ready
        .as_ref()
        .unwrap()
        .base
        .prefix_has_cpu_payload());

    host.application
        .workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = drawing;
    let (trace, receipt) = host
        .execute_traced_with_edit(crate::Command {
            schema_version: 1,
            id: crate::CommandId::CreateLine,
            document: DocumentId(1),
            viewport: cad_domain::ViewportId(1),
            payload: crate::CommandPayload::Points(vec![p(10.0, 0.0), p(15.0, 5.0)]),
        })
        .unwrap();
    trace.result.unwrap();
    let receipt = receipt.expect("a single model insert is appendable");
    drawing = host.drawing().unwrap();
    prepare(&mut controller, drawing, Some(&receipt));

    let ready = controller.ready.as_ref().unwrap();
    assert!(
        ready.base_append.is_some(),
        "the released prefix is still an exact append predecessor"
    );
    assert!(
        !ready.base.prefix_has_cpu_payload(),
        "the append must not re-materialise the released prefix"
    );
    assert!(ready.base.batch_len() > prefix_len);
    assert!(
        ready.base.materialized(0..prefix_len).is_err(),
        "the released prefix is not materialisable"
    );
    assert!(
        ready
            .base
            .materialized(prefix_len..ready.base.batch_len())
            .is_ok(),
        "only the appended tail is staged"
    );
}

#[test]
fn append_pushes_a_segment_without_touching_the_prefix() {
    let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let mut drawing = host.drawing().unwrap();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, drawing.clone(), None);

    let prefix = controller
        .ready
        .as_ref()
        .unwrap()
        .base
        .prefix_delta()
        .clone();
    let prefix_batches = controller.ready.as_ref().unwrap().base.batch_len();
    let mut expected_batches = prefix_batches;
    let mut appended = 0;

    for step in 0..3 {
        host.application
            .workspace
            .documents
            .get_mut(&DocumentId(1))
            .unwrap()
            .drawing = drawing;
        let (trace, receipt) = host
            .execute_traced_with_edit(crate::Command {
                schema_version: 1,
                id: crate::CommandId::CreateLine,
                document: DocumentId(1),
                viewport: cad_domain::ViewportId(1),
                payload: crate::CommandPayload::Points(vec![
                    p(step as f64 * 10.0, 0.0),
                    p(step as f64 * 10.0 + 5.0, 5.0),
                ]),
            })
            .unwrap();
        trace.result.unwrap();
        let receipt = receipt.expect("a single model insert is appendable");
        drawing = host.drawing().unwrap();
        prepare(&mut controller, drawing.clone(), Some(&receipt));

        let ready = controller.ready.as_ref().unwrap();
        assert!(
            ready.base_append.is_some(),
            "step {step}: the append must be accepted"
        );
        let append = ready.base_append.as_ref().unwrap();
        assert_eq!(
            append.keep_batches, expected_batches,
            "step {step}: the prefix is exactly the previous total"
        );
        assert!(
            Arc::ptr_eq(ready.base.prefix_delta(), &prefix),
            "step {step}: the append must not re-wrap the prefix Arc"
        );
        appended += 1;
        assert_eq!(
            ready.base.appended_len(),
            appended,
            "step {step}: exactly one segment per accepted append"
        );
        let added = ready.base.batch_len() - expected_batches;
        assert!(added > 0, "step {step}: the inserted line draws");
        expected_batches = ready.base.batch_len();
    }

    assert_eq!(expected_batches, prefix_batches + appended);
    assert!(
        prefix.added.is_empty(),
        "the prefix delta carries no duplicate batch payload"
    );
}
