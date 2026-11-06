//! P4 single-entity display update: replace one entity's resident batches in place.
//!
//! A commit that only changes one entity's display state (a move, a radius or a
//! colour) should not rebuild the scene. The controller therefore rebuilds **just
//! that entity**, proves the rebuilt delta occupies exactly the entity's resident
//! batch range, and publishes one `(resident index, Arc<RenderBatch>)` pair per
//! slot on [`PreparedScene::overwrites`]. The engine applies them to the renderer
//! (`Renderer::overwrite_batch`), which rewrites the existing GPU buffers without
//! moving the batch slot, the pick key, the batch order or the draw order — so a
//! selection survives a move and the resident base is not re-staged.
//!
//! The controller does not own the renderer, so it never writes to the GPU
//! itself. If the renderer refuses any overwrite, the engine falls back to a
//! **full base re-stage** of the published base; the published CPU base already
//! holds the new display state, so the fallback always draws the new scene.
//!
//! # Bounds
//!
//! A display edit may change the drawing's model-space extent, and the update
//! path deliberately does **not** recompute it: `DrawingDatabase::bounds()` is a
//! revision-tagged lazy memo, so the commit leaves it stale and the next
//! consumer recomputes it exactly on demand (that consumer may be the overlay's
//! axes/grid built later in the same preparation, or the host). Recomputing it
//! here would be an O(scene) scan on the update path, and substituting an
//! approximate extent is never acceptable.
use std::ops::Range;
use std::sync::Arc;

use cad_db::DrawingDatabase;
use cad_domain::{EntityId, TaskStamp};
use cad_representation::{FontEngine, SpaceSelection};
use cad_resources::DecodedImageCache;
use cad_scene::RenderBatch;

use super::entity_index::EntityBatchIndex;
use super::retire::retire_keeps_base;
use super::{
    appendable_delta, assembly, CommitDelta, DrawingEditReceipt, ResidentScene, SceneVersion,
};
use crate::layers::LayerOverrideSet;
use crate::vendor::VendorRepresentationStore;
use std::collections::BTreeSet;

/// The resident base can be kept for this receipt's in-place update, and this is
/// the entity plus the resident batch range it owns.
///
/// This is only the *structural* half of the admission: the receipt was already
/// classified [`CommitDelta::Update`] and every scene-wide input is pinned. The
/// footprint proof (the rebuilt delta has exactly `range.len()` batches) lives in
/// [`stage_update`].
///
/// Every precondition is the same kind the retire path uses
/// ([`retire_keeps_base`]): the document, space, fonts, layer overrides, image
/// cache and vendor snapshot must be identical, so the resident base still
/// describes the drawing. On top of that the entity must be a model-space entity
/// with a resident batch range that is entirely inside the base, and it must not
/// be a retired id (its batches are being suppressed, so replacing them in place
/// would fight the hidden flag; the compaction rebuild is the honest answer).
// The inputs are exactly the controller fields the narrow decision reads; bundling
// them into a struct would only move the same list one call away.
#[allow(clippy::too_many_arguments)]
pub(super) fn update_candidate(
    edit: Option<&DrawingEditReceipt>,
    doc: Option<&DrawingDatabase>,
    space: SpaceSelection,
    base: Option<&ResidentScene>,
    entity_batches: &EntityBatchIndex,
    retired: &BTreeSet<EntityId>,
    previous: Option<&SceneVersion>,
    current: Option<&SceneVersion>,
) -> Option<(EntityId, Range<usize>)> {
    let edit = edit?;
    let CommitDelta::Update(id) = &edit.delta else {
        return None;
    };
    let doc = doc?;
    if !std::ptr::eq(doc, edit.after.as_ref()) {
        return None;
    }
    if space != SpaceSelection::Model || retired.contains(id) {
        return None;
    }
    let resident = base?;
    let range = entity_batches.get(*id)?;
    if range.end > resident.batch_len() {
        return None;
    }
    if !retire_keeps_base(previous?, current?) {
        return None;
    }
    edit.after.entity(*id)?;
    Some((*id, range))
}

/// Rebuild one entity and return its reconciled batches, or `None` when the
/// in-place path cannot be proven and the caller must full-rebuild.
///
/// This is the **footprint gate**, and it is the only admission proof that the
/// change fits the resident slot:
///
/// - the rebuilt delta must have exactly `range.len()` render batches (a radius
///   or colour change keeps that count, a linetype change that subdivides into a
///   different number of dash segments does not);
/// - it must not carry raster draw items (an image or wipeout mask would have to
///   grow the resident texture/mask sets, which the overwrite path never
///   re-uploads).
///
/// A build error also declines: the fallback is the full rebuild, which runs the
/// same provider path and surfaces the error honestly instead of hiding it here.
/// A decline is never a partial write — the caller publishes no overwrite and
/// rebuilds the whole base.
// Mirrors `assembly::build_model_entity_scene`'s arguments (the full-build inputs).
#[allow(clippy::too_many_arguments)]
pub(super) fn stage_update(
    doc: &DrawingDatabase,
    id: EntityId,
    range: &Range<usize>,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    images: Option<Arc<DecodedImageCache>>,
    layers: &LayerOverrideSet,
    vendor: Option<&VendorRepresentationStore>,
) -> Option<Vec<Arc<RenderBatch>>> {
    let entity = doc.entity(id)?;
    let delta =
        assembly::build_model_entity_scene(doc, entity, stamp, fonts, images, layers, vendor)
            .ok()?;
    if delta.added.len() != range.len() || !appendable_delta(&delta) {
        return None;
    }
    Some(delta.added.into_iter().map(Arc::new).collect())
}
