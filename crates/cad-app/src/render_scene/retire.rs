//! P3 retire bookkeeping: keep the resident base for a delete-only commit.
//!
//! A delete necessarily changes the scene identity (revision and entity count),
//! so the ordinary version comparison always asks for a full rebuild.
//! This module holds the narrow comparison that decides whether the resident
//! base still describes the surviving drawing, the class filter for entities a
//! whole-entity flag can actually suppress, and the retired-set flag plumbing.
//!
//! The deleted batches stay physically resident and are suppressed by
//! `FLAG_HIDDEN`; [`RETIRED_COMPACTION_THRESHOLD`] bounds that accumulation. The
//! accepted "epoch reset" is the existing full rebuild, which renumbers every
//! pick key.

use super::*;
use cad_domain::{EntityId, InstancePath, SemanticGeometry};

/// Number of retired ids the resident base may accumulate before a full rebuild
/// compacts them.
///
/// A retire keeps the resident batches physically present and suppresses them
/// through `FLAG_HIDDEN`, so every delete grows the base and its pick-key space
/// until an epoch reset. Once more than this many ids are retired, the next
/// prepare does the existing full rebuild (which renumbers every pick key) and
/// clears the retired set. The threshold is deliberately coarse: a full rebuild
/// of a large drawing is expensive, so amortising it over thousands of deletes
/// is the accepted trade-off for this phase.
pub(crate) const RETIRED_COMPACTION_THRESHOLD: usize = 4096;

/// Whether the resident base can stay for a delete-only commit.
///
/// A delete necessarily changes the scene identity's `revision` and `entities`
/// (and, for a fresh snapshot `Arc`, `document_epoch`), so those are
/// deliberately ignored. Everything that decides how a surviving entity's
/// batches were built — the drawing's construction nonce and database id, font
/// engine, layer overrides, space, image cache, vendor snapshot — must be
/// identical, otherwise the resident base no longer describes the drawing.
pub(super) fn retire_keeps_base(previous: &SceneVersion, current: &SceneVersion) -> bool {
    previous.identity.construction_nonce == current.identity.construction_nonce
        && previous.identity.database == current.identity.database
        && previous.identity.layers == current.identity.layers
        && previous.document == current.document
        && previous.fonts == current.fonts
        && previous.font_identity == current.font_identity
        && previous.layers == current.layers
        && previous.space == current.space
        && previous.images == current.images
        && previous.vendor == current.vendor
        && previous.vendor_identity == current.vendor_identity
}

/// Whether hiding this entity's resident batches by its whole-entity ref is
/// enough to retire it.
///
/// `FLAG_HIDDEN` is resolved through the pick table by
/// `(document, entity, default instance)`, so three classes cannot be suppressed
/// this way and must keep today's full rebuild:
///
/// * a POINT has no base batch at all — its glyph is the drawing-wide marker
///   overlay, which only a base rebuild rescans;
/// * an INSERT expands into batches sourced to its *children* with a non-default
///   instance path, so the root's whole-entity ref resolves to no key;
/// * a WIPEOUT mask is not part of the pick table, so it cannot carry a flag.
pub(super) fn retire_is_hideable(geometry: &SemanticGeometry) -> bool {
    match geometry {
        SemanticGeometry::Point(_)
        | SemanticGeometry::Insert { .. }
        | SemanticGeometry::Mask { .. } => false,
        SemanticGeometry::Compound(children) => children.iter().all(retire_is_hideable),
        _ => true,
    }
}

/// The flags fingerprint: the overlay inputs plus the retired id set.
///
/// The retired set is not an `OverlayInputs` field (it is controller state
/// derived from commits), but a delete must change this fingerprint or the
/// controller would skip the flags re-resolution and the engine's flags-delta
/// gate would see no change.
pub(super) fn flags_fingerprint(overlays: &OverlayInputs, retired: &BTreeSet<EntityId>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    entity_flags_fingerprint(overlays).hash(&mut hasher);
    retired.len().hash(&mut hasher);
    for id in retired {
        id.0.hash(&mut hasher);
    }
    hasher.finish()
}

/// Merge a retired id's whole-entity ref into the resolved flag set.
///
/// A retired id no longer exists in the database, so it must bypass
/// `selection_entity_flags`'s resolver (which would report it as unresolved).
/// Bits are OR-ed by `(document, entity, instance)`, exactly like the resolver.
pub(super) fn merge_retired_flags(
    flags: &mut Vec<(SelectionRef, u32)>,
    document: DocumentId,
    retired: &BTreeSet<EntityId>,
) {
    for entity in retired {
        let reference = SelectionRef {
            document,
            entity: *entity,
            instance: InstancePath::default(),
            sub_element: None,
        };
        match flags.iter_mut().find(|(existing, _)| {
            existing.document == reference.document
                && existing.entity == reference.entity
                && existing.instance == reference.instance
        }) {
            Some(entry) => entry.1 |= cad_scene::FLAG_HIDDEN,
            None => flags.push((reference, cad_scene::FLAG_HIDDEN)),
        }
    }
}
