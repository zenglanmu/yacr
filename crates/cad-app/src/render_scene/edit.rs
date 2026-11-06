//! ChangeSet-authorized incremental display transition.
//!
//! The evidence for an append is the dirty set the database commit published
//! (`ChangeSet`), not the command that produced it: the command identity adds no
//! information the revision pair and the ordered changes do not already carry.
//! Callers still hand over the exact pre/post snapshots, so a receipt can never
//! be manufactured for a different pair of drawings.
use std::sync::Arc;

use cad_db::{ChangeMask, ChangeSet, DrawingDatabase, ObjectChange};
use cad_domain::{EntityId, SemanticGeometry, SpaceId};

/// One entity's accumulated dirtiness inside a [`ChangeSet`].
///
/// [`ChangeSet`] publishes **entries**, not one entry per id (an
/// [`ObjectChange::Insert`] may be followed by an
/// [`ObjectChange::Update`]`(id, ChangeMask::STYLE)` for the same object, see
/// `cad_db::change`), so the masks for one id are unioned and the id keeps the
/// position of its first entry.
///
/// [`ObjectChange::Insert`] and [`ObjectChange::Delete`] carry no mask in the
/// database's model, so they contribute `ChangeMask(0)`; the id is still dirty.
pub(crate) fn dirty_entities(changes: &ChangeSet) -> Vec<(EntityId, ChangeMask)> {
    let mut dirty: Vec<(EntityId, ChangeMask)> = Vec::new();
    for change in &changes.changes {
        let (object, mask) = match change {
            ObjectChange::Insert(object) => (*object, ChangeMask(0)),
            ObjectChange::Update(object, mask) => (*object, *mask),
            ObjectChange::Delete(object) => (*object, ChangeMask(0)),
        };
        let id = EntityId(object.0);
        match dirty.iter_mut().find(|(seen, _)| *seen == id) {
            Some((_, seen_mask)) => *seen_mask = seen_mask.union(mask),
            None => dirty.push((id, mask)),
        }
    }
    dirty
}

/// What kind of incremental display transition a commit authorizes.
///
/// This is the controller's match key: only the three named arms keep resident
/// state; every other shape goes through the full rebuild.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CommitDelta {
    /// Exactly one inserted model-space entity, admitted for a tail append.
    Append(EntityId),
    /// Exactly one entity whose only display-relevant change is a rigid
    /// transform, admitted for an **in-place** overwrite of its resident batch
    /// range (same slot, same pick key, no rebuild).
    ///
    /// The admission is narrow: one dirty id, no insert and no delete, the
    /// transform bit set, and no style/reference/metadata bit (those can change
    /// the display class, the placement or the source set). The controller still
    /// proves the batch-count invariant by rebuilding the entity and requiring
    /// exactly `range.len()` batches; a geometry change that alters the count
    /// declines to the full rebuild.
    Update(EntityId),
    /// No inserts and no updates: every dirty entity is a delete. (A `ChangeSet`
    /// with an empty `changes` list and a revision bump — layer/point-style — is
    /// NOT this: the receipt is not issued and the prepare stays a full rebuild.)
    Retire(Vec<EntityId>),
    /// Anything else that the commit can still describe precisely (currently a
    /// mixed insert/delete batch, a mixed insert/update batch, and a geometry
    /// update that is not a transform). It always falls back to the full rebuild.
    Other,
}

/// A successful incremental display transition admitted from the commit's
/// authoritative [`ChangeSet`].
///
/// Block/table writes are not fully described by `ChangeSet`, so the gate stays
/// narrow: for an append, exactly one dirty entity, inserted in model space, with
/// no source handle, and a geometry class whose per-entity rebuild is the same
/// work a full rebuild would do for it (see [`appends_like_full_rebuild`]); for a
/// retire, a delete-only batch following the committed revision pair; for an
/// update, exactly one model-space entity whose mask is a transform (see
/// [`update_candidate`]). Everything else declines, and declining always falls
/// back to the full rebuild.
#[derive(Clone)]
pub struct DrawingEditReceipt {
    pub(super) before: Arc<DrawingDatabase>,
    pub(super) after: Arc<DrawingDatabase>,
    pub(super) delta: CommitDelta,
}

impl DrawingEditReceipt {
    pub(crate) fn from_commit(
        before: Arc<DrawingDatabase>,
        after: Arc<DrawingDatabase>,
        changes: &ChangeSet,
    ) -> Option<Self> {
        if Arc::ptr_eq(&before, &after) {
            return None;
        }
        if !changes.follows(before.id(), before.revision())
            || after.id() != before.id()
            || after.revision() != changes.after
        {
            return None;
        }
        let delta = classify_commit(&before, &after, changes)?;
        Some(Self {
            before,
            after,
            delta,
        })
    }

    pub fn after(&self) -> &Arc<DrawingDatabase> {
        &self.after
    }
}

/// Classify a committed revision transition.
///
/// `None` is the historical "decline" answer for a shape the receipt cannot
/// describe (an empty layer/point-style revision bump, a non-appendable single
/// insert, a style-only update). The controller treats a missing receipt exactly
/// like [`CommitDelta::Other`]: full rebuild.
fn classify_commit(
    before: &DrawingDatabase,
    after: &DrawingDatabase,
    changes: &ChangeSet,
) -> Option<CommitDelta> {
    // Retire: every published entry is a delete, and there is at least one. A
    // mixed insert/update+delete batch is deliberately NOT a retire (the insert
    // would have to be appended and the delete hidden in the same base), and an
    // empty entry list is a non-entity signal (layer/point-style), never a
    // delete-only commit.
    if !changes.changes.is_empty()
        && changes
            .changes
            .iter()
            .all(|change| matches!(change, ObjectChange::Delete(_)))
    {
        let ids = changes
            .changes
            .iter()
            .filter_map(|change| match change {
                ObjectChange::Delete(object) => Some(EntityId(object.0)),
                _ => None,
            })
            .collect();
        return Some(CommitDelta::Retire(ids));
    }

    // Append: exactly one dirty entity, inserted in model space, with no source
    // handle, and appending its own batches is the same work a full rebuild
    // would do for it.
    if let Some(inserted) = append_candidate(before, after, changes) {
        return Some(CommitDelta::Append(inserted));
    }
    // In-place transform update: exactly one dirty model-space entity whose only
    // display-relevant change is the move.
    if let Some(updated) = update_candidate(before, after, changes) {
        return Some(CommitDelta::Update(updated));
    }
    // A geometry update that is **not** a transform is described precisely enough
    // to name it: the geometry may change the batch counts, so it is the
    // full-rebuild arm, never an in-place overwrite.
    if let Some((_, mask)) = single_dirty(changes) {
        if mask.contains(ChangeMask::GEOMETRY) {
            return Some(CommitDelta::Other);
        }
    }
    // An update mixed with an insert (for a different id, since the
    // insert-with-display-state shape above was already admitted as an append)
    // cannot be an append or an in-place overwrite: the full-rebuild arm.
    let has_insert = changes
        .changes
        .iter()
        .any(|change| matches!(change, ObjectChange::Insert(_)));
    let has_update = changes
        .changes
        .iter()
        .any(|change| matches!(change, ObjectChange::Update(..)));
    if has_insert && has_update {
        return Some(CommitDelta::Other);
    }
    // A delete mixed with anything else is still described precisely enough to
    // say "not a retire" — it is the controller's full-rebuild arm.
    changes
        .changes
        .iter()
        .any(|change| matches!(change, ObjectChange::Delete(_)))
        .then_some(CommitDelta::Other)
}

/// The single dirty entity and its unioned mask, or `None` when the commit names
/// zero or several entities.
fn single_dirty(changes: &ChangeSet) -> Option<(EntityId, ChangeMask)> {
    let dirty = dirty_entities(changes);
    match dirty.as_slice() {
        [entry] => Some(*entry),
        _ => None,
    }
}

/// The append admission checks (unchanged from P1), returning the inserted id.
fn append_candidate(
    before: &DrawingDatabase,
    after: &DrawingDatabase,
    changes: &ChangeSet,
) -> Option<EntityId> {
    let dirty = dirty_entities(changes);
    let [(inserted, _)] = dirty.as_slice() else {
        return None;
    };
    let inserted = *inserted;
    // A later `Update` for the same id is fine (an insert that restores a
    // captured display state), but the entry itself must be an insert.
    if !changes.changes.iter().any(
        |change| matches!(change, ObjectChange::Insert(object) if EntityId(object.0) == inserted),
    ) {
        return None;
    }
    if before.entity(inserted).is_some()
        || before.entity_count().checked_add(1)? != after.entity_count()
    {
        return None;
    }
    let entity = after.entity(inserted)?;
    if entity.space != SpaceId::Model
        || entity.object.source_handle.is_some()
        || !appends_like_full_rebuild(&entity.geometry)
    {
        return None;
    }
    Some(inserted)
}

/// The in-place transform admission, returning the updated entity id.
///
/// The database's only transform-producing path
/// ([`cad_db::DrawingTransaction::transform_entity`]) rewrites the stored geometry,
/// so it publishes [`ChangeMask::GEOMETRY`] **and** [`ChangeMask::TRANSFORM`]
/// (`crates/cad-db/src/drawing/transaction.rs`). A bare `TRANSFORM` mask is also
/// accepted. Any other bit — `STYLE` (layer/space/draw order), `REFERENCES`,
/// `METADATA` (type key/source handle) — changes the display class, placement or
/// source set, so it declines.
///
/// The mask cannot prove that the batch **count** is unchanged (a stretched
/// spline can subdivide differently), so that proof lives in the controller: it
/// rebuilds exactly this entity and requires the rebuilt delta to have exactly
/// the resident range's length. A mismatch is a full rebuild.
fn update_candidate(
    before: &DrawingDatabase,
    after: &DrawingDatabase,
    changes: &ChangeSet,
) -> Option<EntityId> {
    let (id, mask) = single_dirty(changes)?;
    if !mask.contains(ChangeMask::TRANSFORM) {
        return None;
    }
    let allowed = ChangeMask::GEOMETRY.0 | ChangeMask::TRANSFORM.0;
    if mask.0 & !allowed != 0 {
        return None;
    }
    // An insert or a delete anywhere in the commit means the resident stream
    // changes length; an in-place overwrite only ever replaces an existing slot.
    if changes
        .changes
        .iter()
        .any(|change| matches!(change, ObjectChange::Insert(_) | ObjectChange::Delete(_)))
    {
        return None;
    }
    if before.entity_count() != after.entity_count() {
        return None;
    }
    let before_entity = before.entity(id)?;
    let after_entity = after.entity(id)?;
    if before_entity.space != SpaceId::Model || after_entity.space != SpaceId::Model {
        return None;
    }
    Some(id)
}

/// Whether appending one entity's own batches reproduces exactly the batches a
/// full rebuild would emit for that entity.
///
/// The append path re-runs the same per-entity representation/compaction code the
/// full build runs (`assembly::build_model_entity_scene`), so every class whose
/// display is derived from the entity plus the drawing's shared tables (fonts,
/// linetypes, layers, block definitions, decoded images, vendor store — all
/// pinned by the controller's `SceneVersion` comparison) is equivalent.
///
/// [`SemanticGeometry::Point`] is **deliberately excluded by name**: a visible
/// POINT's glyph is not part of the base batches at all. It is derived from the
/// drawing-wide `$PDMODE`/`$PDSIZE` scan
/// (`overlay::visible_point_marker_positions`) that the controller recomputes only
/// on a full base rebuild; appending a POINT would publish a base whose marker
/// overlay still lacks the new marker, which a full rebuild would have drawn.
///
/// [`SemanticGeometry::Image`] and [`SemanticGeometry::Mask`] are excluded by
/// name as well: their per-entity rebuild always emits raster draw items
/// (`SceneDelta::images` / `SceneDelta::masks`), and the engine's append path
/// deliberately keeps the resident textures/masks and their earlier uploads
/// unchanged (`crates/cad-engine/src/render/runtime/session.rs`). Growing those
/// arrays on the CPU alone would be a silent partial success. The controller also
/// guards the block/compound case at runtime (`appendable_delta`), because an
/// `Insert` can expand to raster members without the entity's own geometry saying
/// so.
fn appends_like_full_rebuild(geometry: &SemanticGeometry) -> bool {
    match geometry {
        SemanticGeometry::Point(_) => false,
        SemanticGeometry::Image { .. } | SemanticGeometry::Mask { .. } => false,
        SemanticGeometry::Line { .. }
        | SemanticGeometry::Polyline { .. }
        | SemanticGeometry::Circle { .. }
        | SemanticGeometry::Arc { .. }
        | SemanticGeometry::Ellipse { .. }
        | SemanticGeometry::Spline { .. }
        | SemanticGeometry::Mesh(_)
        | SemanticGeometry::Insert { .. }
        | SemanticGeometry::Text { .. }
        | SemanticGeometry::Shape { .. }
        | SemanticGeometry::Opaque { .. }
        | SemanticGeometry::Compound(_)
        | SemanticGeometry::Dimension(_)
        | SemanticGeometry::Hatch(_)
        | SemanticGeometry::MultiLeader(_) => true,
    }
}

/// GPU append proof: the exact prepared CPU base that must already be resident.
///
/// `predecessor` is a [`super::ResidentScene`] retained by identity: its prefix
/// `Arc` **and** every appended segment must be the same allocations the GPU last
/// committed (see `ResidentScene::same_as`). Retaining it also keeps those
/// addresses alive, so a freed-and-reallocated scene cannot alias it.
#[derive(Clone)]
pub struct BaseAppend {
    pub predecessor: super::ResidentScene,
    pub keep_batches: usize,
}
