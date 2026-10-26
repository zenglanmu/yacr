//! The `DIMENSION` handle family: one [`super::GripKind::DefinitionPoint`] grip
//! per DXF definition point, plus one for the text middle point.
//!
//! Split out of `grips/handles.rs` and `grips/drag.rs` so those files stay well
//! under the 1000-line cap. Both halves of the family live here, next to each
//! other, because they **must** agree about one thing: the documented per-subtype
//! order. [`slots`] is that single order — the handle list is it verbatim and the
//! drag rule indexes into the same vector — so the two halves cannot drift.
//!
//! Everything here is a pure function of one stored [`DimensionGeometry`]: no
//! drawing, no session, no transaction, no marker.

use cad_domain::{
    CadError, CadResult, DimensionDefinition, DimensionGeometry, DimensionKind, DimensionPoints,
    EntityId, Point3, SemanticGeometry,
};

use super::reasons::{finite_target, stale_handle, zero_displacement};
use super::{Grip, GripId, GripKind, REASON_DIMENSION_DEGENERATE, REASON_DIMENSION_INCONSISTENT};

/// One draggable definition-point slot, in the documented order.
///
/// `name` is the DXF group label the slot carries (for refusal text only — it is
/// never parsed), and `position` is the **stored** point, bit for bit.
#[derive(Debug, Clone, Copy)]
pub(super) struct Slot {
    pub(super) name: &'static str,
    pub(super) position: Point3,
}

/// The draggable slots of `definition`, in the documented `index` order.
///
/// The order is per subtype (`index` is the position in this list):
///
/// * `Linear` (`Linear { rotation }` and `Aligned` alike): `0` `first` (13),
///   `1` `second` (14), `2` `dimension_line` (10), `3` `text_middle` (11);
/// * `Radial` (`Radius`/`Diameter`): `0` `center` (10), `1` `point_on_arc` (15),
///   `2` `text_middle` (11) — `leader_length` (40) is a **scalar**, not a point,
///   so it is not a grip;
/// * `Angular2Ln`: `0` `line1_start` (13), `1` `line1_end` (14),
///   `2` `line2_start` (15), `3` `line2_end` (10), `4` `arc_point` (16),
///   `5` `text_middle` (11);
/// * `Angular3Pt`: `0` `vertex` (15), `1` `first` (13), `2` `second` (14),
///   `3` `arc_point` (10), `4` `text_middle` (11);
/// * `Ordinate`: `0` `feature` (13), `1` `leader_end` (14), `2` `base` (10),
///   `3` `text_middle` (11).
///
/// `text_middle` is always last, so its index is the subtype's definition-point
/// count and a caller can never confuse it with a point slot.
///
/// `None` only when the stored subtype and point slots **disagree**. Both public
/// constructors of [`DimensionGeometry`] run the synthesis, which refuses such a
/// definition, so this case cannot be reached through a stored entity; it is
/// handled instead of unwrapped because inventing a handle order for a
/// self-contradictory definition is exactly what this model must not do.
pub(super) fn slots(definition: &DimensionDefinition) -> Option<Vec<Slot>> {
    let slot = |name, position| Slot { name, position };
    let mut slots = match (definition.kind, &definition.points) {
        (
            DimensionKind::Linear { .. } | DimensionKind::Aligned,
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            },
        ) => vec![
            slot("first (group 13)", *first),
            slot("second (group 14)", *second),
            slot("dimension line (group 10)", *dimension_line),
        ],
        (
            DimensionKind::Radius | DimensionKind::Diameter,
            DimensionPoints::Radial {
                center,
                point_on_arc,
                // `leader_length` is a length, not a point: it has no handle and
                // is preserved verbatim by every drag (see the drag rule).
                leader_length: _,
            },
        ) => vec![
            slot("centre (group 10)", *center),
            slot("point on arc (group 15)", *point_on_arc),
        ],
        (
            DimensionKind::Angular2Ln,
            DimensionPoints::Angular2Ln {
                line1_start,
                line1_end,
                line2_start,
                line2_end,
                arc_point,
            },
        ) => vec![
            slot("line 1 start (group 13)", *line1_start),
            slot("line 1 end (group 14)", *line1_end),
            slot("line 2 start (group 15)", *line2_start),
            slot("line 2 end (group 10)", *line2_end),
            slot("arc location (group 16)", *arc_point),
        ],
        (
            DimensionKind::Angular3Pt,
            DimensionPoints::Angular3Pt {
                vertex,
                first,
                second,
                arc_point,
            },
        ) => vec![
            slot("vertex (group 15)", *vertex),
            slot("first point (group 13)", *first),
            slot("second point (group 14)", *second),
            slot("arc location (group 10)", *arc_point),
        ],
        (
            DimensionKind::Ordinate { .. },
            DimensionPoints::Ordinate {
                feature,
                leader_end,
                base,
            },
        ) => vec![
            slot("feature point (group 13)", *feature),
            slot("leader end (group 14)", *leader_end),
            slot("datum base (group 10)", *base),
        ],
        _ => return None,
    };
    // Group 11 is carried by every subtype, so it is always the last handle.
    slots.push(slot("text middle point (group 11)", definition.text_middle));
    Some(slots)
}

/// The [`GripKind::DefinitionPoint`] handles `dimension` exposes, in order.
///
/// A dimension whose stored subtype and point slots disagree exposes **no** grip
/// at all (see [`slots`]); every other dimension exposes one handle per DXF
/// definition point plus the text middle point, so between three and six handles
/// depending on the subtype. Each position is the stored point verbatim.
pub(super) fn definition_point_grips(entity: EntityId, dimension: &DimensionGeometry) -> Vec<Grip> {
    let Some(slots) = slots(dimension.definition()) else {
        return Vec::new();
    };
    slots
        .into_iter()
        .enumerate()
        .map(|(index, slot)| Grip {
            id: GripId {
                entity,
                kind: GripKind::DefinitionPoint,
                // At most six slots, so the `u8` conversion is exact.
                index: index as u8,
            },
            position: slot.position,
        })
        .collect()
}

/// A `grip.dimension_inconsistent` refusal (the definition's own bug signal).
pub(super) fn dimension_inconsistent(cause: &str) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_DIMENSION_INCONSISTENT}: {cause}; a dimension whose subtype and definition points \
         disagree exposes no DefinitionPoint grip and is never repaired by this model"
    ))
}

/// A `grip.dimension_degenerate` refusal naming the slot the drag moved.
pub(super) fn dimension_degenerate(slot: &str, cause: &str) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_DIMENSION_DEGENERATE}: moving the AcDbDimension {slot} cannot be stored: {cause}; \
         a dimension must keep a finite measurement and a non-empty synthesized display, so the \
         result is refused by name and never clamped"
    ))
}

/// The stored definition with the slot at `index` replaced by `to`.
///
/// `index` must be one [`slots`] names; `None` means it is not a slot of this
/// subtype (the caller has already refused that case as
/// [`super::REASON_STALE_HANDLE`], so this is a defensive second check).
///
/// Only the one point is written: `kind` (including the subtype scalars
/// `rotation`/`x_ordinate`), `text_rotation` and `normal` are carried by the
/// clone, and the untouched point slots plus `leader_length` are copied verbatim.
fn with_point(
    definition: &DimensionDefinition,
    index: usize,
    to: Point3,
) -> Option<DimensionDefinition> {
    let mut next = definition.clone();
    // The list from [`slots`] is the single statement of the order: its last entry
    // is the text middle point, so the point-slot count is one less than its length.
    // Re-deriving that count here from the subtype again would be a second table to
    // keep in sync.
    let point_count = slots(definition)?.len() - 1;
    if index == point_count {
        next.text_middle = to;
        return Some(next);
    }
    match (&definition.kind, &mut next.points) {
        (
            DimensionKind::Linear { .. } | DimensionKind::Aligned,
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            },
        ) => match index {
            0 => *first = to,
            1 => *second = to,
            2 => *dimension_line = to,
            _ => return None,
        },
        (
            DimensionKind::Radius | DimensionKind::Diameter,
            DimensionPoints::Radial {
                center,
                point_on_arc,
                leader_length: _,
            },
        ) => match index {
            0 => *center = to,
            1 => *point_on_arc = to,
            _ => return None,
        },
        (
            DimensionKind::Angular2Ln,
            DimensionPoints::Angular2Ln {
                line1_start,
                line1_end,
                line2_start,
                line2_end,
                arc_point,
            },
        ) => match index {
            0 => *line1_start = to,
            1 => *line1_end = to,
            2 => *line2_start = to,
            3 => *line2_end = to,
            4 => *arc_point = to,
            _ => return None,
        },
        (
            DimensionKind::Angular3Pt,
            DimensionPoints::Angular3Pt {
                vertex,
                first,
                second,
                arc_point,
            },
        ) => match index {
            0 => *vertex = to,
            1 => *first = to,
            2 => *second = to,
            3 => *arc_point = to,
            _ => return None,
        },
        (
            DimensionKind::Ordinate { .. },
            DimensionPoints::Ordinate {
                feature,
                leader_end,
                base,
            },
        ) => match index {
            0 => *feature = to,
            1 => *leader_end = to,
            2 => *base = to,
            _ => return None,
        },
        _ => return None,
    };
    Some(next)
}

/// The `DIMENSION` drag rule: one definition point (or the text middle point),
/// then a **re-synthesis** of the display from the mutated definition.
///
/// ## Re-synthesis, not a display patch
///
/// A dimension's display is `f(definition, style)` — `cad-domain` is the single
/// builder of it (ADR 0017 A.4 invariant 1) and `cad-app` never writes dimension
/// children by hand. The rule therefore clones the stored definition, replaces
/// exactly one point, and rebuilds the entity through
/// [`DimensionGeometry::synthesized`] with the **same** `style_name`, `style` and
/// `text_override` it read from the entity. What that preserves:
///
/// * `kind` (so the subtype scalars — `Linear { rotation }`, `Ordinate
///   { x_ordinate }` — are untouched), `text_rotation` and `normal`: they live in
///   the cloned definition and are never rewritten;
/// * every point slot the drag does not address, and `leader_length` (a scalar
///   that is not a handle);
/// * `style_name`, `style` and `text_override` (the constructor takes all three,
///   so nothing is dropped): a user text override keeps its exact string, and the
///   style snapshot is not re-resolved from the DIMSTYLE table;
/// * the derivation itself: `measurement` and the display (extension lines,
///   dimension line, arrows, text) are recomputed from the mutated definition, so
///   group 42 and the drawn number stay in step.
///
/// The text middle point is a handle for the same reason the definition points
/// are: it is a stored DXF point (group 11). It is **not** a text-content grip —
/// the override string is preserved, never replaced.
///
/// ## Refusals (never clamped, never patched around)
///
/// * [`super::REASON_STALE_HANDLE`] — a non-[`GripKind::DefinitionPoint`] kind on
///   a dimension, or an `index` past this subtype's handle count (a `Radial`
///   dimension has no index `3`, a `Linear` one has no index `4`);
/// * [`super::REASON_NON_FINITE_TARGET`] — a `NaN`/infinite target;
/// * [`super::REASON_ZERO_DISPLACEMENT`] — the target equals that slot's stored
///   point (IEEE `==`, no tolerance);
/// * [`REASON_DIMENSION_INCONSISTENT`] — the stored definition is
///   self-contradictory, or the mutated one stops being consistent. The second
///   case is a **bug signal**: replacing one point in place cannot change the
///   subtype/points pairing and the target is already known finite;
/// * [`REASON_DIMENSION_DEGENERATE`] — the mutated definition has no synthesizable
///   display (a `Radial` `point_on_arc` dragged onto its centre, an angular
///   `arc_point` dragged onto the vertex, two lines made parallel, …), or the
///   synthesis would yield a non-finite measurement or an empty display. Refused
///   **instead of** storing a `NaN` group 42 or an empty display; the synthesis is
///   what decides, and it is called exactly once.
///
/// ## Declared limitation: the display-source branch
///
/// `cad-domain` keeps `DisplaySource` private, so `cad-app` cannot observe whether
/// a dimension's display is our synthesis or an imported anonymous-block picture
/// ([`DimensionGeometry::from_picture`]), and no constructor rebuilds the picture
/// branch from a mutated definition. A drag therefore always re-synthesizes: a
/// picture-branch dimension becomes a synthesized one (its measurement is
/// recomputed from the mutated definition and its display is our synthesis). That
/// is declared here rather than approximated silently — the alternative would be
/// to keep an imported picture that no longer matches any stored definition point.
pub(super) fn apply_dimension(
    dimension: &DimensionGeometry,
    id: &GripId,
    to: Point3,
) -> CadResult<SemanticGeometry> {
    if id.kind != GripKind::DefinitionPoint {
        return Err(stale_handle(
            "AcDbDimension exposes DefinitionPoint grips only (one per DXF definition point, then \
             the text middle point)"
                .to_string(),
        ));
    }
    let definition = dimension.definition();
    let Some(slots) = slots(definition) else {
        return Err(dimension_inconsistent(
            "the stored definition's subtype and point slots disagree",
        ));
    };
    let index = usize::from(id.index);
    let Some(slot) = slots.get(index) else {
        return Err(stale_handle(format!(
            "AcDbDimension has no DefinitionPoint grip at index {index} (this subtype exposes {} \
             definition points and then the text middle point)",
            slots.len() - 1
        )));
    };
    finite_target(to)?;
    if to == slot.position {
        return Err(zero_displacement(&format!("AcDbDimension {}", slot.name)));
    }
    let Some(mutated) = with_point(definition, index, to) else {
        // Unreachable: `index` was validated against the very same order one line
        // above, so this is a bug signal rather than a user-facing refusal.
        return Err(dimension_inconsistent(
            "the addressed slot is not a point of the stored definition",
        ));
    };
    if !mutated.is_consistent() {
        return Err(dimension_inconsistent(
            "the mutated definition is no longer self-consistent",
        ));
    }
    let rebuilt = DimensionGeometry::synthesized(
        mutated,
        dimension.style_name().to_string(),
        dimension.style().clone(),
        dimension.text_override().map(str::to_string),
    )
    .map_err(|_| {
        dimension_degenerate(
            slot.name,
            "the mutated definition has no synthesizable display (degenerate points)",
        )
    })?;
    // Defence in depth: a `NaN` measurement or an empty display would be silent
    // corruption, so it is refused by name instead of being written back.
    if !rebuilt.measurement().is_finite() || rebuilt.display().is_empty() {
        return Err(dimension_degenerate(
            slot.name,
            "the synthesis of the mutated definition is not finite or has an empty display",
        ));
    }
    Ok(SemanticGeometry::Dimension(Box::new(rebuilt)))
}
