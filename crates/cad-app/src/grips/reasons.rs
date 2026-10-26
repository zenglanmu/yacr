//! The `grip.*` refusal constructors — one place per stable reason code.
//!
//! Split out of `grips/drag.rs` so that file stays well under the 1000-line cap
//! now that every handle family (LINE, CIRCLE, ARC, ELLIPSE, POLYLINE, SPLINE,
//! TEXT, INSERT, DIMENSION) has a drag rule. Nothing here decides anything: each
//! helper only formats one stable code with the detail the rule passes in, and the
//! reason codes themselves (and their documented meanings) live in the parent
//! module.
//!
//! [`super::REASON_DIMENSION_INCONSISTENT`] and
//! [`super::REASON_DIMENSION_DEGENERATE`] are raised by `grips/dimension.rs`,
//! which owns that family's wording; everything else is here.

use super::{
    is_finite, REASON_DEGENERATE_ARC, REASON_DEGENERATE_RADIUS, REASON_DEGENERATE_TRANSFORM,
    REASON_NON_FINITE_TARGET, REASON_PARTIAL_ELLIPSE, REASON_STALE_HANDLE, REASON_UNSUPPORTED_KIND,
    REASON_ZERO_DISPLACEMENT,
};
use cad_domain::{CadError, CadResult, Point3};

/// A `grip.unsupported_kind` refusal naming the DXF class.
pub(super) fn unsupported_kind(class: &str) -> CadError {
    CadError::Unsupported(format!(
        "{REASON_UNSUPPORTED_KIND}: {class} exposes no grippable handle"
    ))
}

/// A `grip.stale_handle` refusal with the specific mismatch.
pub(super) fn stale_handle(detail: String) -> CadError {
    CadError::InvalidInput(format!("{REASON_STALE_HANDLE}: {detail}"))
}

/// A `grip.zero_displacement` refusal naming the handle that did not move.
pub(super) fn zero_displacement(handle: &str) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_ZERO_DISPLACEMENT}: the {handle} is already at the drag target"
    ))
}

/// A `grip.zero_displacement` refusal for a quadrant drag whose radius is
/// already stored (an off-plane target at the current radius).
pub(super) fn unchanged_radius(radius: f64) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_ZERO_DISPLACEMENT}: the AcDbCircle quadrant drag leaves the radius at {radius}"
    ))
}

/// A `grip.degenerate_radius` refusal naming the rejected radius.
pub(super) fn degenerate_radius(radius: f64) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_DEGENERATE_RADIUS}: the AcDbCircle quadrant drag yields radius {radius}; \
         a circle radius must be positive and finite (the centre is not moved by this handle)"
    ))
}

/// A `grip.degenerate_radius` refusal for an ellipse quadrant drag, naming the
/// cause (a measured radius or a stored frame that cannot yield one).
pub(super) fn degenerate_ellipse_radius(cause: &str) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_DEGENERATE_RADIUS}: the AcDbEllipse quadrant drag yields no usable in-plane \
         radius: {cause}; an ellipse semi-axis must be positive and finite (the centre and the \
         normal are not moved by this handle)"
    ))
}

/// A `grip.zero_displacement` refusal for an ellipse quadrant drag whose measured
/// radius is already stored (the axis direction never rotates).
pub(super) fn unchanged_ellipse_radius(radius: f64) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_ZERO_DISPLACEMENT}: the AcDbEllipse quadrant drag leaves the axis radius at \
         {radius}"
    ))
}

/// A `grip.partial_ellipse` refusal naming the quadrant handle that a partial
/// ellipse (elliptical arc) does not expose.
pub(super) fn partial_ellipse(index: u8) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_PARTIAL_ELLIPSE}: AcDbEllipse quadrant {index} is not a handle of a partial \
         ellipse; a stored parameter range that is not a full turn exposes the Center grip only"
    ))
}

/// A `grip.degenerate_arc` refusal naming the rejected sweep.
pub(super) fn degenerate_arc(handle: &str, sweep: f64) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_DEGENERATE_ARC}: the {handle} drag yields sweep {sweep}; an arc sweep must \
         stay finite and strictly inside (0, 2*pi) in its stored direction \
         (the centre, radius and normal are not moved by this handle)"
    ))
}

/// A `grip.degenerate_transform` refusal for a block reference whose stored
/// placement cannot be written back (a non-finite coefficient).
pub(super) fn degenerate_transform(cause: &str) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_DEGENERATE_TRANSFORM}: the AcDbBlockReference insert transform cannot be moved \
         rigidly: {cause}; a translation column rewrite keeps every other coefficient verbatim, \
         so the result would be an unstorable transform"
    ))
}

/// A `grip.non_finite_target` refusal for a finite input whose result overflows.
pub(super) fn overflowed(handle: &str) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_NON_FINITE_TARGET}: translating the {handle} produces a non-finite coordinate"
    ))
}

/// Refuse a target that could not be stored as a world point.
///
/// Exposed to the `GRIPSTRETCH` command so it can refuse a non-finite target
/// **before** it even reads the drawing, with the same code and message the drag
/// rule would produce.
pub(crate) fn finite_target(to: Point3) -> CadResult<()> {
    if is_finite(to) {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "{REASON_NON_FINITE_TARGET}: ({}, {}, {})",
            to.x, to.y, to.z
        )))
    }
}
