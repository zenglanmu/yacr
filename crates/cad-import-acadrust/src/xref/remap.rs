//! Block/entity-id remapping for spliced external-reference content.
//!
//! Submodule of [`crate::xref`]. Referenced databases have their own id space;
//! every id copied into the host must be remapped, and a miss must stay an
//! explicit unmapped reference rather than alias an unrelated host row.

use std::collections::HashMap;

use cad_db::{DynamicBlockState, DynamicBlockVisibility};
use cad_domain::*;

/// Sentinel used when a referenced block id has no host mapping.
///
/// Never a host id: a miss must not alias a block that exists in the host
/// drawing, so it becomes an explicit unmapped reference plus a diagnostic.
pub(crate) const UNMAPPED_BLOCK: BlockId = BlockId(u128::MAX);

/// Rewrite block references inside a copied geometry tree.
///
/// Coordinates and transforms are copied exactly: the host INSERT applies its
/// own `T(insertion)·T(-base)` transform, so baking a unit scale or the xref
/// insertion transform into the members would double-apply it.
///
/// The traversal mirrors [`crate::geometry::referenced_blocks`]: a dimension
/// picture block can contain INSERTs (and its display may nest compounds), so
/// the display wrapper is rebuilt after remapping those children. A referenced
/// block id with no mapping is recorded in `missing` and replaced by
/// [`UNMAPPED_BLOCK`], never by the referenced id (which may numerically alias
/// an unrelated host block).
pub(crate) fn remap_geometry(
    geometry: &SemanticGeometry,
    block_map: &HashMap<BlockId, BlockId>,
    missing: &mut Vec<BlockId>,
) -> SemanticGeometry {
    match geometry {
        SemanticGeometry::Insert { block, transform } => {
            let mapped = match block_map.get(block).copied() {
                Some(mapped) => mapped,
                None => {
                    missing.push(*block);
                    UNMAPPED_BLOCK
                }
            };
            SemanticGeometry::Insert {
                block: mapped,
                transform: *transform,
            }
        }
        SemanticGeometry::Compound(children) => SemanticGeometry::Compound(
            children
                .iter()
                .map(|child| remap_geometry(child, block_map, missing))
                .collect(),
        ),
        SemanticGeometry::Dimension(dimension) => {
            let display: Vec<SemanticGeometry> = dimension
                .display()
                .iter()
                .map(|child| remap_geometry(child, block_map, missing))
                .collect();
            if display == *dimension.display() {
                return SemanticGeometry::Dimension(dimension.clone());
            }
            // `from_picture` is the only constructor that accepts a display and
            // it recomputes the same measurement from the same definition, so a
            // previously valid dimension cannot fail here.
            let rebuilt = DimensionGeometry::from_picture(
                dimension.definition().clone(),
                dimension.style_name().to_string(),
                dimension.style().clone(),
                dimension.text_override().map(str::to_string),
                display,
            )
            .expect("mapped dimension display preserves a valid definition");
            SemanticGeometry::Dimension(Box::new(rebuilt))
        }
        // HATCH and MULTILEADER displays are derived by the domain from loops /
        // parts and cannot contain an INSERT; there is no constructor that
        // accepts a display. If that ever changes, this arm must be extended
        // together with `referenced_blocks`.
        SemanticGeometry::Hatch(_) | SemanticGeometry::MultiLeader(_) => geometry.clone(),
        other => other.clone(),
    }
}

/// Remap a dynamic visibility descriptor onto copied entity ids.
///
/// Entries whose entity was not merged (for example paper-space content) are
/// dropped rather than pointing at an unrelated host entity.
pub(super) fn remap_visibility(
    visibility: &DynamicBlockVisibility,
    entity_map: &HashMap<EntityId, EntityId>,
) -> DynamicBlockVisibility {
    let map_entities = |entities: &[EntityId]| -> Vec<EntityId> {
        entities
            .iter()
            .filter_map(|entity| entity_map.get(entity).copied())
            .collect()
    };
    DynamicBlockVisibility {
        member_entities: map_entities(&visibility.member_entities),
        states: visibility
            .states
            .iter()
            .map(|state| DynamicBlockState {
                name: state.name.clone(),
                entities: map_entities(&state.entities),
            })
            .collect(),
        active_state: visibility.active_state.clone(),
    }
}
