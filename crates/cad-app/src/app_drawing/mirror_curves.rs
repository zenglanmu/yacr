//! MIRROR's explicit `Arc`/`Ellipse` arms.
//!
//! ## Why this module exists
//!
//! A reflection is an **orientation-reversing** transform. `cad_db`'s generic
//! transform surface (`transform_geometry`) deliberately refuses it for `Arc`,
//! `Ellipse`, `Text` and `Shape`: a reflected parameterisation would have to be
//! rewritten by hand there, and that surface's job is the proper (rotation +
//! uniform scale) similarity. Rather than loosening that generic contract, MIRROR
//! handles the two curved kinds **explicitly here** — the one command that really
//! is a reflection — and delegates every other kind to `transform_geometry`
//! unchanged (so `Text`/`Shape`/`Opaque` keep their existing refusals).
//!
//! ## The declared rule: the plane normal is negated
//!
//! The mirrored entity keeps the plane of the reflected curve but stores the
//! **opposite** extrusion normal:
//!
//! ```text
//! centre' = R(centre)      normal' = −R(normal)      radius' = radius
//! ```
//!
//! `R(normal)` alone would point out of the reflected plane on the wrong side.
//! Negating it restores the honest orientation of the reflected *plane*, and — the
//! part that decides the stored parameters — keeps `(R·ax, R·ay)` a **right-handed**
//! frame for `normal'`, exactly like [`cad_geometry::arbitrary_axis`] builds for
//! every other arc in this build:
//!
//! ```text
//! (R·ax) × (R·ay) = det(R)·R·(ax × ay) = −R(normal) = normal'
//! ```
//!
//! ### Arc: the start angle is re-derived, the sweep keeps its sign
//!
//! With `normal'` negated as above, the reflected curve is
//! `R(P(t)) = centre' + radius·(cos(t + α)·ax' + sin(t + α)·ay')` for the fixed
//! frame rotation `α` between the two right-handed frames. So the stored start is
//! the **mirrored start point's angle in the mirrored frame** (never a three-point
//! re-derivation), and the parameter still advances with `+t`: the **sweep keeps
//! its sign**. Its magnitude and sign both stay, which is what makes
//! `mirror(start) → mirror(end)` the stored `start → end` (this is pinned by the
//! endpoint tests). The opposite-sweep alternative only holds if the normal is
//! **not** negated; the two choices cannot be mixed, and the negated-normal one is
//! the one this build implements.
//!
//! ### Ellipse: the parameter is preserved (elliptical arcs are supported)
//!
//! For an ellipse `P(θ) = c + cos θ·A + sin θ·B` with `B` along
//! `ellipse_minor_dir(normal, Â)·|A|·|ratio|`, the reflected axis is `A' = R·A`
//! and the canonical minor direction of the mirrored frame is
//!
//! ```text
//! cross(normal', Â') = cross(−R·n, R·Â) = R·cross(n, Â)
//! ```
//!
//! so `R(P(θ)) = centre' + cos θ·A' + sin θ·B'`: the **same** `θ`. The stored
//! `start`/`sweep` are therefore copied verbatim — a full ellipse and an
//! **elliptical arc** (start/sweep params) are both exact, with **no** `θ' = −θ`
//! mapping needed *because* the normal is negated. This module implements the
//! elliptical arc rather than refusing it; `mirror.elliptical_arc` does not exist.
//! The tests assert that the mirrored start, end **and** mid-parameter points equal
//! the reflections of the originals, which is what proves the parameter mapping.

use super::*;
use cad_geometry::{add, arbitrary_axis, dot, length, normalize, scale, sub};

/// Below this the source normal is treated as unusable and the mirrored normal
/// falls back to `−Z` (the reflected `+Z` default every other kind starts at).
const DEGENERATE_NORMAL: f64 = 1e-12;

/// Reflect one entity's geometry for MIRROR.
///
/// `Arc`/`Ellipse` are handled here; every other kind keeps the generic
/// `transform_geometry` behaviour (including its `Text`/`Shape`/`Opaque`
/// refusals). A **proper** transform (positive determinant) is not a reflection, so
/// it is delegated too.
pub(super) fn mirror_geometry(
    geometry: &SemanticGeometry,
    transform: &Transform3,
) -> CadResult<SemanticGeometry> {
    if transform.determinant() >= 0.0 {
        return transform_geometry(geometry, transform);
    }
    match geometry {
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => mirror_arc(*center, *normal, *radius, *start, *sweep, transform),
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => mirror_ellipse(
            *center,
            *normal,
            *major_axis,
            *ratio,
            *start,
            *sweep,
            transform,
        ),
        other => transform_geometry(other, transform),
    }
}

/// `ARC`: reflect the centre and the plane, re-derive the start angle from the
/// mirrored start point, keep the sweep (magnitude **and** sign).
fn mirror_arc(
    center: Point3,
    normal: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
    transform: &Transform3,
) -> CadResult<SemanticGeometry> {
    let new_center = transform.apply_point(center);
    let new_normal = reflected_normal(normal, transform);
    let radius = radius.abs();
    let (ax, ay, _) = arbitrary_axis(normal);
    let start_point = arc_point(center, radius, (ax, ay), start);
    let reflected_start = transform.apply_point(start_point);
    let (new_ax, new_ay, _) = arbitrary_axis(new_normal);
    let new_start = plane_angle(reflected_start, new_center, new_ax, new_ay);
    if !is_finite_point(new_center) || !new_start.is_finite() || !sweep.is_finite() {
        return Err(CadError::InvalidInput(
            "MIRROR cannot mirror an ARC: the reflected parameters are not finite".into(),
        ));
    }
    // The mirrored entity is a directed curve: it runs mirror(start) →
    // mirror(end), which is exactly what the re-derived start plus the unchanged
    // sweep describe in the mirrored (right-handed) frame.
    Ok(SemanticGeometry::Arc {
        center: new_center,
        normal: new_normal,
        radius,
        start: new_start,
        sweep,
    })
}

/// `ELLIPSE`: reflect the centre and the major axis, keep the ratio and the
/// parameter (`start`/`sweep`), negate the normal.
///
/// This covers a **full** ellipse (`sweep = τ`) and an **elliptical arc** alike;
/// see the module docs for the proof that the parameter is preserved under the
/// negated-normal convention.
fn mirror_ellipse(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
    transform: &Transform3,
) -> CadResult<SemanticGeometry> {
    let new_center = transform.apply_point(center);
    let new_major = transform.apply_vector(major_axis);
    let new_normal = reflected_normal(normal, transform);
    if !is_finite_point(new_center)
        || !is_finite_point(new_major)
        || !start.is_finite()
        || !sweep.is_finite()
    {
        return Err(CadError::InvalidInput(
            "MIRROR cannot mirror an ELLIPSE: the reflected parameters are not finite".into(),
        ));
    }
    if length(new_major) < DEGENERATE_NORMAL {
        return Err(CadError::InvalidInput(
            "MIRROR cannot mirror an ELLIPSE: the reflected major axis is degenerate".into(),
        ));
    }
    Ok(SemanticGeometry::Ellipse {
        center: new_center,
        normal: new_normal,
        major_axis: new_major,
        ratio,
        start,
        sweep,
    })
}

/// The mirrored plane normal: `−R(normal)`, normalised.
///
/// Negating the reflected normal is what keeps the mirrored frame right-handed
/// (see the module docs); a source normal too short to normalise falls back to the
/// reflected `+Z`, i.e. `−Z`.
fn reflected_normal(normal: Point3, transform: &Transform3) -> Point3 {
    let mapped = transform.apply_vector(normal);
    if !length(mapped).is_finite() || length(mapped) < DEGENERATE_NORMAL {
        return Point3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        };
    }
    scale(normalize(mapped), -1.0)
}

/// One point on the arc: `centre + radius·(cos(angle)·ax + sin(angle)·ay)`.
///
/// Term for term the tessellator's expression over
/// [`cad_geometry::arbitrary_axis`], so the derived start angle measures the same
/// frame the display draws.
fn arc_point(center: Point3, radius: f64, axes: (Point3, Point3), angle: f64) -> Point3 {
    add(
        center,
        add(
            scale(axes.0, angle.cos() * radius),
            scale(axes.1, angle.sin() * radius),
        ),
    )
}

/// The in-plane angle of `point` about `center` in the orthonormal frame
/// `(ax, ay)`: `atan2(dot(d, ay), dot(d, ax))`.
fn plane_angle(point: Point3, center: Point3, ax: Point3, ay: Point3) -> f64 {
    let d = sub(point, center);
    dot(d, ay).atan2(dot(d, ax))
}
