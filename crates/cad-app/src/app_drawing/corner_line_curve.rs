//! Pure geometry for the FILLET line ↔ circle/arc form.
//!
//! Declared with an explicit `#[path]` beside the other corner planners so no
//! file outgrows the 1000-line ceiling. Pure: no database, session or history.
//! The command layer resolves the picks, calls the planner and commits the
//! returned plan in one transaction.
//!
//! The arc centre is the intersection of the line offset by `R` (on the circle's
//! side) with the circle of radius `r + R` about the circle centre; the tangent
//! points are the perpendicular foot on the line and the point at ratio
//! `r / (r + R)` toward the circle centre. A circle is never trimmed; an arc is
//! trimmed at its tangent point on the pick side. `R = 0` is not this form's job:
//! it is the sharp corner, planned by [`super::corner_sharp`].

use super::super::helpers::{distance, normalize_signed};
use super::super::*;
use super::corner_geometry::{arc, pt, retarget_pick_side, trim_arc_to, unit_xy, TwoLinePlan};

/// The argument bundle of one line↔circle/arc fillet plan.
pub(super) struct PlanArgs {
    pub(super) line: (Point3, Point3),
    pub(super) line_pick: Point3,
    pub(super) center: Point3,
    pub(super) circle_radius: f64,
    /// `Some((start, sweep))` when the curve target is a finite ARC.
    pub(super) arc: Option<(f64, f64)>,
    pub(super) curve_pick: Point3,
    pub(super) radius: f64,
    pub(super) trim: bool,
    pub(super) tol: f64,
    /// Whether the retargeted line is the caller's first pick.
    pub(super) line_is_first: bool,
}

/// Plan FILLET of one line and one circle (or arc).
///
/// The arc centre is the intersection of the line offset by `R` (on the circle's
/// side) with the circle of radius `r + R` about the circle centre; the tangent
/// points are the perpendicular foot on the line and the point at ratio
/// `r / (r + R)` toward the circle centre. A circle is never trimmed.
pub(super) fn plan_fillet_line_curve(args: PlanArgs) -> Result<TwoLinePlan, String> {
    let PlanArgs {
        line: (a, b),
        line_pick,
        center,
        circle_radius,
        arc: arc_target,
        curve_pick,
        radius,
        trim,
        tol,
        line_is_first,
    } = args;
    if !circle_radius.is_finite() || circle_radius <= tol {
        return Err("FILLET.degenerate_circle: the picked circle has no radius".into());
    }
    let z_tol = 1e-9 * (1.0 + a.z.abs() + b.z.abs() + center.z.abs());
    if (a.z - center.z).abs() > z_tol || (b.z - center.z).abs() > z_tol {
        return Err("FILLET.not_coplanar: the line and the circle are not coplanar (+Z)".into());
    }
    let (ux, uy) = unit_xy(a, b, tol, "FILLET")?;
    let (nx, ny) = (-uy, ux);
    // `R = 0` is the sharp corner and is planned by [`super::corner_sharp`] before
    // this form is reached (it trims **both** the line and the picked arc, and
    // leaves a circle whole); a zero radius here is a caller bug.
    if radius == 0.0 {
        return Err(
            "FILLET.radius_zero: the line↔circle/arc tangency form needs a positive radius \
             (R=0 is planned by the sharp-corner form, corner_sharp)"
                .into(),
        );
    }
    // The fillet lies on the side of the line the circle is on; when the line
    // crosses the circle the pick side decides (declared).
    let centre_side = (center.x - a.x) * nx + (center.y - a.y) * ny;
    let side = if centre_side.abs() > circle_radius {
        centre_side.signum()
    } else {
        let pick_side = (line_pick.x - a.x) * nx + (line_pick.y - a.y) * ny;
        if pick_side.abs() <= tol {
            return Err(
                "FILLET.degenerate_side: the line pick does not select a side of the circle".into(),
            );
        }
        pick_side.signum()
    };
    let reach = circle_radius + radius;
    let offset = pt(a.x + nx * side * radius, a.y + ny * side * radius, a.z);
    let wx = offset.x - center.x;
    let wy = offset.y - center.y;
    let half_b = wx * ux + wy * uy;
    let discriminant = half_b * half_b - (wx * wx + wy * wy - reach * reach);
    if !discriminant.is_finite() || discriminant < 0.0 {
        return Err(
            "FILLET.no_tangent_arc: no arc of this radius is tangent to both objects".into(),
        );
    }
    let root = discriminant.sqrt();
    let mut best: Option<(f64, Point3, Point3, Point3, f64, f64)> = None;
    for parameter in [-half_b - root, -half_b + root] {
        let arc_centre = pt(
            offset.x + ux * parameter,
            offset.y + uy * parameter,
            offset.z,
        );
        let t_line = pt(
            arc_centre.x - nx * side * radius,
            arc_centre.y - ny * side * radius,
            a.z,
        );
        let ratio = circle_radius / reach;
        let t_curve = pt(
            center.x + (arc_centre.x - center.x) * ratio,
            center.y + (arc_centre.y - center.y) * ratio,
            center.z,
        );
        let start = (t_line.y - arc_centre.y).atan2(t_line.x - arc_centre.x);
        let end = (t_curve.y - arc_centre.y).atan2(t_curve.x - arc_centre.x);
        let sweep = normalize_signed(end - start);
        if !sweep.is_finite() || sweep.abs() <= 1e-12 {
            continue;
        }
        let score = distance(t_line, line_pick) + distance(t_curve, curve_pick);
        if best
            .as_ref()
            .is_none_or(|(best_score, ..)| score < *best_score)
        {
            best = Some((score, arc_centre, t_line, t_curve, start, sweep));
        }
    }
    let Some((_, arc_centre, t_line, t_curve, start, sweep)) = best else {
        return Err("FILLET.no_tangent_arc: no usable tangent arc was found".into());
    };
    let line_update = if trim {
        Some(retarget_pick_side(
            (a, b),
            line_pick,
            t_line,
            tol,
            "FILLET",
        )?)
    } else {
        None
    };
    let arc_update = match (arc_target, trim) {
        (Some((arc_start, arc_sweep)), true) => Some(trim_arc_to(
            center, arc_start, arc_sweep, t_curve, curve_pick, tol,
        )?),
        _ => None,
    };
    let inserted = arc(arc_centre, radius, start, sweep);
    Ok(match line_is_first {
        true => TwoLinePlan {
            first: line_update,
            second: arc_update,
            inserted: Some(inserted),
        },
        false => TwoLinePlan {
            first: arc_update,
            second: line_update,
            inserted: Some(inserted),
        },
    })
}
