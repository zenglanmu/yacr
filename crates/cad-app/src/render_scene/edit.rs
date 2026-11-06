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

/// A successful single-entity insert transition admitted from the commit's
/// authoritative [`ChangeSet`].
///
/// Block/table writes are not fully described by `ChangeSet`, so the gate stays
/// narrow: exactly one dirty entity, inserted in model space, with no source
/// handle, and a geometry class whose per-entity rebuild is the same work a full
/// rebuild would do for it (see [`appends_like_full_rebuild`]). Everything else
/// declines, and declining always falls back to the full rebuild.
#[derive(Clone)]
pub struct DrawingEditReceipt {
    pub(super) before: Arc<DrawingDatabase>,
    pub(super) after: Arc<DrawingDatabase>,
    pub(super) inserted: EntityId,
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
        let dirty = dirty_entities(changes);
        let [(inserted, _)] = dirty.as_slice() else {
            return None;
        };
        let inserted = *inserted;
        // A later `Update` for the same id is fine (an insert that restores a
        // captured display state), but the entry itself must be an insert.
        if !changes
            .changes
            .iter()
            .any(|change| matches!(change, ObjectChange::Insert(object) if EntityId(object.0) == inserted))
        {
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
        Some(Self {
            before,
            after,
            inserted,
        })
    }

    pub fn after(&self) -> &Arc<DrawingDatabase> {
        &self.after
    }
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
