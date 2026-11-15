//! Model-space draw-order plans for `HATCHEDIT DRAWORDER`.
//!
//! Split out of `hatch_edit.rs` to stay inside the code-size gate. Every plan is
//! pure: it reads the candidate drawing and returns the target orders, which the
//! command then writes through the one entity row field (`DbEntity::draw_order`)
//! in its single transaction. `DRAWORDER BOUNDARY` is **not** here: it needs a
//! stored boundary record this build does not have, so it stays a named refusal
//! in `hatch_edit.rs`.

use super::hatch_edit::invalid;
use super::*;

/// The first free draw order above every model-space row.
pub(super) fn next_model_order(drawing: &DrawingDatabase) -> i64 {
    drawing
        .entities()
        .filter(|entity| entity.space == SpaceId::Model)
        .map(|entity| entity.draw_order)
        .max()
        .map(|order| order.saturating_add(1))
        .unwrap_or(0)
}

/// Target orders that put the selected hatches behind every other model-space
/// object, keeping their relative order.
pub(super) fn behind_all_orders(
    drawing: &DrawingDatabase,
    selected: &std::collections::BTreeSet<EntityId>,
) -> CadResult<Vec<(EntityId, i64)>> {
    let min_other = drawing
        .entities()
        .filter(|entity| entity.space == SpaceId::Model && !selected.contains(&entity.id))
        .map(|entity| entity.draw_order)
        .min();
    let Some(min_other) = min_other else {
        return Ok(Vec::new());
    };
    let mut ordered: Vec<&DbEntity> = drawing
        .entities()
        .filter(|entity| selected.contains(&entity.id))
        .collect();
    ordered.sort_by_key(|entity| (entity.draw_order, entity.id.0));
    if ordered.iter().all(|entity| entity.draw_order < min_other) {
        return Ok(Vec::new());
    }
    let count = i64::try_from(ordered.len())
        .map_err(|_| invalid("order_exhausted", "too many selected hatches to reorder"))?;
    let first = min_other.checked_sub(count).ok_or_else(|| {
        invalid(
            "order_exhausted",
            "the target draw orders would underflow i64",
        )
    })?;
    Ok(ordered
        .iter()
        .enumerate()
        .map(|(index, entity)| (entity.id, first + index as i64))
        .collect())
}

/// Target orders that put the selected hatches in front of every other
/// model-space object, keeping their relative order.
pub(super) fn front_all_orders(
    drawing: &DrawingDatabase,
    selected: &std::collections::BTreeSet<EntityId>,
) -> CadResult<Vec<(EntityId, i64)>> {
    let base = drawing
        .entities()
        .filter(|entity| entity.space == SpaceId::Model && !selected.contains(&entity.id))
        .map(|entity| entity.draw_order)
        .max()
        .unwrap_or(0);
    let mut ordered: Vec<&DbEntity> = drawing
        .entities()
        .filter(|entity| selected.contains(&entity.id))
        .collect();
    ordered.sort_by_key(|entity| (entity.draw_order, entity.id.0));
    if ordered.iter().all(|entity| entity.draw_order > base) {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(ordered.len());
    for (index, entity) in ordered.iter().enumerate() {
        let target = base.checked_add(index as i64 + 1).ok_or_else(|| {
            invalid(
                "order_exhausted",
                "the target draw orders would overflow i64",
            )
        })?;
        out.push((entity.id, target));
    }
    Ok(out)
}
