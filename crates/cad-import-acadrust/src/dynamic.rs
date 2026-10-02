//! Dynamic-block visibility mapping (spec §3.2).
//!
//! acadrust parses the `AcDbBlockVisibilityParameter` side view of a dynamic
//! block: the union of governed member entities (`all_blocks`) and, per named
//! state, the members that state makes visible. The currently shown state is
//! baked into the anonymous block by marking the other states' entities
//! invisible (`EntityCommon::invisible`).
//!
//! This module turns that side view into the neutral [`DynamicBlockVisibility`]
//! descriptor. The active state is resolved honestly from the member visibility
//! flags; when the flags are ambiguous (no state matches, or several do) the
//! active state stays `None` and the result is `Partial` with a stable reason
//! code. Nothing is guessed.

use std::collections::BTreeMap;

use acadrust::objects::BlockVisibilityParameter;
use cad_db::{DynamicBlockState, DynamicBlockVisibility};
use cad_domain::{Completeness, EntityId};

/// Stable reason code recorded when the active visibility state cannot be
/// resolved from the source flags.
pub(crate) const REASON_ACTIVE_UNKNOWN: &str = "dynamic_block_active_state_unknown";

/// Stable reason code recorded when a governed/visible member handle has no
/// imported entity (a dangling reference in the parameter).
pub(crate) const REASON_MEMBER_UNRESOLVED: &str = "dynamic_block_member_unresolved";

/// The result of mapping one visibility parameter.
pub(crate) struct MappedVisibility {
    pub descriptor: DynamicBlockVisibility,
    pub completeness: Completeness,
}

/// Map a parsed [`BlockVisibilityParameter`] into the database descriptor.
///
/// `member_ids` maps a source handle value to the entity id the importer
/// assigned, for the entities of the block this parameter governs.
/// `visible_flags` maps a source handle value to the entity's actual
/// `!invisible` flag, used to resolve the active state.
pub(crate) fn map_visibility(
    param: &BlockVisibilityParameter,
    member_ids: &BTreeMap<u64, EntityId>,
    visible_flags: &BTreeMap<u64, bool>,
) -> MappedVisibility {
    let mut reasons: Vec<String> = Vec::new();

    let mut member_entities: Vec<EntityId> = param
        .all_blocks
        .iter()
        .filter_map(|h| member_ids.get(&h.value()).copied())
        .collect();
    if member_entities.len() != param.all_blocks.len() {
        reasons.push(REASON_MEMBER_UNRESOLVED.to_string());
    }
    // `all_blocks` is the union of the states in practice, but a malformed or
    // truncated parameter may leave it empty. Fall back to the union of the
    // states' members so the governed set is never understated.
    for state in &param.states {
        for handle in &state.visible_blocks {
            match member_ids.get(&handle.value()).copied() {
                Some(id) => {
                    if !member_entities.contains(&id) {
                        member_entities.push(id);
                    }
                }
                None => {
                    if !reasons.iter().any(|r| r == REASON_MEMBER_UNRESOLVED) {
                        reasons.push(REASON_MEMBER_UNRESOLVED.to_string());
                    }
                }
            }
        }
    }
    member_entities.sort();
    member_entities.dedup();

    let mut states = Vec::with_capacity(param.states.len());
    for state in &param.states {
        let mut entities: Vec<EntityId> = Vec::with_capacity(state.visible_blocks.len());
        for handle in &state.visible_blocks {
            match member_ids.get(&handle.value()).copied() {
                Some(id) => entities.push(id),
                None => {
                    if !reasons.iter().any(|r| r == REASON_MEMBER_UNRESOLVED) {
                        reasons.push(REASON_MEMBER_UNRESOLVED.to_string());
                    }
                }
            }
        }
        entities.sort();
        entities.dedup();
        states.push(DynamicBlockState {
            name: state.name.clone(),
            entities,
        });
    }

    let active_state = resolve_active_state(param, member_ids, visible_flags, &states);

    let descriptor = DynamicBlockVisibility {
        member_entities,
        states,
        active_state,
    };

    if descriptor.active_state.is_none() {
        reasons.push(REASON_ACTIVE_UNKNOWN.to_string());
    }

    let completeness = if reasons.is_empty() {
        Completeness::Complete
    } else {
        Completeness::Partial(reasons)
    };
    MappedVisibility {
        descriptor,
        completeness,
    }
}

/// Determine which state the member visibility flags currently show.
///
/// Considers every governed member handle: `all_blocks` plus each state's
/// `visible_blocks` (the union, matching the descriptor's membership). A state
/// is consistent when its membership agrees with the member's actual
/// `!invisible` flag for every governed member. Returns `Some(name)` only when
/// exactly one state is consistent; any other outcome (no governed members, no
/// states, zero or several consistent states, or a governed handle with no
/// known flag) returns `None`: an unresolved active state is never guessed.
fn resolve_active_state(
    param: &BlockVisibilityParameter,
    member_ids: &BTreeMap<u64, EntityId>,
    visible_flags: &BTreeMap<u64, bool>,
    states: &[DynamicBlockState],
) -> Option<String> {
    if states.is_empty() {
        return None;
    }
    // The set of governed source handles: all_blocks plus every state member.
    let mut governed: Vec<u64> = param.all_blocks.iter().map(|h| h.value()).collect();
    for state in &param.states {
        for handle in &state.visible_blocks {
            governed.push(handle.value());
        }
    }
    governed.sort();
    governed.dedup();
    if governed.is_empty() {
        return None;
    }

    let mut actual_visible: BTreeMap<EntityId, bool> = BTreeMap::new();
    for handle_value in governed {
        // A governed member with no imported entity or no known flag makes the
        // active state unresolvable; do not guess.
        let (Some(id), Some(visible)) = (
            member_ids.get(&handle_value),
            visible_flags.get(&handle_value),
        ) else {
            return None;
        };
        actual_visible.insert(*id, *visible);
    }

    let mut matches: Vec<&DynamicBlockState> = Vec::new();
    for state in states {
        let consistent = actual_visible.iter().all(|(id, visible)| {
            let in_state = state.entities.contains(id);
            *visible == in_state
        });
        if consistent {
            matches.push(state);
        }
    }
    match matches.as_slice() {
        [only] => Some(only.name.clone()),
        _ => None,
    }
}
