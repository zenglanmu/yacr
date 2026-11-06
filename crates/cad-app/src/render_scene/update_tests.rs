//! P4 transform-only update (in-place batch overwrite).
//!
//! Deterministic and GPU-free: these tests inspect the commit classification, the
//! resident CPU base and the per-entity index. The GPU write itself is contracted
//! in `cad-render-wgpu` (`renderer_impl::overwrite_gpu_tests`) and the engine's
//! apply/fallback admission in `cad-engine` (`render::runtime::tests`).

use super::*;
use crate::layers::LayerOverrideSet;
use crate::render_scene::edit::dirty_entities;
use cad_db::{ChangeMask, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    DatabaseId, DocumentId, EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry,
    SpaceId, TransactionId, Transform3,
};
use cad_representation::SpaceSelection;
use cad_scene::RenderTopology;
use std::sync::Arc;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// One layer and `count` model-space lines (`x = index * 10`), one per entity id
/// `1..=count`.
fn database_with(count: usize) -> Arc<DrawingDatabase> {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for index in 0..count {
        let id = EntityId(index as u128 + 1);
        let x = index as f64 * 10.0;
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id.0),
                    type_key: "AcDbLine".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id,
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry: SemanticGeometry::Line {
                    start: p(x, 0.0),
                    end: p(x + 5.0, 0.0),
                },
                draw_order: index as i64,
            })
            .unwrap();
    }
    Arc::new(builder.finish().unwrap())
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

/// Move `id` by `dx` through the sanctioned transform path and return the receipt.
fn move_entity(
    before: &Arc<DrawingDatabase>,
    id: EntityId,
    dx: f64,
) -> (Arc<DrawingDatabase>, DrawingEditReceipt) {
    let mut after = before.as_ref().clone();
    let mut transaction = after
        .begin_drawing_transaction("test: move", TransactionId(3))
        .unwrap();
    transaction
        .transform_entity(id, &Transform3::translation(p(dx, 0.0)))
        .unwrap();
    let changes = transaction.commit().unwrap();
    let after = Arc::new(after);
    let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes)
        .expect("a single transform update must be classified");
    (after, receipt)
}

#[test]
fn classification_admits_one_transform_update() {
    let before = database_with(3);
    let (_, receipt) = move_entity(&before, EntityId(2), 100.0);
    assert_eq!(
        receipt.delta,
        CommitDelta::Update(EntityId(2)),
        "one TRANSFORM(+GEOMETRY) update of one entity is the in-place arm"
    );
}

#[test]
fn classification_keeps_a_geometry_update_on_the_full_rebuild() {
    let before = database_with(2);
    let mut after = before.as_ref().clone();
    let mut entity = after.entity(EntityId(1)).unwrap().clone();
    entity.geometry = SemanticGeometry::Line {
        start: p(0.0, 0.0),
        end: p(50.0, 0.0),
    };
    let mut transaction = after
        .begin_drawing_transaction("test: stretch", TransactionId(4))
        .unwrap();
    transaction.update_entity(entity).unwrap();
    let changes = transaction.commit().unwrap();
    let after = Arc::new(after);
    let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes)
        .expect("a geometry update is described precisely");
    assert_eq!(
        dirty_entities(&changes),
        vec![(EntityId(1), ChangeMask::GEOMETRY)],
        "the fixture is a geometry change without the transform bit"
    );
    assert_eq!(
        receipt.delta,
        CommitDelta::Other,
        "a GEOMETRY change can alter the batch count, so it is never in-place"
    );
}

#[test]
fn classification_keeps_a_mixed_update_and_insert_on_the_full_rebuild() {
    let before = database_with(2);
    let mut after = before.as_ref().clone();
    let inserted = after.allocate_entity_id();
    let draw_order = before
        .model_space()
        .iter()
        .map(|entity| entity.draw_order)
        .max()
        .unwrap_or(-1)
        + 1;
    let mut transaction = after
        .begin_drawing_transaction("test: move+insert", TransactionId(5))
        .unwrap();
    transaction
        .transform_entity(EntityId(1), &Transform3::translation(p(0.0, 10.0)))
        .unwrap();
    transaction
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(inserted.0),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: inserted,
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: p(100.0, 0.0),
                end: p(105.0, 0.0),
            },
            draw_order,
        })
        .unwrap();
    let changes = transaction.commit().unwrap();
    let after = Arc::new(after);
    let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes)
        .expect("a mixed update+insert is described precisely");
    assert_eq!(
        receipt.delta,
        CommitDelta::Other,
        "a mixed update+insert is neither a pure update nor an append"
    );
}

#[test]
fn transform_update_keeps_the_resident_base_and_moves_the_batch_in_place() {
    let before = database_with(3);
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before.clone(), None);

    let (base, base_revision, batch_len, range) = {
        let ready = controller.ready.as_ref().unwrap();
        let range = controller
            .entity_batch_range(EntityId(2))
            .expect("entity 2 owns a resident batch");
        (
            ready.base.clone(),
            ready.base_revision,
            ready.base.batch_len(),
            range,
        )
    };
    assert_eq!(range, 1..2, "one line batch per entity");
    let moved_before = controller
        .ready
        .as_ref()
        .unwrap()
        .base
        .materialized(range.clone());
    assert_eq!(moved_before.len(), 1);
    // The world start of the line before the move.
    let world_x = |batch: &RenderBatch| batch.local_origin.x + f64::from(batch.vertices[0][0]);
    assert_eq!(world_x(&moved_before[0]), 10.0);

    let (after, receipt) = move_entity(&before, EntityId(2), 100.0);
    prepare(&mut controller, after.clone(), Some(&receipt));

    let ready = controller.ready.as_ref().unwrap();
    assert_eq!(
        controller.entity_batch_range(EntityId(2)),
        Some(range.clone()),
        "the moved entity keeps the same resident batch range"
    );
    assert_eq!(
        ready.base.batch_len(),
        batch_len,
        "an in-place update never changes the resident batch count"
    );
    assert_eq!(
        ready.base_revision, base_revision,
        "the base publication revision does not advance: the GPU keeps its base"
    );
    assert!(
        Arc::ptr_eq(ready.base.prefix_delta(), base.prefix_delta()),
        "the prefix Arc is retained (no rebuild)"
    );
    assert!(
        !ready.base.same_as(&base),
        "the replaced per-batch Arc is a new resident identity"
    );
    assert_eq!(
        ready
            .overwrites
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>(),
        vec![range.start],
        "the move is published as one in-place overwrite at the resident slot"
    );
    let moved_after = ready.base.materialized(range.clone());
    assert_eq!(moved_after.len(), 1);
    assert_eq!(
        world_x(&moved_after[0]),
        110.0,
        "the resident CPU base already holds the moved geometry"
    );
    assert_eq!(moved_after[0].draw_order, moved_before[0].draw_order);
    assert_eq!(moved_after[0].sources, moved_before[0].sources);
    assert_eq!(moved_after[0].topology, moved_before[0].topology);
    // Every other entity's batches are untouched.
    for (index, batch) in ready.base.batches().enumerate() {
        if index == range.start {
            continue;
        }
        let before_batch = base.batches().nth(index).expect("same batch count");
        assert_eq!(batch, before_batch, "batch {index} must be unchanged");
    }

    // Bounds are lazily stale, then exact on demand: the controller never
    // recomputes them (the database memo is revision-tagged and the update left it
    // invalid), and this first consumer sees the moved extent exactly.
    let (min, max) = after.bounds().expect("model-space bounds");
    assert_eq!(min.x, 0.0);
    assert_eq!(
        max.x, 115.0,
        "the moved line reaches 110..115, beyond entity 3's 20..25"
    );
    assert_eq!(min.y, 0.0);
    assert_eq!(max.y, 0.0);
}

#[test]
fn an_update_that_does_not_fit_falls_back_to_a_full_rebuild() {
    let before = database_with(2);
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before.clone(), None);
    let base = controller.ready.as_ref().unwrap().base.clone();
    let base_revision = controller.ready.as_ref().unwrap().base_revision;
    let range = controller.entity_batch_range(EntityId(1)).unwrap();

    let (after, receipt) = move_entity(&before, EntityId(1), 50.0);
    // The rebuilt delta cannot prove the resident batch range (for example a
    // stretch that subdivides a different number of batches).
    controller.set_fail_next_update_fit(true);
    prepare(&mut controller, after.clone(), Some(&receipt));

    let ready = controller.ready.as_ref().unwrap();
    assert!(
        !ready.base.same_as(&base),
        "a declined in-place update must full-rebuild (new base identity)"
    );
    assert!(
        !Arc::ptr_eq(ready.base.prefix_delta(), base.prefix_delta()),
        "the declined update must not reuse the resident prefix Arc"
    );
    assert_eq!(
        ready.base_revision,
        base_revision + 1,
        "the full rebuild advances the base revision"
    );
    assert!(
        ready.overwrites.is_empty(),
        "a full rebuild publishes no in-place overwrite"
    );
    assert_eq!(
        controller.entity_batch_range(EntityId(1)),
        Some(range.clone()),
        "the rebuilt index records the entity again"
    );
    let batch = ready.base.materialized(range).remove(0);
    assert_eq!(
        batch.local_origin.x + f64::from(batch.vertices[0][0]),
        50.0,
        "the rebuilt base contains the moved geometry"
    );
}

/// One layer and one circle entity (`EntityId(1)`) at the origin, radius 5.
fn database_with_circle() -> Arc<DrawingDatabase> {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbCircle".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Circle {
                center: p(0.0, 0.0),
                normal: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                radius: 5.0,
            },
            draw_order: 0,
        })
        .unwrap();
    Arc::new(builder.finish().unwrap())
}

/// A conic batch is not a special case for the controller: a transform-only move
/// of a circle is published as the same single in-place overwrite, so the
/// renderer's conic fit gate (not a full rebuild) decides the GPU write.
#[test]
fn a_circle_transform_update_keeps_the_resident_conic_base() {
    let before = database_with_circle();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before.clone(), None);

    let (base, base_revision, batch_len, range) = {
        let ready = controller.ready.as_ref().unwrap();
        (
            ready.base.clone(),
            ready.base_revision,
            ready.base.batch_len(),
            controller
                .entity_batch_range(EntityId(1))
                .expect("the circle owns a resident batch"),
        )
    };
    let before_batch = controller
        .ready
        .as_ref()
        .unwrap()
        .base
        .materialized(range.clone())
        .remove(0);
    assert_eq!(
        before_batch.topology,
        RenderTopology::Conics,
        "a circle is one SDF conic batch"
    );
    assert_eq!(before_batch.conics.len(), 1);
    let start_x = before_batch.conics[0].center.x;
    let radius = before_batch.conics[0].major.x;

    let (after, receipt) = move_entity(&before, EntityId(1), 100.0);
    assert_eq!(
        receipt.delta,
        CommitDelta::Update(EntityId(1)),
        "a circle translation is the in-place arm"
    );
    prepare(&mut controller, after.clone(), Some(&receipt));

    let ready = controller.ready.as_ref().unwrap();
    assert_eq!(
        ready.base_revision, base_revision,
        "the conic move does not advance the base revision"
    );
    assert!(
        Arc::ptr_eq(ready.base.prefix_delta(), base.prefix_delta()),
        "the prefix Arc is retained (no rebuild)"
    );
    assert_eq!(
        ready.base.batch_len(),
        batch_len,
        "an in-place conic update never changes the batch count"
    );
    assert_eq!(
        controller.entity_batch_range(EntityId(1)),
        Some(range.clone()),
        "the moved circle keeps the same resident batch range"
    );
    assert_eq!(
        ready
            .overwrites
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>(),
        vec![range.start],
        "the circle move is published as one in-place overwrite"
    );
    let moved = ready.base.materialized(range).remove(0);
    assert_eq!(moved.topology, before_batch.topology);
    assert_eq!(moved.conics.len(), 1, "the conic count is unchanged");
    assert_eq!(
        moved.conics[0].center.x,
        start_x + 100.0,
        "the resident CPU base already holds the moved centre"
    );
    assert_eq!(
        moved.conics[0].major.x, radius,
        "a translation does not change the radius"
    );
}
