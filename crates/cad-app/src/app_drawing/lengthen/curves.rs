//! LENGTHEN's open-polyline, elliptical-arc and open-spline length solvers.
//!
//! Split out of `lengthen.rs` so both files stay inside the code-size limit.
//! These are only reached through [`super::lengthen_geometry`]; the settings,
//! refusals and transaction shape stay in the parent module.

use super::super::arc_length::ArcLengthTable;
use super::super::polyline_curve::PolylineCurve;
use super::super::trim_curves::{cut_tolerance, TargetCurve};
use super::{error_detail, new_line_length, refusal, LengthenMode, LengthenSettings};
use cad_domain::{CadResult, Point3, SemanticGeometry};
use cad_geometry::distance;

/// `LENGTHEN` on an open polyline: length along the polyline, with the end
/// nearest the pick moving. Trimming splits at the new endpoint through the
/// exact sub-curve helper; extending appends one straight tangent segment.
pub(super) fn lengthen_polyline(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    settings: &LengthenSettings,
    pick: Option<Point3>,
) -> CadResult<(SemanticGeometry, f64, f64, bool)> {
    let eps = cut_tolerance();
    if closed {
        return Err(refusal(
            "lengthen.closed",
            "LENGTHEN does not affect a closed polyline",
        ));
    }
    let curve = PolylineCurve::new(points, bulges, false, "lengthen")?;
    let old = curve.total_length();
    if !old.is_finite() || old <= eps {
        return Err(refusal(
            "lengthen.degenerate",
            "the LENGTHEN target polyline has no measurable length",
        ));
    }
    if settings.mode == LengthenMode::Dynamic {
        return Err(refusal(
            "lengthen.invalid_payload",
            "the dynamic LENGTHEN mode is resolved from its points, not a scalar length",
        ));
    }
    let new = new_line_length(old, settings)?;
    let first = points[0];
    let last = *points.last().ok_or_else(|| {
        refusal(
            "lengthen.degenerate",
            "the LENGTHEN target polyline has no vertices",
        )
    })?;
    let move_start = pick.is_some_and(|point| distance(point, first) < distance(point, last));
    let changed = (new - old).abs() > eps;
    if !changed {
        return Ok((
            SemanticGeometry::Polyline {
                points: points.to_vec(),
                bulges: bulges.to_vec(),
                closed: false,
                width: 0.0,
            },
            old,
            old,
            false,
        ));
    }
    if new < old {
        let n = curve.segment_count() as f64;
        let (u0, u1) = if move_start {
            (polyline_param_at_distance(&curve, old - new), n)
        } else {
            (0.0, polyline_param_at_distance(&curve, new))
        };
        let geometry = curve
            .sub_curve(u0, u1)
            .map_err(|reason| refusal("lengthen.degenerate", error_detail(reason)))?;
        return Ok((geometry, old, new, true));
    }

    let delta = new - old;
    let mut points = points.to_vec();
    let mut bulges = bulges.to_vec();
    bulges.resize(points.len().saturating_sub(1), 0.0);
    if move_start {
        let tangent = polyline_end_tangent(&curve, true).ok_or_else(|| {
            refusal(
                "lengthen.degenerate",
                "the LENGTHEN polyline start has no tangent direction",
            )
        })?;
        let start = Point3 {
            x: first.x - tangent.x * delta,
            y: first.y - tangent.y * delta,
            z: first.z - tangent.z * delta,
        };
        if !is_finite_point(start) {
            return Err(refusal(
                "lengthen.non_positive",
                "the LENGTHEN result is not finite",
            ));
        }
        points.insert(0, start);
        bulges.insert(0, 0.0);
    } else {
        let tangent = polyline_end_tangent(&curve, false).ok_or_else(|| {
            refusal(
                "lengthen.degenerate",
                "the LENGTHEN polyline end has no tangent direction",
            )
        })?;
        let end = Point3 {
            x: last.x + tangent.x * delta,
            y: last.y + tangent.y * delta,
            z: last.z + tangent.z * delta,
        };
        if !is_finite_point(end) {
            return Err(refusal(
                "lengthen.non_positive",
                "the LENGTHEN result is not finite",
            ));
        }
        points.push(end);
        bulges.push(0.0);
    }
    Ok((
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed: false,
            width: 0.0,
        },
        old,
        new,
        true,
    ))
}

/// Whether every coordinate of `point` is finite.
fn is_finite_point(point: Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

/// The polyline parameter (one unit per segment) at `target` arc length.
fn polyline_param_at_distance(curve: &PolylineCurve, target: f64) -> f64 {
    let total = curve.total_length();
    let n = curve.segment_count() as f64;
    if !target.is_finite() || target <= 0.0 {
        return 0.0;
    }
    if target >= total {
        return n;
    }
    let mut traversed = 0.0;
    for segment in 0..curve.segment_count() {
        let length = curve.segment_length(segment);
        if traversed + length >= target {
            let fraction = if length > 0.0 {
                (target - traversed) / length
            } else {
                0.0
            };
            return segment as f64 + fraction;
        }
        traversed += length;
    }
    n
}

/// The unit tangent at the polyline's start or end, in the direction of travel.
fn polyline_end_tangent(curve: &PolylineCurve, at_start: bool) -> Option<Point3> {
    let n = curve.segment_count();
    if n == 0 {
        return None;
    }
    let (from, to) = if at_start {
        (1e-6, 1e-4)
    } else {
        (1.0 - 1e-4, 1.0 - 1e-6)
    };
    let segment = if at_start { 0.0 } else { (n - 1) as f64 };
    let a = curve.point_at(segment + from);
    let b = curve.point_at(segment + to);
    let direction = Point3 {
        x: b.x - a.x,
        y: b.y - a.y,
        z: b.z - a.z,
    };
    let length = direction.x.hypot(direction.y).hypot(direction.z);
    if !length.is_finite() || length <= 1e-12 {
        return None;
    }
    Some(Point3 {
        x: direction.x / length,
        y: direction.y / length,
        z: direction.z / length,
    })
}

/// `LENGTHEN` on an elliptical arc (`Ellipse` with a partial sweep): length
/// along the ellipse for Delta/Percent, included angle in degrees for Total.
#[allow(clippy::too_many_arguments)]
pub(super) fn lengthen_ellipse(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
    settings: &LengthenSettings,
    pick: Option<Point3>,
) -> CadResult<(SemanticGeometry, f64, f64, bool)> {
    let eps = cut_tolerance();
    let ratio = ratio.abs();
    let (major, minor) = ellipse_frame(center, normal, major_axis, ratio)?;
    if !sweep.is_finite() || sweep.abs() <= eps {
        return Err(refusal(
            "lengthen.degenerate",
            "the LENGTHEN target ELLIPSE has no measurable length",
        ));
    }
    if sweep.abs() >= std::f64::consts::TAU - 1e-9 {
        return Err(refusal(
            "lengthen.closed",
            "LENGTHEN does not affect a closed object such as a full ellipse",
        ));
    }
    let old = ellipse_arc_length(center, major, minor, start, sweep);
    if !old.is_finite() || old <= eps {
        return Err(refusal(
            "lengthen.degenerate",
            "the LENGTHEN target ELLIPSE has no measurable length",
        ));
    }
    let end_angle = start + sweep;
    let start_point = ellipse_point(center, major, minor, start);
    let end_point = ellipse_point(center, major, minor, end_angle);
    let move_start =
        pick.is_some_and(|point| distance(point, start_point) < distance(point, end_point));
    let (new_start, new_sweep) = match settings.mode {
        LengthenMode::Dynamic => {
            return Err(refusal(
                "lengthen.invalid_payload",
                "the dynamic LENGTHEN mode is resolved from its points, not a scalar length",
            ))
        }
        LengthenMode::Total => {
            let angle = settings.value.to_radians();
            if !angle.is_finite() {
                return Err(refusal(
                    "lengthen.invalid_value",
                    format!("LENGTHEN Total angle is not finite: {angle} rad"),
                ));
            }
            let new_sweep = sweep.signum() * angle.abs();
            if new_sweep.abs() > std::f64::consts::TAU + 1e-9 {
                return Err(refusal(
                    "lengthen.sweep_overflow",
                    format!(
                        "the LENGTHEN result sweeps {:.9} rad, more than one full turn ({}); \
                         it is refused rather than wrapped",
                        new_sweep.abs(),
                        std::f64::consts::TAU
                    ),
                ));
            }
            if move_start {
                (end_angle - new_sweep, new_sweep)
            } else {
                (start, new_sweep)
            }
        }
        _ => {
            let new = new_line_length(old, settings)?;
            // Measure from the kept end in the sweep's own direction, up to one
            // full turn: an ellipse's arc length depends on where the span starts.
            let keep_angle = if move_start { end_angle } else { start };
            let direction = if move_start {
                -sweep.signum()
            } else {
                sweep.signum()
            };
            let point_at = |u: f64| {
                ellipse_point(
                    center,
                    major,
                    minor,
                    keep_angle + direction * std::f64::consts::TAU * u,
                )
            };
            let table = ArcLengthTable::sample(&point_at);
            let perimeter = table.total();
            if !perimeter.is_finite() || perimeter <= eps {
                return Err(refusal(
                    "lengthen.degenerate",
                    "the LENGTHEN target ELLIPSE has no measurable length",
                ));
            }
            if new > perimeter + 1e-9 {
                return Err(refusal(
                    "lengthen.sweep_overflow",
                    format!(
                        "the LENGTHEN result needs {new} of an ellipse whose full perimeter is \
                         {perimeter}; it would sweep more than one full turn"
                    ),
                ));
            }
            let span = table.parameter_at(new) * std::f64::consts::TAU;
            let new_sweep = sweep.signum() * span;
            if move_start {
                (end_angle - new_sweep, new_sweep)
            } else {
                (start, new_sweep)
            }
        }
    };
    if !new_sweep.is_finite() || new_sweep.abs() <= eps {
        return Err(refusal(
            "lengthen.non_positive",
            "the LENGTHEN result would have no sweep",
        ));
    }
    if new_sweep.abs() > std::f64::consts::TAU + 1e-9 {
        return Err(refusal(
            "lengthen.sweep_overflow",
            format!(
                "the LENGTHEN result sweeps {:.9} rad, more than one full turn ({}); it is \
                 refused rather than wrapped",
                new_sweep.abs(),
                std::f64::consts::TAU
            ),
        ));
    }
    let new = ellipse_arc_length(center, major, minor, new_start, new_sweep);
    let changed = (new_start - start).abs() > 1e-12 || (new_sweep - sweep).abs() > 1e-12;
    Ok((
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start: new_start,
            sweep: new_sweep,
        },
        old,
        new,
        changed,
    ))
}

/// The ellipse's major-axis vector and its minor-axis vector (`a · ratio`).
fn ellipse_frame(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
) -> CadResult<(Point3, Point3)> {
    let eps = cut_tolerance();
    let major_len = cad_geometry::length(major_axis);
    let normal_len = cad_geometry::length(normal);
    if !major_len.is_finite()
        || major_len <= eps
        || !normal_len.is_finite()
        || normal_len <= eps
        || !ratio.is_finite()
        || ratio <= eps
        || !is_finite_point(center)
    {
        return Err(refusal(
            "lengthen.degenerate",
            "the LENGTHEN target ELLIPSE has no measurable length",
        ));
    }
    let unit_major = cad_geometry::scale(major_axis, 1.0 / major_len);
    let cross = cad_geometry::cross(normal, unit_major);
    let cross_len = cad_geometry::length(cross);
    if !cross_len.is_finite() || cross_len <= eps {
        return Err(refusal(
            "lengthen.degenerate",
            "the LENGTHEN target ELLIPSE has a degenerate minor axis",
        ));
    }
    let unit_minor = cad_geometry::scale(cross, 1.0 / cross_len);
    let minor = cad_geometry::scale(unit_minor, major_len * ratio);
    Ok((major_axis, minor))
}

/// The world point at ellipse parameter `angle`.
fn ellipse_point(center: Point3, major: Point3, minor: Point3, angle: f64) -> Point3 {
    Point3 {
        x: center.x + angle.cos() * major.x + angle.sin() * minor.x,
        y: center.y + angle.cos() * major.y + angle.sin() * minor.y,
        z: center.z + angle.cos() * major.z + angle.sin() * minor.z,
    }
}

/// The sampled arc length of an elliptical arc.
fn ellipse_arc_length(center: Point3, major: Point3, minor: Point3, start: f64, sweep: f64) -> f64 {
    let point_at = |u: f64| ellipse_point(center, major, minor, start + sweep * u);
    ArcLengthTable::sample(&point_at).total()
}

/// `LENGTHEN` on an open spline: trimming uses the exact sub-curve helper;
/// extending a NURBS curve has no continuation semantics and is refused.
pub(super) fn lengthen_spline(
    geometry: &SemanticGeometry,
    settings: &LengthenSettings,
    pick: Option<Point3>,
) -> CadResult<(SemanticGeometry, f64, f64, bool)> {
    let eps = cut_tolerance();
    let curve = TargetCurve::new(
        geometry,
        "LENGTHEN",
        "LINE, ARC, CIRCLE, ELLIPSE, SPLINE or LWPOLYLINE",
    )?;
    let old = curve.length();
    if !old.is_finite() || old <= eps {
        return Err(refusal(
            "lengthen.degenerate",
            "the LENGTHEN target SPLINE has no measurable length",
        ));
    }
    if settings.mode == LengthenMode::Dynamic {
        return Err(refusal(
            "lengthen.invalid_payload",
            "the dynamic LENGTHEN mode is resolved from its points, not a scalar length",
        ));
    }
    let new = new_line_length(old, settings)?;
    if new > old + eps {
        return Err(refusal(
            "lengthen.spline_extend_unsupported",
            "LENGTHEN cannot extend a SPLINE: a NURBS curve is defined by its knot vector, so \
             moving an end would change its parameterisation and no continuation policy exists; \
             shortening the spline is supported",
        ));
    }
    if (new - old).abs() <= eps {
        return Ok((geometry.clone(), old, old, false));
    }
    let start_point = curve.point_at(0.0);
    let end_point = curve.point_at(1.0);
    let move_start =
        pick.is_some_and(|point| distance(point, start_point) < distance(point, end_point));
    let (u0, span) = {
        // Invert the arc length (never the raw knot parameter) to find the
        // parameter span that keeps `new` of the curve.
        let table = ArcLengthTable::sample(&|u: f64| curve.point_at(u));
        if !table.total().is_finite() || table.total() <= eps {
            return Err(refusal(
                "lengthen.degenerate",
                "the LENGTHEN target SPLINE has no measurable length",
            ));
        }
        if move_start {
            let u0 = table.parameter_at(old - new);
            (u0, 1.0 - u0)
        } else {
            (0.0, table.parameter_at(new))
        }
    };
    let geometry = curve
        .with_span(u0, span)
        .map_err(|reason| refusal("lengthen.degenerate", reason))?;
    Ok((geometry, old, new, true))
}
