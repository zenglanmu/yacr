//! Which resident base batches belong to which entity.
//!
//! The scene controller knows "entity X changed" from the database commit, but
//! the GPU ranges live in the compacted base stream. This side index is that
//! translation: for every visible **model-space** entity that emitted at least
//! one render batch, the range of `base.added` its own compacted delta occupies.
//!
//! Invariants (enforced by [`super::CadSceneController`]):
//!
//! * a full base build (initial, version change, space switch, append decline)
//!   **replaces** the whole index with the one the build reported — stale ids
//!   never survive;
//! * an accepted append extends the resident index with the inserted entity's
//!   contiguous suffix range;
//! * paper space owns no per-entity ranges, so the index is empty there;
//! * an entity that produces no batches (hidden layer, non-drawable geometry, a
//!   POINT) has no entry at all — never an empty/zero range.
//!
//! It is bookkeeping only: it is not part of [`cad_scene::SceneDelta`], is not
//! published on the wire, and never changes what is drawn.
use std::collections::BTreeMap;
use std::ops::Range;

use cad_domain::EntityId;

#[derive(Clone, Debug, Default)]
pub(super) struct EntityBatchIndex {
    ranges: BTreeMap<EntityId, Range<usize>>,
}

impl EntityBatchIndex {
    pub(super) fn from_ranges(ranges: BTreeMap<EntityId, Range<usize>>) -> Self {
        Self { ranges }
    }

    /// Drop every entry. Used when the base is rebuilt (a rebuild starts from
    /// the freshly reported map) or when the base is not model space.
    pub(super) fn clear(&mut self) {
        self.ranges.clear();
    }

    /// Record the resident range `range` as owned by `id`.
    ///
    /// An empty range is not recorded: an entity that produced no batches has no
    /// resident range, and an empty entry would be indistinguishable from a
    /// stale one.
    pub(super) fn record(&mut self, id: EntityId, range: Range<usize>) {
        if range.is_empty() {
            self.ranges.remove(&id);
            return;
        }
        self.ranges.insert(id, range);
    }

    /// The stored range for `id`, or `None` when the entity owns no resident
    /// batch in the current model-space base.
    pub(super) fn get(&self, id: EntityId) -> Option<Range<usize>> {
        self.ranges.get(&id).cloned()
    }

    /// Number of entities with a resident range.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn len(&self) -> usize {
        self.ranges.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::LayerOverrideSet;
    use crate::render_scene::{CadSceneController, DrawingEditReceipt, OverlayInputs};
    use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};
    use cad_domain::{
        DatabaseId, DocumentId, LayerId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId,
        TransactionId,
    };
    use cad_representation::SpaceSelection;
    use std::sync::Arc;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    /// Two layers and the given `(entity id, layer, x)` model-space lines.
    fn database_with(lines: &[(u64, LayerId, f64)]) -> DrawingDatabase {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(9));
        for id in [LayerId(0), LayerId(1)] {
            builder
                .insert_layer(Layer {
                    id,
                    name: format!("L{}", id.0),
                    visible: true,
                })
                .unwrap();
        }
        for (order, (id, layer, x)) in lines.iter().enumerate() {
            builder
                .insert_entity(DbEntity {
                    object: DbObject {
                        id: ObjectId(*id as u128),
                        type_key: "AcDbLine".into(),
                        revision: Revision(0),
                        source_handle: None,
                    },
                    id: EntityId(*id as u128),
                    layer: *layer,
                    space: SpaceId::Model,
                    geometry: SemanticGeometry::Line {
                        start: p(*x, 0.0),
                        end: p(*x + 5.0, 0.0),
                    },
                    draw_order: order as i64,
                })
                .unwrap();
        }
        builder.finish().unwrap()
    }

    fn prepare(
        controller: &mut CadSceneController,
        db: &DrawingDatabase,
        layers: &LayerOverrideSet,
    ) {
        controller
            .prepare(
                Some(db),
                DocumentId(73),
                None,
                layers,
                SpaceSelection::Model,
            )
            .unwrap();
    }

    /// Shared-snapshot preparation: the only path that can hand the controller
    /// an append receipt.
    fn prepare_shared(
        controller: &mut CadSceneController,
        db: Arc<DrawingDatabase>,
        edit: Option<&DrawingEditReceipt>,
    ) {
        controller
            .prepare_shared_with_overlays_and_images_and_edit(
                Some(db),
                DocumentId(73),
                None,
                &LayerOverrideSet::new(),
                SpaceSelection::Model,
                &OverlayInputs::default(),
                None,
                edit,
            )
            .unwrap();
    }

    /// Insert one model-space line through the sanctioned transaction path.
    fn append_line(
        before: &Arc<DrawingDatabase>,
        x: f64,
    ) -> (Arc<DrawingDatabase>, DrawingEditReceipt, EntityId) {
        let mut after = before.as_ref().clone();
        let id = after.allocate_entity_id();
        let draw_order = before
            .model_space()
            .iter()
            .map(|entity| entity.draw_order)
            .max()
            .unwrap_or(-1)
            + 1;
        let mut transaction = after
            .begin_drawing_transaction("test: append line", TransactionId(1))
            .unwrap();
        transaction
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
                draw_order,
            })
            .unwrap();
        let changes = transaction.commit().unwrap();
        let after = Arc::new(after);
        let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes)
            .expect("a single model insert is appendable");
        (after, receipt, id)
    }

    #[test]
    fn full_build_records_each_entity_batch_range() {
        let db = database_with(&[
            (1, LayerId(0), 0.0),
            (2, LayerId(0), 10.0),
            (3, LayerId(0), 20.0),
        ]);
        let mut controller = CadSceneController::default();
        prepare(&mut controller, &db, &LayerOverrideSet::new());
        assert_eq!(controller.entity_batch_index_len(), 3);

        let base = &controller.ready.as_ref().unwrap().base;
        let added: Vec<_> = base.batches().collect();
        let mut ranges = Vec::new();
        for id in [EntityId(1), EntityId(2), EntityId(3)] {
            let range = controller
                .entity_batch_range(id)
                .unwrap_or_else(|| panic!("{id:?} emitted batches and must be indexed"));
            assert!(!range.is_empty());
            assert!(range.end <= added.len());
            for batch in &added[range.clone()] {
                assert!(
                    batch.sources.iter().any(|source| source.entity == id),
                    "{id:?} range must only cover its own batches"
                );
            }
            ranges.push(range);
        }
        // The per-entity ranges tile the emitted stream in entity order: every
        // base batch belongs to exactly one tracked entity.
        ranges.sort_by_key(|range| range.start);
        let mut cursor = 0;
        for range in ranges {
            assert_eq!(range.start, cursor);
            cursor = range.end;
        }
        assert_eq!(cursor, added.len());
    }

    #[test]
    fn accepted_append_records_the_inserted_suffix() {
        let before = Arc::new(database_with(&[
            (1, LayerId(0), 0.0),
            (2, LayerId(0), 10.0),
        ]));
        let mut controller = CadSceneController::default();
        prepare_shared(&mut controller, before.clone(), None);
        let first = controller.entity_batch_range(EntityId(1)).unwrap();
        let second = controller.entity_batch_range(EntityId(2)).unwrap();
        let old_added_len = controller.ready.as_ref().unwrap().base.batch_len();

        let (after, receipt, inserted) = append_line(&before, 20.0);
        prepare_shared(&mut controller, after, Some(&receipt));

        let ready = controller.ready.as_ref().unwrap();
        assert!(ready.base_append.is_some(), "the append must be accepted");
        assert_eq!(controller.entity_batch_index_len(), 3);
        assert_eq!(controller.entity_batch_range(EntityId(1)), Some(first));
        assert_eq!(controller.entity_batch_range(EntityId(2)), Some(second));
        let range = controller
            .entity_batch_range(inserted)
            .expect("the inserted entity owns the appended suffix");
        assert!(range.end > range.start, "the inserted line draws");
        assert_eq!(range, old_added_len..ready.base.batch_len());
        for batch in ready.base.slices(range).flatten() {
            assert!(batch.sources.iter().any(|source| source.entity == inserted));
        }
    }

    #[test]
    fn full_rebuild_replaces_the_index_instead_of_patching_it() {
        let before = Arc::new(database_with(&[
            (1, LayerId(0), 0.0),
            (2, LayerId(0), 10.0),
        ]));
        let mut controller = CadSceneController::default();
        prepare_shared(&mut controller, before.clone(), None);
        let (appended, receipt, inserted) = append_line(&before, 20.0);
        prepare_shared(&mut controller, appended.clone(), Some(&receipt));
        assert!(controller.entity_batch_range(inserted).is_some());

        // Delete entity 1, then force the full-rebuild path (no receipt): the
        // stale range for the deleted entity must not survive.
        let mut rebuilt = appended.as_ref().clone();
        let mut transaction = rebuilt
            .begin_drawing_transaction("test: delete line", TransactionId(2))
            .unwrap();
        transaction.delete_entity(EntityId(1)).unwrap();
        transaction.commit().unwrap();
        let rebuilt = Arc::new(rebuilt);
        prepare_shared(&mut controller, rebuilt.clone(), None);

        assert!(
            controller.entity_batch_range(EntityId(1)).is_none(),
            "a deleted entity must not keep a resident range"
        );
        assert_eq!(controller.entity_batch_index_len(), 2);

        // The rebuilt index equals the one a controller that only ever saw the
        // rebuilt drawing maintains.
        let mut fresh = CadSceneController::default();
        prepare_shared(&mut fresh, rebuilt, None);
        assert_eq!(
            controller.entity_batch_index_len(),
            fresh.entity_batch_index_len()
        );
        for id in [EntityId(1), EntityId(2), inserted] {
            assert_eq!(
                controller.entity_batch_range(id),
                fresh.entity_batch_range(id),
                "{id:?} range must match a from-scratch build"
            );
        }
    }

    #[test]
    fn hidden_layer_entity_has_no_batch_range() {
        let db = database_with(&[(1, LayerId(0), 0.0), (2, LayerId(1), 10.0)]);
        let mut layers = LayerOverrideSet::new();
        layers.set(LayerId(1), false);
        let mut controller = CadSceneController::default();
        prepare(&mut controller, &db, &layers);

        assert_eq!(controller.entity_batch_index_len(), 1);
        assert!(controller.entity_batch_range(EntityId(2)).is_none());
        let range = controller.entity_batch_range(EntityId(1)).unwrap();
        for batch in controller
            .ready
            .as_ref()
            .unwrap()
            .base
            .slices(range)
            .flatten()
        {
            assert!(batch
                .sources
                .iter()
                .any(|source| source.entity == EntityId(1)));
        }
    }
}
