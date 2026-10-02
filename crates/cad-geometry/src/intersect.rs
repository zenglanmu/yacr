//! intersect module.

use super::*;

/// Exact or near-exact intersection of two 3D segments.
///
/// Works for segments in any plane (the old version projected onto XY and only
/// accepted lines that crossed there). `tol` is an absolute world tolerance.
pub(crate) fn segment_segment_intersection_3d(
    a1: Point3,
    a2: Point3,
    b1: Point3,
    b2: Point3,
    tol: f64,
) -> Option<Point3> {
    let d1 = sub(a2, a1);
    let d2 = sub(b2, b1);
    // Closest-point formula from Ericson, *Real-Time Collision Detection*:
    // r must go from the second segment's start to the first's.
    let r = sub(a1, b1);
    let aa = dot(d1, d1);
    let ee = dot(d2, d2);
    let bb = dot(d1, d2);
    let cc = dot(d1, r);
    let ff = dot(d2, r);
    let denom = aa * ee - bb * bb;
    let len_scale = (aa * ee).sqrt().max(1.0);
    let (s, t) = if denom.abs() > 1e-15 * len_scale {
        ((bb * ff - cc * ee) / denom, (aa * ff - bb * cc) / denom)
    } else {
        // Parallel: pick the projection of b1 onto a.
        let s = if aa > 0.0 { cc / aa } else { 0.0 };
        let t = if ee > 0.0 { ff / ee } else { 0.0 };
        (s, t)
    };
    let slack = if len_scale > 0.0 {
        (tol.max(1e-12) / len_scale.sqrt()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if s < -slack || s > 1.0 + slack || t < -slack || t > 1.0 + slack {
        return None;
    }
    let p = add(a1, scale(d1, s.clamp(0.0, 1.0)));
    let q = add(b1, scale(d2, t.clamp(0.0, 1.0)));
    if distance(p, q) <= tol.max(1e-12) {
        Some(scale(add(p, q), 0.5))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Analytic curve/curve intersection (audit B23/F06).
//
// Measurement and snapping must not depend on the display tessellation. Where
// both operands are conic primitives the intersection is solved on the analytic
// curve; anything else falls back to adaptive sampling at the world-space
// predicate tolerance.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub(crate) enum Conic {
    Segment {
        a: Point3,
        b: Point3,
    },
    Circle {
        center: Point3,
        normal: Point3,
        radius: f64,
    },
    Arc {
        center: Point3,
        normal: Point3,
        radius: f64,
        start: f64,
        sweep: f64,
    },
    Ellipse {
        center: Point3,
        normal: Point3,
        major_axis: Point3,
        ratio: f64,
        start: f64,
        sweep: f64,
    },
}

pub(crate) fn conic_of(geometry: &SemanticGeometry) -> Option<Conic> {
    match geometry {
        SemanticGeometry::Line { start, end } => Some(Conic::Segment { a: *start, b: *end }),
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => Some(Conic::Circle {
            center: *center,
            normal: *normal,
            radius: radius.abs(),
        }),
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => Some(Conic::Arc {
            center: *center,
            normal: *normal,
            radius: radius.abs(),
            start: *start,
            sweep: *sweep,
        }),
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => Some(Conic::Ellipse {
            center: *center,
            normal: *normal,
            major_axis: *major_axis,
            ratio: ratio.abs(),
            start: *start,
            sweep: *sweep,
        }),
        _ => None,
    }
}

/// Whether a parameter angle lies on the (possibly signed) arc.
pub(crate) fn angle_on_arc(angle: f64, start: f64, sweep: f64, ang_tol: f64) -> bool {
    let tau = std::f64::consts::TAU;
    if sweep.abs() >= tau - 1e-9 {
        return true;
    }
    let norm = |mut x: f64| {
        x %= tau;
        if x < 0.0 {
            x += tau;
        }
        x
    };
    let s = norm(start);
    let a = norm(angle);
    if sweep >= 0.0 {
        norm(a - s) <= sweep + ang_tol
    } else {
        norm(s - a) <= -sweep + ang_tol
    }
}

pub(crate) fn push_unique(out: &mut Vec<Point3>, p: Point3, tol: f64) {
    if !out.iter().any(|q| distance(*q, p) <= tol) {
        out.push(p);
    }
}

pub(crate) fn segment_circle_points(
    a: Point3,
    b: Point3,
    center: Point3,
    normal: Point3,
    radius: f64,
    tol: f64,
) -> Vec<Point3> {
    let mut out = Vec::new();
    let n = normalize(normal);
    let d = sub(b, a);
    let dn = dot(d, n);
    let ac = sub(a, center);
    let a_off = dot(ac, n);
    if dn.abs() > tol {
        // The segment pierces the circle's plane at a single parameter.
        let t = -a_off / dn;
        if t >= -tol && t <= 1.0 + tol {
            let p = add(a, scale(d, t.clamp(0.0, 1.0)));
            if (distance(p, center) - radius).abs() <= tol {
                push_unique(&mut out, p, tol);
            }
        }
        return out;
    }
    // Parallel to the plane: only an in-plane segment can meet the circle.
    if a_off.abs() > tol {
        return out;
    }
    let qa = dot(d, d);
    if qa <= tol * tol {
        if (distance(a, center) - radius).abs() <= tol {
            push_unique(&mut out, a, tol);
        }
        return out;
    }
    let qb = 2.0 * dot(ac, d);
    let qc = dot(ac, ac) - radius * radius;
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return out;
    }
    let sq = disc.sqrt();
    for t in [(-qb - sq) / (2.0 * qa), (-qb + sq) / (2.0 * qa)] {
        if (-tol..=1.0 + tol).contains(&t) {
            push_unique(&mut out, add(a, scale(d, t.clamp(0.0, 1.0))), tol);
        }
    }
    out
}

pub(crate) fn circle_circle_points(
    c1: Point3,
    n1: Point3,
    r1: f64,
    c2: Point3,
    n2: Point3,
    r2: f64,
    tol: f64,
) -> Option<Vec<Point3>> {
    let u1 = normalize(n1);
    let u2 = normalize(n2);
    if length(cross(u1, u2)) > 1e-9 {
        // Non-parallel planes: not handled analytically (caller samples).
        return None;
    }
    if dot(sub(c2, c1), u1).abs() > tol {
        return None;
    }
    let mut out = Vec::new();
    let d_vec = sub(c2, c1);
    let d = length(d_vec);
    if d <= tol {
        // Concentric: either the same circle (infinitely many points) or none.
        return Some(out);
    }
    if d > r1 + r2 + tol || d < (r1 - r2).abs() - tol {
        return Some(out);
    }
    let aa = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let hh = r1 * r1 - aa * aa;
    if hh < -tol {
        return Some(out);
    }
    let h = hh.max(0.0).sqrt();
    let dir = scale(d_vec, 1.0 / d);
    let base = add(c1, scale(dir, aa));
    let perp = normalize(cross(dir, u1));
    push_unique(&mut out, add(base, scale(perp, h)), tol);
    if h > tol {
        push_unique(&mut out, sub(base, scale(perp, h)), tol);
    }
    Some(out)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn segment_ellipse_points(
    a: Point3,
    b: Point3,
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
    tol: f64,
) -> Vec<Point3> {
    let mut out = Vec::new();
    let n = normalize(normal);
    let ma = length(major_axis);
    if ma < 1e-12 {
        return out;
    }
    let major_unit = scale(major_axis, 1.0 / ma);
    let mb = ma * ratio;
    let minor_dir = ellipse_minor_dir(n, major_unit);
    let d = sub(b, a);
    let ac = sub(a, center);
    let du = dot(d, major_unit);
    let dv = dot(d, minor_dir);
    let au = dot(ac, major_unit);
    let av = dot(ac, minor_dir);
    if mb < 1e-12 {
        // Degenerate ellipse (a segment): the minor coordinate must be zero.
        if dv.abs() > tol {
            return out;
        }
        for u in [-ma, ma] {
            let t = (u - au) / du;
            if !(-tol..=1.0 + tol).contains(&t) {
                continue;
            }
            let p = add(a, scale(d, t.clamp(0.0, 1.0)));
            if dot(sub(p, center), n).abs() <= tol {
                push_unique(&mut out, p, tol);
            }
        }
        return out;
    }
    let qa = (du / ma).powi(2) + (dv / mb).powi(2);
    if qa <= 1e-300 {
        return out;
    }
    let qb = 2.0 * (au * du / (ma * ma) + av * dv / (mb * mb));
    let qc = (au / ma).powi(2) + (av / mb).powi(2) - 1.0;
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return out;
    }
    let sq = disc.sqrt();
    let ang_tol = (tol / ma.max(mb)).max(1e-12);
    for t in [(-qb - sq) / (2.0 * qa), (-qb + sq) / (2.0 * qa)] {
        if !(-tol..=1.0 + tol).contains(&t) {
            continue;
        }
        let p = add(a, scale(d, t.clamp(0.0, 1.0)));
        if dot(sub(p, center), n).abs() > tol {
            continue;
        }
        let u = dot(sub(p, center), major_unit) / ma;
        let v = dot(sub(p, center), minor_dir) / mb;
        if angle_on_arc(v.atan2(u), start, sweep, ang_tol) {
            push_unique(&mut out, p, tol);
        }
    }
    out
}

/// Analytic intersection when both operands are conic primitives.
///
/// Returns `None` when the pair is not solved analytically, so the caller can
/// fall back to sampling.
pub(crate) fn analytic_intersections(a: &Conic, b: &Conic, tol: f64) -> Option<Vec<Point3>> {
    use Conic::*;
    let mut out = Vec::new();
    match (a, b) {
        (Segment { a: a1, b: a2 }, Segment { a: b1, b: b2 }) => {
            if let Some(p) = segment_segment_intersection_3d(*a1, *a2, *b1, *b2, tol) {
                out.push(p);
            }
            Some(out)
        }
        (
            Segment { a: a1, b: a2 },
            Circle {
                center,
                normal,
                radius,
            },
        )
        | (
            Circle {
                center,
                normal,
                radius,
            },
            Segment { a: a1, b: a2 },
        ) => Some(segment_circle_points(
            *a1, *a2, *center, *normal, *radius, tol,
        )),
        (
            Segment { a: a1, b: a2 },
            Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            },
        )
        | (
            Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            },
            Segment { a: a1, b: a2 },
        ) => {
            let hits = segment_circle_points(*a1, *a2, *center, *normal, *radius, tol);
            let (ax, ay, _) = arbitrary_axis(*normal);
            let ang_tol = (tol / radius.abs().max(1e-12)).max(1e-12);
            for p in hits {
                let v = sub(p, *center);
                if angle_on_arc(dot(v, ay).atan2(dot(v, ax)), *start, *sweep, ang_tol) {
                    push_unique(&mut out, p, tol);
                }
            }
            Some(out)
        }
        (
            Segment { a: a1, b: a2 },
            Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            },
        )
        | (
            Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            },
            Segment { a: a1, b: a2 },
        ) => Some(segment_ellipse_points(
            *a1,
            *a2,
            *center,
            *normal,
            *major_axis,
            *ratio,
            *start,
            *sweep,
            tol,
        )),
        (
            Circle {
                center: c1,
                normal: n1,
                radius: r1,
            },
            Circle {
                center: c2,
                normal: n2,
                radius: r2,
            },
        ) => circle_circle_points(*c1, *n1, *r1, *c2, *n2, *r2, tol),
        (
            Circle {
                center: c1,
                normal: n1,
                radius: r1,
            },
            Arc {
                center: c2,
                normal: n2,
                radius: r2,
                start,
                sweep,
            },
        )
        | (
            Arc {
                center: c2,
                normal: n2,
                radius: r2,
                start,
                sweep,
            },
            Circle {
                center: c1,
                normal: n1,
                radius: r1,
            },
        ) => {
            let hits = circle_circle_points(*c1, *n1, *r1, *c2, *n2, *r2, tol)?;
            let (ax, ay, _) = arbitrary_axis(*n2);
            let ang_tol = (tol / r2.abs().max(1e-12)).max(1e-12);
            for p in hits {
                let v = sub(p, *c2);
                if angle_on_arc(dot(v, ay).atan2(dot(v, ax)), *start, *sweep, ang_tol) {
                    push_unique(&mut out, p, tol);
                }
            }
            Some(out)
        }
        (
            Arc {
                center: c1,
                normal: n1,
                radius: r1,
                start: s1,
                sweep: w1,
            },
            Arc {
                center: c2,
                normal: n2,
                radius: r2,
                start: s2,
                sweep: w2,
            },
        ) => {
            let hits = circle_circle_points(*c1, *n1, *r1, *c2, *n2, *r2, tol)?;
            let (ax1, ay1, _) = arbitrary_axis(*n1);
            let (ax2, ay2, _) = arbitrary_axis(*n2);
            for p in hits {
                let v1 = sub(p, *c1);
                let v2 = sub(p, *c2);
                let a1 = dot(v1, ay1).atan2(dot(v1, ax1));
                let a2 = dot(v2, ay2).atan2(dot(v2, ax2));
                if angle_on_arc(a1, *s1, *w1, 1e-9) && angle_on_arc(a2, *s2, *w2, 1e-9) {
                    push_unique(&mut out, p, tol);
                }
            }
            Some(out)
        }
        _ => None,
    }
}
