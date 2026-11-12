//! OFFSET with an explicit **fixed distance** (the `OFFSET` distance prompt, as
//! opposed to the bare `Through` pick).
//!
//! Split out of `trim_extend.rs` (the code-size limit) where these helpers had
//! drifted next to the TRIM/EXTEND handlers; the behaviour is unchanged: each one
//! reuses the same `Through` planner (`offset_line` / `offset_circle` /
//! `offset_arc`) to learn the pick's side, then moves the curve by the requested
//! distance on that side. A distance that would collapse a radius is an explicit
//! `offset.collapsed` refusal, never a clamped radius.

use super::*;

/// Offset a LINE by a fixed distance to the pick's side (`Through` when
/// `distance` is `None`).
pub(super) fn offset_line_with_distance(
    start: Point3,
    end: Point3,
    side_point: Point3,
    distance: Option<f64>,
) -> CadResult<(SemanticGeometry, f64)> {
    let (probe, picked) = offset_line(start, end, side_point)?;
    let Some(requested) = distance else {
        return Ok((probe, picked));
    };
    let SemanticGeometry::Line {
        start: probe_start, ..
    } = probe
    else {
        return Err(CadError::Invariant(
            "the LINE offset planner returned a non-line".into(),
        ));
    };
    let unit = cad_geometry::scale(
        cad_geometry::sub(probe_start, start),
        1.0 / picked.max(f64::MIN_POSITIVE),
    );
    Ok((
        SemanticGeometry::Line {
            start: cad_geometry::add(start, cad_geometry::scale(unit, requested)),
            end: cad_geometry::add(end, cad_geometry::scale(unit, requested)),
        },
        requested,
    ))
}

/// Offset a CIRCLE by a fixed distance to the pick's side (`Through` when
/// `distance` is `None`). A point outside grows the radius, inside shrinks it.
pub(super) fn offset_circle_with_distance(
    center: Point3,
    normal: Point3,
    radius: f64,
    side_point: Point3,
    distance: Option<f64>,
) -> CadResult<(SemanticGeometry, f64)> {
    let (probe, picked) = offset_circle(center, normal, radius, side_point)?;
    let Some(requested) = distance else {
        return Ok((probe, picked));
    };
    let tolerance = super::helpers::offset_tolerance();
    let stored = radius.abs();
    let sign = if picked > stored { 1.0 } else { -1.0 };
    let new_radius = stored + sign * requested;
    if !new_radius.is_finite() || new_radius <= tolerance {
        return Err(CadError::Unsupported(
            "OFFSET offset.collapsed: the fixed distance would collapse the circle's radius".into(),
        ));
    }
    Ok((
        SemanticGeometry::Circle {
            center,
            normal,
            radius: new_radius,
        },
        requested,
    ))
}

/// Offset an ARC by a fixed distance to the pick's side (`Through` when
/// `distance` is `None`). The span and frame are preserved; only the radius
/// changes, exactly like the `Through` arm.
pub(super) fn offset_arc_with_distance(
    center: Point3,
    normal: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
    side_point: Point3,
    distance: Option<f64>,
) -> CadResult<(SemanticGeometry, f64)> {
    let (probe, picked) = offset_arc(center, normal, radius, start, sweep, side_point)?;
    let Some(requested) = distance else {
        return Ok((probe, picked));
    };
    let tolerance = super::helpers::offset_tolerance();
    let sign = if picked > radius.abs() { 1.0 } else { -1.0 };
    let new_radius = radius.abs() + sign * requested;
    if !new_radius.is_finite() || new_radius <= tolerance {
        return Err(CadError::Unsupported(
            "OFFSET offset.collapsed: the fixed distance would collapse the arc's radius".into(),
        ));
    }
    Ok((
        SemanticGeometry::Arc {
            center,
            normal,
            radius: new_radius,
            start,
            sweep,
        },
        requested,
    ))
}
