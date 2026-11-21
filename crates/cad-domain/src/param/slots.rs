//! The closed slot registry and slot rewriting (ADR-0043 D3).

use super::{ParamError, ParamKind};
use super::{PARAM_GEOMETRY_FAILED, PARAM_NO_EVALUATOR, PARAM_UNKNOWN_SLOT};
use crate::SemanticGeometry;

// ---------------------------------------------------------------------------
// Slots.
// ---------------------------------------------------------------------------

/// One evaluator-known field of a type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamSlot {
    pub name: &'static str,
    pub kind: ParamKind,
}

const CIRCLE_SLOTS: &[ParamSlot] = &[ParamSlot {
    name: "radius",
    kind: ParamKind::Length,
}];
const POLYLINE_SLOTS: &[ParamSlot] = &[ParamSlot {
    name: "width",
    kind: ParamKind::Length,
}];
const TEXT_SLOTS: &[ParamSlot] = &[ParamSlot {
    name: "height",
    kind: ParamKind::Length,
}];

/// The closed slot registry for one `cad:*` type key (D3).
///
/// P0 deliberately registers only slots that do not change topology. Endpoint
/// driving (a rectangle's four lines, an array's items) needs P1 slot semantics
/// and is refused with [`PARAM_NO_EVALUATOR`] instead of guessed.
pub fn param_slots(type_key: &str) -> &'static [ParamSlot] {
    match type_key {
        "cad:Circle" => CIRCLE_SLOTS,
        "cad:Polyline" => POLYLINE_SLOTS,
        "cad:Text" => TEXT_SLOTS,
        _ => &[],
    }
}

/// Look one slot up, or `None` when the type does not register it.
pub fn param_slot(type_key: &str, slot: &str) -> Option<ParamSlot> {
    param_slots(type_key)
        .iter()
        .copied()
        .find(|candidate| candidate.name == slot)
}

/// The `cad:*` type key of a geometry, when the type has an evaluator family.
///
/// `cad:Insert` is deliberately absent in P0: its `scale` slot needs the P1
/// transform split, so a binding on an insert refuses with
/// [`PARAM_NO_EVALUATOR`] instead of reporting an unknown slot.
pub fn geometry_type_key(geometry: &SemanticGeometry) -> Option<&'static str> {
    match geometry {
        SemanticGeometry::Circle { .. } => Some("cad:Circle"),
        SemanticGeometry::Polyline { .. } => Some("cad:Polyline"),
        SemanticGeometry::Text { .. } => Some("cad:Text"),
        _ => None,
    }
}

/// Rewrite one slot of a geometry, leaving everything else untouched (D7).
///
/// The construction rules are the same ones `cad-db` validates for imported
/// geometry: a radius or text height must be finite and strictly positive, a
/// polyline width finite and non-negative. A violation is
/// [`PARAM_GEOMETRY_FAILED`] — the caller keeps last-good geometry (D6).
pub fn apply_slot(
    geometry: &SemanticGeometry,
    slot: &str,
    value: f64,
) -> Result<SemanticGeometry, ParamError> {
    let type_key = geometry_type_key(geometry).ok_or_else(|| {
        ParamError::new(
            PARAM_NO_EVALUATOR,
            format!("this geometry type has no parameter evaluator (slot {slot:?})"),
        )
    })?;
    let spec = param_slot(type_key, slot).ok_or_else(|| {
        ParamError::new(
            PARAM_UNKNOWN_SLOT,
            format!("{type_key} does not register the slot {slot:?}"),
        )
    })?;
    if !value.is_finite() {
        return Err(ParamError::new(
            PARAM_GEOMETRY_FAILED,
            format!("slot {slot:?} evaluated to the non-finite value {value}"),
        ));
    }
    let _ = spec;
    let mut next = geometry.clone();
    match (&mut next, slot) {
        (SemanticGeometry::Circle { radius, .. }, "radius") => {
            if value <= 0.0 {
                return Err(ParamError::new(
                    PARAM_GEOMETRY_FAILED,
                    format!("a circle radius must be positive, got {value}"),
                ));
            }
            *radius = value;
        }
        (SemanticGeometry::Polyline { width, .. }, "width") => {
            if value < 0.0 {
                return Err(ParamError::new(
                    PARAM_GEOMETRY_FAILED,
                    format!("a polyline width must be non-negative, got {value}"),
                ));
            }
            *width = value;
        }
        (SemanticGeometry::Text { height, .. }, "height") => {
            if value <= 0.0 {
                return Err(ParamError::new(
                    PARAM_GEOMETRY_FAILED,
                    format!("a text height must be positive, got {value}"),
                ));
            }
            *height = value;
        }
        _ => {
            return Err(ParamError::new(
                PARAM_UNKNOWN_SLOT,
                format!("{type_key} does not accept the slot {slot:?}"),
            ));
        }
    }
    Ok(next)
}
