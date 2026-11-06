//! P3 delete-only (retire) incremental preparation.
//!
//! A delete-only commit keeps the resident base and hides the deleted ids with
//! `FLAG_HIDDEN` until a threshold rebuild compacts them. These tests are
//! deterministic and GPU-free: they inspect the prepared CPU base, the
//! per-entity index and the published flag set.

use super::*;
use crate::layers::LayerOverrideSet;
use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    DatabaseId, DocumentId, EntityId, InstancePath, LayerId, ObjectId, Point3, Revision,
    SemanticGeometry, SpaceId, TransactionId,
};
use cad_representation::SpaceSelection;
use std::sync::Arc;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// One layer and `count` model-space lines, one per entity id `1..=count`.
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

fn prepare_with(
    controller: &mut CadSceneController,
    db: Arc<DrawingDatabase>,
    layers: &LayerOverrideSet,
    edit: Option<&DrawingEditReceipt>,
) {
    controller
        .prepare_shared_with_overlays_and_images_and_edit(
            Some(db),
            DocumentId(1),
            None,
            layers,
            SpaceSelection::Model,
            &OverlayInputs::default(),
            None,
            edit,
        )
        .unwrap();
}

fn prepare(
    controller: &mut CadSceneController,
    db: Arc<DrawingDatabase>,
    edit: Option<&DrawingEditReceipt>,
) {
    prepare_with(controller, db, &LayerOverrideSet::new(), edit);
}

/// Delete `ids` through the sanctioned transaction path.
fn delete(
    before: &Arc<DrawingDatabase>,
    ids: &[EntityId],
) -> (Arc<DrawingDatabase>, DrawingEditReceipt) {
    let mut after = before.as_ref().clone();
    let mut transaction = after
        .begin_drawing_transaction("test: delete", TransactionId(7))
        .unwrap();
    for id in ids {
        transaction.delete_entity(*id).unwrap();
    }
    let changes = transaction.commit().unwrap();
    let after = Arc::new(after);
    let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes)
        .expect("a delete-only commit must be classified");
    assert!(
        matches!(receipt.delta, CommitDelta::Retire(_)),
        "a delete-only commit must be a retire, got {:?}",
        receipt.delta
    );
    (after, receipt)
}

fn is_hidden(ready: &PreparedScene, id: EntityId) -> bool {
    ready.entity_flags.iter().any(|(reference, bits)| {
        reference.entity == id
            && reference.instance == InstancePath::default()
            && bits & cad_scene::FLAG_HIDDEN != 0
    })
}

#[test]
fn delete_only_commit_keeps_the_resident_base() {
    let before = database_with(3);
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before.clone(), None);
    let base = controller.ready.as_ref().unwrap().base.clone();
    let batch_len = base.batch_len();
    let base_revision = controller.ready.as_ref().unwrap().base_revision;
    assert!(controller.entity_batch_range(EntityId(1)).is_some());

    let (after, receipt) = delete(&before, &[EntityId(1)]);
    prepare(&mut controller, after, Some(&receipt));

    let ready = controller.ready.as_ref().unwrap();
    assert!(
        ready.base.same_as(&base),
        "a delete-only commit must not rebuild or re-wrap the resident base"
    );
    assert_eq!(ready.base.batch_len(), batch_len, "no batch is removed");
    assert_eq!(
        ready.base_revision, base_revision,
        "the base revision does not advance"
    );
    assert!(
        is_hidden(ready, EntityId(1)),
        "the deleted entity must be published with FLAG_HIDDEN"
    );
    assert!(ready.base_append.is_none(), "a retire is not an append");
    assert_eq!(controller.retired_count(), 1);
}

#[test]
fn retired_flags_accumulate_across_prepares() {
    let before = database_with(3);
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before.clone(), None);
    let base = controller.ready.as_ref().unwrap().base.clone();
    let batch_len = base.batch_len();

    let (after, receipt) = delete(&before, &[EntityId(1)]);
    prepare(&mut controller, after.clone(), Some(&receipt));
    let (after, receipt) = delete(&after, &[EntityId(2)]);
    prepare(&mut controller, after, Some(&receipt));

    let ready = controller.ready.as_ref().unwrap();
    assert!(ready.base.same_as(&base), "both deletes keep the same base");
    assert_eq!(ready.base.batch_len(), batch_len);
    assert_eq!(controller.retired_count(), 2);
    for id in [EntityId(1), EntityId(2)] {
        assert!(is_hidden(ready, id), "{id:?} must stay hidden");
    }
}

#[test]
fn reinserting_a_retired_id_forces_a_full_rebuild() {
    let before = database_with(3);
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before.clone(), None);
    let base = controller.ready.as_ref().unwrap().base.clone();

    let (after, receipt) = delete(&before, &[EntityId(1)]);
    prepare(&mut controller, after.clone(), Some(&receipt));
    assert_eq!(controller.retired_count(), 1);

    // Undo re-inserts the same id. Give it a trailing draw order so the append
    // admission itself would succeed — the retire guard must be what declines.
    let mut restored = after.as_ref().clone();
    let mut entity = before.entity(EntityId(1)).unwrap().clone();
    entity.draw_order = after
        .model_space()
        .iter()
        .map(|entity| entity.draw_order)
        .max()
        .unwrap_or(-1)
        + 1;
    let mut transaction = restored
        .begin_drawing_transaction("test: undo delete", TransactionId(8))
        .unwrap();
    transaction.insert_entity(entity).unwrap();
    let changes = transaction.commit().unwrap();
    let restored = Arc::new(restored);
    let receipt = DrawingEditReceipt::from_commit(after.clone(), restored.clone(), &changes)
        .expect("the re-insert is otherwise append-eligible");
    assert!(
        matches!(receipt.delta, CommitDelta::Append(id) if id == EntityId(1)),
        "the re-insert must reach the append path before the retire guard"
    );

    prepare(&mut controller, restored, Some(&receipt));

    let ready = controller.ready.as_ref().unwrap();
    assert!(
        !ready.base.same_as(&base),
        "re-inserting a retired id must full-rebuild, never append"
    );
    assert_eq!(controller.retired_count(), 0, "the rebuild compacts them");
    assert!(
        controller.entity_batch_range(EntityId(1)).is_some(),
        "the re-inserted entity draws again"
    );
    assert!(
        !is_hidden(ready, EntityId(1)),
        "the rebuilt base needs no hidden flag for the re-inserted entity"
    );
}

#[test]
fn retired_compaction_threshold_rebuilds_and_clears() {
    let before = database_with(4);
    let mut controller = CadSceneController::default();
    controller.set_retired_compaction_threshold(1);
    prepare(&mut controller, before.clone(), None);
    let base = controller.ready.as_ref().unwrap().base.clone();

    let (after, receipt) = delete(&before, &[EntityId(1)]);
    prepare(&mut controller, after.clone(), Some(&receipt));
    assert!(
        controller.ready.as_ref().unwrap().base.same_as(&base),
        "one retired id is still under the lowered threshold"
    );
    assert_eq!(controller.retired_count(), 1);

    let (after, receipt) = delete(&after, &[EntityId(2)]);
    prepare(&mut controller, after, Some(&receipt));

    let ready = controller.ready.as_ref().unwrap();
    assert!(
        !ready.base.same_as(&base),
        "crossing the threshold must full-rebuild"
    );
    assert_eq!(controller.retired_count(), 0, "the rebuild clears the set");
    assert!(
        !is_hidden(ready, EntityId(1)) && !is_hidden(ready, EntityId(2)),
        "a rebuilt base contains neither deleted entity, so no hidden flag is needed"
    );
}

#[test]
fn delete_of_a_non_resident_id_is_harmless() {
    let before = database_with(2);
    let mut layers = LayerOverrideSet::new();
    layers.set(LayerId(0), false);
    let mut controller = CadSceneController::default();
    prepare_with(&mut controller, before.clone(), &layers, None);
    let base = controller.ready.as_ref().unwrap().base.clone();
    assert!(
        controller.entity_batch_range(EntityId(1)).is_none(),
        "the hidden layer produced no resident batch for the entity"
    );

    let (after, receipt) = delete(&before, &[EntityId(1)]);
    prepare_with(&mut controller, after, &layers, Some(&receipt));

    let ready = controller.ready.as_ref().unwrap();
    assert!(ready.base.same_as(&base));
    assert_eq!(controller.retired_count(), 1);
    assert!(
        is_hidden(ready, EntityId(1)),
        "the retired set still publishes the id even without a resident batch"
    );
}

#[test]
fn mixed_insert_and_delete_commit_classifies_as_other_and_rebuilds() {
    let before = database_with(3);
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before.clone(), None);
    let base = controller.ready.as_ref().unwrap().base.clone();

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
        .begin_drawing_transaction("test: insert+delete", TransactionId(9))
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
    transaction.delete_entity(EntityId(1)).unwrap();
    let changes = transaction.commit().unwrap();
    let after = Arc::new(after);
    let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes)
        .expect("a mixed insert+delete commit still has a precise classification");
    assert_eq!(
        receipt.delta,
        CommitDelta::Other,
        "a mixed insert+delete is not a retire and not an append"
    );

    prepare(&mut controller, after, Some(&receipt));

    let ready = controller.ready.as_ref().unwrap();
    assert!(
        !ready.base.same_as(&base),
        "Other always takes the full rebuild"
    );
    assert_eq!(controller.retired_count(), 0);
    assert!(controller.entity_batch_range(inserted).is_some());
    assert!(controller.entity_batch_range(EntityId(1)).is_none());
}
