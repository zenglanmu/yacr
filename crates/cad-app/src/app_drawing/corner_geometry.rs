//! Pure geometry for the two-object CHAMFER/FILLET planners.
//!
//! Declared from [`super`] with an explicit `#[path]` so the shared
//! `app_drawing/mod.rs` (owned by other lanes) needs no edit. Everything here is
//! pure: no database, no session, no history — the command layer resolves the
//! picks, calls one planner and commits the returned plan in one transaction.
//!
//! The contracts (distance order, angle derivation, the parallel auto-radius
//! derivation, trim-off semantics and every refusal code) are documented on the
//! command module [`super`] and repeated next to each construction.

use super::super::helpers::{self, distance, normalize_signed};
use super::super::*;
use super::corner_curve::{plan_fillet_curve_curve, positive_z_normal, RoundCurve};
use super::corner_ellipse::{plan_fillet_with_ellipse, EllipseCurve};
use super::corner_line_curve::{plan_fillet_line_curve, PlanArgs};
use cad_geometry::{cross, infinite_line_intersection, length, normalize, scale, LineIntersection};

/// Map the shared two-line planner's result onto this module's plan type.
fn from_edit(edit: helpers::TwoLineEdit) -> TwoLinePlan {
    let line = |(start, end)| GeometryUpdate::Line { start, end };
    TwoLinePlan {
        first: Some(line(edit.first)),
        second: Some(line(edit.second)),
        inserted: edit.inserted,
    }
}

/// The shared [`helpers::TwoLineInput`] view of this module's input.
fn helper_input(input: &TwoLineInput) -> helpers::TwoLineInput {
    helpers::TwoLineInput {
        first: input.first,
        pick_first: input.pick_first,
        second: input.second,
        pick_second: input.pick_second,
    }
}

/// The CHAMFER construction a two-line invocation uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ChamferSpec {
    /// `A` on the first object, `B` on the second (selection order).
    Distances { a: f64, b: f64 },
    /// The distance from the first object and the chamfer angle from it.
    Angle { distance: f64, angle_radians: f64 },
}

/// The one geometry update a two-line edit applies to a picked entity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum GeometryUpdate {
    Line {
        start: Point3,
        end: Point3,
    },
    /// Rewrite an ARC's start angle and sweep (the trimmed pick side).
    Arc {
        start: f64,
        sweep: f64,
    },
    /// Rewrite an ELLIPSE's parametric start angle and sweep (the trimmed pick
    /// side); `center`/`normal`/`major_axis`/`ratio` are preserved.
    Ellipse {
        start: f64,
        sweep: f64,
    },
}

/// The resolved edit: optional updates for the two picks plus the optional new
/// entity (the chamfer LINE or the fillet ARC).
pub(super) struct TwoLinePlan {
    pub first: Option<GeometryUpdate>,
    pub second: Option<GeometryUpdate>,
    pub inserted: Option<SemanticGeometry>,
}

/// The geometric kind a FILLET pick resolved to.
#[derive(Debug, Clone, Copy)]
pub(super) enum Curve {
    Line {
        start: Point3,
        end: Point3,
    },
    Circle {
        center: Point3,
        radius: f64,
    },
    Arc {
        center: Point3,
        radius: f64,
        start: f64,
        sweep: f64,
    },
    Ellipse(EllipseCurve),
}

impl Curve {
    fn label(self) -> &'static str {
        match self {
            Self::Line { .. } => "LINE",
            Self::Circle { .. } => "CIRCLE",
            Self::Arc { .. } => "ARC",
            Self::Ellipse(_) => "ELLIPSE",
        }
    }
}

/// The corner of two infinite lines and the pick-side frame.
struct Frame {
    corner: Point3,
    dir1: (f64, f64),
    dir2: (f64, f64),
    keep1: Point3,
    keep1_is_start: bool,
    reach1: f64,
    keep2: Point3,
    keep2_is_start: bool,
    reach2: f64,
}

/// Which command a two-line plan serves (labels + diagnostic text).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum TwoLineCommandKind {
    Chamfer { spec: ChamferSpec, trim: bool },
    Fillet { radius: f64, trim: bool },
}

impl TwoLineCommandKind {
    pub(super) fn labels(self) -> (&'static str, &'static str) {
        match self {
            Self::Chamfer { .. } => ("CHAMFER", "chamfer lines"),
            Self::Fillet { .. } => ("FILLET", "fillet lines"),
        }
    }

    pub(super) fn diagnostic(self) -> String {
        match self {
            Self::Chamfer { spec, trim } => format!(
                "CHAMFER: {}; trim={trim} (a LINE joins the objects unless the result is \
                 intersect-only)",
                match spec {
                    ChamferSpec::Distances { a, b } => format!("distance A={a}, B={b}"),
                    ChamferSpec::Angle {
                        distance,
                        angle_radians,
                    } => format!(
                        "distance={distance}, angle={} deg",
                        angle_radians.to_degrees()
                    ),
                }
            ),
            Self::Fillet { radius, trim } => format!(
                "FILLET: radius={radius}; trim={trim} (a tangent ARC is inserted unless R=0)"
            ),
        }
    }
}

/// The resolved geometry of the two picked straight lines.
pub(super) struct TwoLineInput {
    pub first: (Point3, Point3),
    pub pick_first: Point3,
    pub second: (Point3, Point3),
    pub pick_second: Point3,
}

pub(super) fn pt(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// A finite, non-degenerate unit XY direction.
pub(super) fn unit_xy(
    from: Point3,
    to: Point3,
    tol: f64,
    command: &str,
) -> Result<(f64, f64), String> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = dx.hypot(dy);
    if !len.is_finite() || len <= tol {
        return Err(format!(
            "{command}.degenerate_line: the picked object has no direction"
        ));
    }
    Ok((dx / len, dy / len))
}

fn point_from(corner: Point3, dir: (f64, f64), dist: f64) -> Point3 {
    pt(corner.x + dir.0 * dist, corner.y + dir.1 * dist, corner.z)
}

fn retarget(keep_is_start: bool, keep: Point3, target: Point3) -> GeometryUpdate {
    GeometryUpdate::Line {
        start: if keep_is_start { keep } else { target },
        end: if keep_is_start { target } else { keep },
    }
}

/// Resolve the intersection corner and the pick-side arms of two straight lines.
fn corner_frame(input: &TwoLineInput, tol: f64, command: &str) -> Result<Frame, String> {
    let TwoLineInput {
        first: (a1, a2),
        pick_first: p1,
        second: (b1, b2),
        pick_second: p2,
    } = *input;
    if !is_finite_point(a1)
        || !is_finite_point(a2)
        || !is_finite_point(p1)
        || !is_finite_point(b1)
        || !is_finite_point(b2)
        || !is_finite_point(p2)
    {
        return Err(format!("{command}.non_finite: picks must be finite"));
    }
    let z_tol = 1e-9 * (1.0 + a1.z.abs() + a2.z.abs() + b1.z.abs() + b2.z.abs());
    if (a2.z - a1.z).abs() > z_tol || (b1.z - a1.z).abs() > z_tol || (b2.z - a1.z).abs() > z_tol {
        return Err(format!(
            "{command}.not_coplanar: needs two coplanar (+Z) straight objects"
        ));
    }
    let (u1x, u1y) = unit_xy(a1, a2, tol, command)?;
    let (u2x, u2y) = unit_xy(b1, b2, tol, command)?;
    let corner = match infinite_line_intersection(a1, a2, b1, b2, tol) {
        LineIntersection::Point(corner) => corner,
        LineIntersection::Parallel => {
            return Err(format!(
                "{command}.lines_parallel: the two lines are parallel and meet no unique corner"
            ));
        }
        LineIntersection::Collinear => {
            return Err(format!(
                "{command}.lines_collinear: the two lines are collinear and have no unique corner"
            ));
        }
    };
    let s1 = (p1.x - corner.x) * u1x + (p1.y - corner.y) * u1y;
    if !s1.is_finite() || s1.abs() <= tol {
        return Err(format!(
            "{command}.pick_at_corner: the pick must be away from the corner"
        ));
    }
    let dir1 = if s1 > 0.0 { (u1x, u1y) } else { (-u1x, -u1y) };
    let s2 = (p2.x - corner.x) * u2x + (p2.y - corner.y) * u2y;
    if !s2.is_finite() || s2.abs() <= tol {
        return Err(format!(
            "{command}.pick_at_corner: the pick must be away from the corner"
        ));
    }
    let dir2 = if s2 > 0.0 { (u2x, u2y) } else { (-u2x, -u2y) };
    let along1_start = (a1.x - corner.x) * dir1.0 + (a1.y - corner.y) * dir1.1;
    let along1_end = (a2.x - corner.x) * dir1.0 + (a2.y - corner.y) * dir1.1;
    let keep1_is_start = along1_start >= along1_end;
    let (keep1, reach1) = if keep1_is_start {
        (a1, along1_start)
    } else {
        (a2, along1_end)
    };
    let along2_start = (b1.x - corner.x) * dir2.0 + (b1.y - corner.y) * dir2.1;
    let along2_end = (b2.x - corner.x) * dir2.0 + (b2.y - corner.y) * dir2.1;
    let keep2_is_start = along2_start >= along2_end;
    let (keep2, reach2) = if keep2_is_start {
        (b1, along2_start)
    } else {
        (b2, along2_end)
    };
    Ok(Frame {
        corner,
        dir1,
        dir2,
        keep1,
        keep1_is_start,
        reach1,
        keep2,
        keep2_is_start,
        reach2,
    })
}

/// The interior angle between the two pick-side arms (`∈ (0, π)`).
fn frame_angle(frame: &Frame) -> f64 {
    (frame.dir1.0 * frame.dir2.0 + frame.dir1.1 * frame.dir2.1)
        .clamp(-1.0, 1.0)
        .acos()
}

/// Plan CHAMFER of two straight lines.
pub(super) fn plan_chamfer_two_line(
    input: &TwoLineInput,
    spec: ChamferSpec,
    trim: bool,
    tol: f64,
) -> Result<TwoLinePlan, String> {
    let frame = corner_frame(input, tol, "CHAMFER")?;
    if trim {
        if let ChamferSpec::Distances { a, b } = spec {
            // The ordinary distance/distance trim-on form is the shared ADR 0016
            // planner; the angle and No-Trim forms below are this module's.
            return helpers::plan_chamfer(&helper_input(input), a, b, tol)
                .map(from_edit)
                .map_err(|message| format!("CHAMFER.{message}"));
        }
    }
    let (d1, d2) = match spec {
        ChamferSpec::Distances { a, b } => {
            if !a.is_finite() || a < 0.0 || !b.is_finite() || b < 0.0 {
                return Err(
                    "CHAMFER.distance_negative: the distances must be finite and non-negative"
                        .into(),
                );
            }
            (a, b)
        }
        ChamferSpec::Angle {
            distance,
            angle_radians,
        } => {
            if !distance.is_finite() || distance < 0.0 {
                return Err(
                    "CHAMFER.distance_negative: the distance must be finite and non-negative"
                        .into(),
                );
            }
            if !angle_radians.is_finite() || angle_radians < 0.0 {
                return Err(
                    "CHAMFER.angle_out_of_range: the angle must be finite and non-negative".into(),
                );
            }
            if angle_radians == 0.0 {
                if distance > tol {
                    // A zero angle with a positive length would lay the "chamfer"
                    // along the first object (second distance 0), i.e. duplicate
                    // the picked edge instead of cutting a distinct one. Not
                    // documented by AutoCAD, so this build refuses by name.
                    return Err(
                        "CHAMFER.angle_zero: a zero angle with a positive distance would lie \
                         along the first object and cut no distinct edge"
                            .into(),
                    );
                }
                (0.0, 0.0)
            } else {
                // The angle is measured from the first object; the chamfer line
                // from `t1` at angle α meets the second arm at
                // `d₂ = d·sin α / sin(φ + α)`.
                let phi = frame_angle(&frame);
                let denominator = (phi + angle_radians).sin();
                if !denominator.is_finite() || denominator <= 1e-9 {
                    return Err(format!(
                        "CHAMFER.angle_out_of_range: the angle {} deg leaves no wedge on the \
                         second object",
                        angle_radians.to_degrees()
                    ));
                }
                let d2 = distance * angle_radians.sin() / denominator;
                if !d2.is_finite() || d2 < 0.0 {
                    return Err(
                        "CHAMFER.angle_out_of_range: the angle cannot define the second distance"
                            .into(),
                    );
                }
                (distance, d2)
            }
        }
    };
    if d1 <= tol && d2 <= tol {
        // Both zero: extend/trim the two objects to the intersection, create nothing.
        if !trim {
            return Err(
                "CHAMFER.trim_off_no_chamfer: with trim off the intersect-only form creates \
                 nothing"
                    .into(),
            );
        }
        if frame.reach1 <= tol || frame.reach2 <= tol {
            return Err(
                "CHAMFER.degenerate_line: a picked line collapses when moved to the corner".into(),
            );
        }
        return Ok(TwoLinePlan {
            first: Some(retarget(frame.keep1_is_start, frame.keep1, frame.corner)),
            second: Some(retarget(frame.keep2_is_start, frame.keep2, frame.corner)),
            inserted: None,
        });
    }
    if trim && (d1 >= frame.reach1 - tol || d2 >= frame.reach2 - tol) {
        return Err("CHAMFER.distance_too_long: a distance collapses the kept segment".into());
    }
    let t1 = point_from(frame.corner, frame.dir1, d1);
    let t2 = point_from(frame.corner, frame.dir2, d2);
    if distance(t1, t2) <= tol {
        return Err(
            "CHAMFER.degenerate_join: the two chamfer points coincide; nothing to join".into(),
        );
    }
    let (first, second) = if trim {
        (
            Some(retarget(frame.keep1_is_start, frame.keep1, t1)),
            Some(retarget(frame.keep2_is_start, frame.keep2, t2)),
        )
    } else {
        (None, None)
    };
    Ok(TwoLinePlan {
        first,
        second,
        inserted: Some(SemanticGeometry::Line { start: t1, end: t2 }),
    })
}

/// Plan FILLET of two resolved objects.
pub(super) fn plan_fillet_two_line(
    first: Curve,
    pick_first: Point3,
    second: Curve,
    pick_second: Point3,
    radius: f64,
    trim: bool,
    tol: f64,
) -> Result<TwoLinePlan, String> {
    if !radius.is_finite() || radius < 0.0 {
        return Err("FILLET.radius_negative: the radius must be finite and non-negative".into());
    }
    // ELLIPSE participants use the numeric one-parameter tangent search; every
    // other pair keeps the exact analytic construction below.
    if matches!(first, Curve::Ellipse(_)) || matches!(second, Curve::Ellipse(_)) {
        return plan_fillet_with_ellipse(first, pick_first, second, pick_second, radius, trim, tol);
    }
    match (first, second) {
        (Curve::Line { start: a1, end: a2 }, Curve::Line { start: b1, end: b2 }) => {
            plan_fillet_lines(
                &TwoLineInput {
                    first: (a1, a2),
                    pick_first,
                    second: (b1, b2),
                    pick_second,
                },
                radius,
                trim,
                tol,
            )
        }
        (Curve::Line { start, end }, Curve::Circle { center, radius: r }) => {
            plan_fillet_line_curve(PlanArgs {
                line: (start, end),
                line_pick: pick_first,
                center,
                circle_radius: r,
                arc: None,
                curve_pick: pick_second,
                radius,
                trim,
                tol,
                line_is_first: true,
            })
        }
        (Curve::Circle { center, radius: r }, Curve::Line { start, end }) => {
            plan_fillet_line_curve(PlanArgs {
                line: (start, end),
                line_pick: pick_second,
                center,
                circle_radius: r,
                arc: None,
                curve_pick: pick_first,
                radius,
                trim,
                tol,
                line_is_first: false,
            })
        }
        (
            Curve::Line { start, end },
            Curve::Arc {
                center,
                radius: r,
                start: arc_start,
                sweep,
            },
        ) => plan_fillet_line_curve(PlanArgs {
            line: (start, end),
            line_pick: pick_first,
            center,
            circle_radius: r,
            arc: Some((arc_start, sweep)),
            curve_pick: pick_second,
            radius,
            trim,
            tol,
            line_is_first: true,
        }),
        (
            Curve::Arc {
                center,
                radius: r,
                start: arc_start,
                sweep,
            },
            Curve::Line { start, end },
        ) => plan_fillet_line_curve(PlanArgs {
            line: (start, end),
            line_pick: pick_second,
            center,
            circle_radius: r,
            arc: Some((arc_start, sweep)),
            curve_pick: pick_first,
            radius,
            trim,
            tol,
            line_is_first: false,
        }),
        (first, second) => match (RoundCurve::resolve(first), RoundCurve::resolve(second)) {
            (Some(first), Some(second)) => {
                plan_fillet_curve_curve(first, pick_first, second, pick_second, radius, trim, tol)
            }
            _ => Err(format!(
                "FILLET.unsupported_pair: {}/{} fillets are not implemented (line↔line, \
                 line↔circle, line↔arc, circle↔circle, circle↔arc and arc↔arc are)",
                first.label(),
                second.label()
            )),
        },
    }
}

/// Plan FILLET of two straight lines, including the parallel auto-radius rule.
fn plan_fillet_lines(
    input: &TwoLineInput,
    radius: f64,
    trim: bool,
    tol: f64,
) -> Result<TwoLinePlan, String> {
    match infinite_line_intersection(
        input.first.0,
        input.first.1,
        input.second.0,
        input.second.1,
        tol,
    ) {
        LineIntersection::Point(_) => {}
        LineIntersection::Parallel => return plan_parallel_fillet(input, trim, tol),
        LineIntersection::Collinear => {
            return Err(
                "FILLET.lines_collinear: collinear lines have no corner and no gap to fillet"
                    .into(),
            );
        }
    }
    let frame = corner_frame(input, tol, "FILLET")?;
    if trim {
        // The trim-on line/line form (including R=0) is the shared ADR 0016
        // planner; the No-Trim form below needs the untouched-object rule.
        return helpers::plan_fillet(&helper_input(input), radius, tol)
            .map(from_edit)
            .map_err(|message| format!("FILLET.{message}"));
    }
    if radius == 0.0 {
        if !trim {
            return Err(
                "FILLET.trim_off_no_arc: with trim off the R=0 sharp corner creates nothing".into(),
            );
        }
        if frame.reach1 <= tol || frame.reach2 <= tol {
            return Err("FILLET.degenerate_line: a picked line collapses at the corner".into());
        }
        return Ok(TwoLinePlan {
            first: Some(retarget(frame.keep1_is_start, frame.keep1, frame.corner)),
            second: Some(retarget(frame.keep2_is_start, frame.keep2, frame.corner)),
            inserted: None,
        });
    }
    let phi = frame_angle(&frame);
    let half = phi * 0.5;
    let (sin_half, cos_half) = half.sin_cos();
    if !sin_half.is_finite() || sin_half.abs() <= 1e-12 || !cos_half.is_finite() {
        return Err("FILLET.degenerate_angle: the lines meet at no usable angle".into());
    }
    let tan_half = sin_half / cos_half;
    if !tan_half.is_finite() || tan_half.abs() <= 1e-12 {
        return Err("FILLET.degenerate_angle: the lines meet at no usable angle".into());
    }
    let tangent_distance = radius / tan_half;
    if !tangent_distance.is_finite() {
        return Err("FILLET.radius_too_large: the radius has no finite tangent distance".into());
    }
    if trim && (tangent_distance >= frame.reach1 - tol || tangent_distance >= frame.reach2 - tol) {
        return Err("FILLET.radius_too_large: the radius does not fit the picked objects".into());
    }
    let t1 = point_from(frame.corner, frame.dir1, tangent_distance);
    let t2 = point_from(frame.corner, frame.dir2, tangent_distance);
    let bisector = (frame.dir1.0 + frame.dir2.0, frame.dir1.1 + frame.dir2.1);
    let bisector_len = bisector.0.hypot(bisector.1);
    if !bisector_len.is_finite() || bisector_len <= 1e-12 {
        return Err("FILLET.degenerate_angle: no inside bisector".into());
    }
    let centre_distance = radius / sin_half;
    let centre = pt(
        frame.corner.x + bisector.0 / bisector_len * centre_distance,
        frame.corner.y + bisector.1 / bisector_len * centre_distance,
        frame.corner.z,
    );
    let start = (t1.y - centre.y).atan2(t1.x - centre.x);
    let end = (t2.y - centre.y).atan2(t2.x - centre.x);
    let sweep = normalize_signed(end - start);
    if !sweep.is_finite() || sweep.abs() <= 1e-12 {
        return Err("FILLET.degenerate_arc: the tangent arc has no sweep".into());
    }
    let (first, second) = if trim {
        (
            Some(retarget(frame.keep1_is_start, frame.keep1, t1)),
            Some(retarget(frame.keep2_is_start, frame.keep2, t2)),
        )
    } else {
        (None, None)
    };
    Ok(TwoLinePlan {
        first,
        second,
        inserted: Some(arc(centre, radius, start, sweep)),
    })
}

pub(super) fn arc(center: Point3, radius: f64, start: f64, sweep: f64) -> SemanticGeometry {
    SemanticGeometry::Arc {
        center,
        normal: pt(0.0, 0.0, 1.0),
        radius,
        start,
        sweep,
    }
}

/// Keep the endpoint of `segment` on the pick side of `target` and move the other
/// endpoint onto `target`.
pub(super) fn retarget_pick_side(
    segment: (Point3, Point3),
    pick: Point3,
    target: Point3,
    tol: f64,
    command: &str,
) -> Result<GeometryUpdate, String> {
    let (a, b) = segment;
    let dir = unit_xy(a, b, tol, command)?;
    let pick_side = (pick.x - target.x) * dir.0 + (pick.y - target.y) * dir.1;
    if !pick_side.is_finite() || pick_side.abs() <= tol {
        return Err(format!(
            "{command}.pick_at_tangent: the pick does not select a side of the tangent point"
        ));
    }
    let a_side = ((a.x - target.x) * dir.0 + (a.y - target.y) * dir.1) * pick_side.signum();
    let b_side = ((b.x - target.x) * dir.0 + (b.y - target.y) * dir.1) * pick_side.signum();
    let keep_is_start = a_side >= b_side;
    let keep = if keep_is_start { a } else { b };
    if distance(keep, target) <= tol {
        return Err(format!(
            "{command}.degenerate_line: the tangent point collapses the kept segment"
        ));
    }
    Ok(retarget(keep_is_start, keep, target))
}

/// Plan FILLET of two parallel lines (the auto-radius rule).
///
/// The effective radius is half the perpendicular gap: any circle tangent to two
/// distinct parallel lines has its centre on the midline, so its radius is the
/// midline-to-line distance. The centre's along-line position is our declared rule
/// (the average of the two pick projections); the arc is the `+Z`
/// counter-clockwise half circle from the first line's tangent point to the
/// second's.
fn plan_parallel_fillet(input: &TwoLineInput, trim: bool, tol: f64) -> Result<TwoLinePlan, String> {
    let (a1, a2) = input.first;
    let (b1, b2) = input.second;
    let (ux, uy) = unit_xy(a1, a2, tol, "FILLET")?;
    let _ = unit_xy(b1, b2, tol, "FILLET")?;
    let gap = ((b1.x - a1.x) * -uy + (b1.y - a1.y) * ux).abs();
    if !gap.is_finite() || gap <= tol {
        return Err("FILLET.lines_collinear: collinear lines have no gap to fillet".into());
    }
    let effective_radius = gap * 0.5;
    let along = (input.pick_first.x * ux
        + input.pick_first.y * uy
        + input.pick_second.x * ux
        + input.pick_second.y * uy)
        * 0.5;
    let along_point = |origin: Point3| {
        let base = origin.x * ux + origin.y * uy;
        pt(
            origin.x + ux * (along - base),
            origin.y + uy * (along - base),
            origin.z,
        )
    };
    let t1 = along_point(a1);
    let t2 = along_point(b1);
    let centre = pt((t1.x + t2.x) * 0.5, (t1.y + t2.y) * 0.5, t1.z);
    if distance(t1, t2) <= tol {
        return Err("FILLET.degenerate_arc: the parallel lines have no gap".into());
    }
    let start = (t1.y - centre.y).atan2(t1.x - centre.x);
    let (first, second) = if trim {
        (
            Some(retarget_pick_side(
                (a1, a2),
                input.pick_first,
                t1,
                tol,
                "FILLET",
            )?),
            Some(retarget_pick_side(
                (b1, b2),
                input.pick_second,
                t2,
                tol,
                "FILLET",
            )?),
        )
    } else {
        (None, None)
    };
    Ok(TwoLinePlan {
        first,
        second,
        inserted: Some(arc(centre, effective_radius, start, std::f64::consts::PI)),
    })
}

/// The along-arc distance from `start` to `point` in the sweep's direction.
pub(super) fn arc_position(center: Point3, start: f64, sweep: f64, point: Point3) -> f64 {
    let sign = sweep.signum();
    let angle = (point.y - center.y).atan2(point.x - center.x);
    let mut value = (angle - start) * sign % std::f64::consts::TAU;
    if value < 0.0 {
        value += std::f64::consts::TAU;
    }
    value
}

/// Trim an arc to the tangent point on the pick side, returning the new
/// `(start, sweep)`.
pub(super) fn trim_arc_to(
    center: Point3,
    start: f64,
    sweep: f64,
    tangent: Point3,
    pick: Point3,
    tol: f64,
) -> Result<GeometryUpdate, String> {
    let sign = sweep.signum();
    let tangent_position = arc_position(center, start, sweep, tangent);
    let pick_position = arc_position(center, start, sweep, pick);
    if !tangent_position.is_finite() || tangent_position > sweep.abs() + 1e-9 {
        return Err(
            "FILLET.tangent_outside_arc: the tangent point does not lie on the picked arc".into(),
        );
    }
    if pick_position > sweep.abs() + 1e-9 {
        return Err("FILLET.pick_outside_arc: the pick does not lie on the picked arc".into());
    }
    if tangent_position <= tol {
        return Err("FILLET.degenerate_arc: the tangent point is the arc's own start".into());
    }
    if pick_position <= tangent_position {
        // The pick is on the start→tangent portion: keep it (trim the far end).
        Ok(GeometryUpdate::Arc {
            start,
            sweep: tangent_position * sign,
        })
    } else {
        // The pick is beyond the tangent point: keep that portion instead. A
        // tangent at (or rounding onto) the arc's far end leaves no arc at all.
        let new_sweep = (sweep.abs() - tangent_position) * sign;
        if !new_sweep.is_finite() || new_sweep.abs() <= tol {
            return Err(
                "FILLET.degenerate_arc: the trim leaves a near-zero remainder of the picked arc"
                    .into(),
            );
        }
        Ok(GeometryUpdate::Arc {
            start: start + tangent_position * sign,
            sweep: new_sweep,
        })
    }
}

/// Resolve one FILLET pick to the supported curve kinds.
///
/// A CIRCLE/ARC participant must store a `+Z` normal: imported curves preserve an
/// arbitrary extrusion normal whose angles are in that frame, and this construction
/// only models `+Z`; the refusal therefore also covers the line↔circle/arc form.
pub(super) fn fillet_curve(geometry: &SemanticGeometry) -> CadResult<Curve> {
    match geometry {
        SemanticGeometry::Line { start, end } => Ok(Curve::Line {
            start: *start,
            end: *end,
        }),
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            positive_z_normal(normal, "CIRCLE")?;
            Ok(Curve::Circle {
                center: *center,
                radius: *radius,
            })
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            positive_z_normal(normal, "ARC")?;
            Ok(Curve::Arc {
                center: *center,
                radius: *radius,
                start: *start,
                sweep: *sweep,
            })
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            positive_z_normal(normal, "ELLIPSE")?;
            let major_len = length(*major_axis);
            if !is_finite_point(*center)
                || !major_len.is_finite()
                || major_len <= 1e-9
                || !ratio.is_finite()
                || ratio.abs() <= 1e-9
                || !start.is_finite()
                || !sweep.is_finite()
                || sweep.abs() <= 1e-12
                || sweep.abs() > std::f64::consts::TAU + 1e-9
            {
                return Err(CadError::Unsupported(
                    "FILLET.degenerate_ellipse: the picked ELLIPSE has no finite, storable \
                     parametric span"
                        .into(),
                ));
            }
            if major_axis.z.abs() > 1e-9 * major_len {
                return Err(CadError::Unsupported(
                    "FILLET.unsupported_plane: the ELLIPSE major axis is not in the +Z work \
                     plane"
                        .into(),
                ));
            }
            let major = scale(*major_axis, 1.0 / major_len);
            let minor = normalize(cross(*normal, major));
            if !is_finite_point(minor) || length(minor) <= 1e-9 {
                return Err(CadError::Unsupported(
                    "FILLET.degenerate_ellipse: the picked ELLIPSE has no usable minor axis".into(),
                ));
            }
            Ok(Curve::Ellipse(EllipseCurve {
                center: *center,
                major,
                minor,
                a: major_len,
                b: major_len * ratio.abs(),
                start: *start,
                sweep: *sweep,
            }))
        }
        other => Err(CadError::Unsupported(format!(
            "FILLET.unsupported_pair: {} is not a supported FILLET participant (LINE, CIRCLE, \
             ARC and ELLIPSE are; a 2D polyline is the separate FILLET POLYLINE command, and \
             SPLINE needs a control-frame edit this build does not model)",
            class_name(other)
        ))),
    }
}

/// Re-export the shared straight-line resolver (keeps one refusal string for
/// both corner commands).
pub(super) use super::super::helpers::straight_line;
