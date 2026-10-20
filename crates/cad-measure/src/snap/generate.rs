//! Candidate generation for each semantic geometry kind.

use cad_domain::*;
use cad_geometry::{
    add, arbitrary_axis, distance, dot, is_xy_parallel, length, normalize, scale, sub,
    tessellate_spline, TessellationParams,
};

use super::{Raw, SnapKind, EPS, FRAC_PI_2, TAU};

// ---- candidate generation -------------------------------------------------

pub(super) fn generate(
    geometry: &SemanticGeometry,
    probe: Option<Point3>,
    out: &mut Vec<Raw>,
    prefix: &str,
) {
    use SemanticGeometry as G;
    match geometry {
        G::Line { start, end } => line_candidates(*start, *end, probe, out, prefix),
        G::Polyline {
            points,
            bulges,
            closed,
        } => polyline_candidates(points, bulges, *closed, probe, out, prefix),
        G::Circle {
            center,
            normal,
            radius,
        } => circle_candidates(*center, *normal, *radius, probe, out, prefix),
        G::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => arc_candidates(
            *center, *normal, *radius, *start, *sweep, probe, out, prefix,
        ),
        G::Ellipse {
            center,
            major_axis,
            ratio,
            start,
            sweep,
            // The minor axis convention is `cross(world_z, major)`; any other
            // ellipse metadata is ignored here.
            ..
        } => ellipse_candidates(
            *center,
            *major_axis,
            *ratio,
            *start,
            *sweep,
            probe,
            out,
            prefix,
        ),
        G::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => spline_candidates(control_points, knots, weights, *degree, probe, out, prefix),
        G::Point(p) => push_point(out, SnapKind::Endpoint, *p, format!("{prefix}point")),
        G::Compound(children) => {
            for (i, child) in children.iter().enumerate() {
                generate(child, probe, out, &format!("{prefix}child:{i}:"));
            }
        }
        // A dimension's display children (including its dimension line and
        // extension lines) are real snap vertices, exactly like a compound
        // (ADR 0017 A.8): merging them into the empty arm would silently delete
        // them.
        G::Dimension(dimension) => {
            for (i, child) in dimension.display().iter().enumerate() {
                generate(child, probe, out, &format!("{prefix}child:{i}:"));
            }
        }
        // A hatch's boundary polylines (and pattern lines) are real snap
        // vertices, exactly like a compound (ADR 0018 E); this is what keeps
        // boundary vertices snappable after the migration.
        G::Hatch(hatch) => {
            for (i, child) in hatch.display().iter().enumerate() {
                generate(child, probe, out, &format!("{prefix}child:{i}:"));
            }
        }
        // A multileader's display children (leader vertices, dogleg endpoints)
        // are real snap vertices, exactly like the compound it used to be
        // (ADR 0019 decision G).
        G::MultiLeader(multileader) => {
            for (i, child) in multileader.display().iter().enumerate() {
                generate(child, probe, out, &format!("{prefix}child:{i}:"));
            }
        }
        G::Mesh(_)
        | G::Insert { .. }
        | G::Text { .. }
        | G::Shape { .. }
        | G::Opaque { .. }
        // Images and masks carry no analytic snap vertices here; picking and
        // bounds consumers still see them via their geometry.
        | G::Image { .. }
        | G::Mask { .. } => {}
    }
}

fn push_point(out: &mut Vec<Raw>, kind: SnapKind, point: Point3, key: String) {
    if cad_geometry::is_finite(point) {
        out.push(Raw { kind, point, key });
    }
}

fn midpoint(a: Point3, b: Point3) -> Point3 {
    scale(add(a, b), 0.5)
}

fn line_candidates(a: Point3, b: Point3, probe: Option<Point3>, out: &mut Vec<Raw>, prefix: &str) {
    push_point(out, SnapKind::Endpoint, a, format!("{prefix}vertex:0"));
    push_point(out, SnapKind::Endpoint, b, format!("{prefix}vertex:1"));
    push_point(
        out,
        SnapKind::Midpoint,
        midpoint(a, b),
        format!("{prefix}edge:0"),
    );
    if let Some(p) = probe {
        if let Some(foot) = foot_on_segment(p, a, b) {
            push_point(
                out,
                SnapKind::Perpendicular,
                foot,
                format!("{prefix}edge:0"),
            );
        }
    }
}

fn foot_on_segment(p: Point3, a: Point3, b: Point3) -> Option<Point3> {
    let d = sub(b, a);
    let len2 = dot(d, d);
    if !len2.is_finite() || len2 < EPS {
        return None;
    }
    let t = dot(sub(p, a), d) / len2;
    if !t.is_finite() || !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some(add(a, scale(d, t)))
}

fn polyline_candidates(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    probe: Option<Point3>,
    out: &mut Vec<Raw>,
    prefix: &str,
) {
    let n = points.len();
    if n == 0 {
        return;
    }
    for (i, p) in points.iter().enumerate() {
        push_point(out, SnapKind::Endpoint, *p, format!("{prefix}vertex:{i}"));
    }
    if n < 2 {
        return;
    }
    let normal = polyline_plane_normal(points);
    let last = if closed { n } else { n - 1 };
    for i in 0..last {
        let a = points[i];
        let b = points[(i + 1) % n];
        let bulge = bulges.get(i).copied().unwrap_or(0.0);
        if bulge.abs() < EPS {
            push_point(
                out,
                SnapKind::Midpoint,
                midpoint(a, b),
                format!("{prefix}edge:{i}"),
            );
            if let Some(p) = probe {
                if let Some(foot) = foot_on_segment(p, a, b) {
                    push_point(
                        out,
                        SnapKind::Perpendicular,
                        foot,
                        format!("{prefix}edge:{i}"),
                    );
                }
            }
        } else if let Some(arc) = bulge_arc_params(a, b, bulge, normal) {
            // The chord midpoint is not on the arc; only the true arc point is.
            push_point(
                out,
                SnapKind::Midpoint,
                arc.point_at(arc.start + arc.sweep * 0.5),
                format!("{prefix}edge:{i}"),
            );
            push_point(
                out,
                SnapKind::Center,
                arc.center,
                format!("{prefix}edge:{i}"),
            );
            for k in 0..4 {
                let q = k as f64 * FRAC_PI_2;
                if arc.in_sweep(q) {
                    push_point(
                        out,
                        SnapKind::Quadrant,
                        arc.point_at(q),
                        format!("{prefix}edge:{i}:q{k}"),
                    );
                }
            }
            if let Some(p) = probe {
                if let Some(point) = arc.nearest(p) {
                    push_point(
                        out,
                        SnapKind::Perpendicular,
                        point,
                        format!("{prefix}edge:{i}"),
                    );
                }
            }
        }
    }
}

fn circle_candidates(
    center: Point3,
    normal: Point3,
    radius: f64,
    probe: Option<Point3>,
    out: &mut Vec<Raw>,
    prefix: &str,
) {
    if !radius.is_finite() || radius.abs() < EPS {
        return;
    }
    let radius = radius.abs();
    push_point(out, SnapKind::Center, center, format!("{prefix}center"));
    let (ax, ay, _) = arbitrary_axis(normal);
    for (i, axis) in [ax, ay, scale(ax, -1.0), scale(ay, -1.0)]
        .iter()
        .enumerate()
    {
        push_point(
            out,
            SnapKind::Quadrant,
            add(center, scale(*axis, radius)),
            format!("{prefix}quadrant:{i}"),
        );
    }
    if let Some(p) = probe {
        if let Some(point) = nearest_on_circle(p, center, normal, radius) {
            push_point(
                out,
                SnapKind::Perpendicular,
                point,
                format!("{prefix}circle"),
            );
        }
    }
}

fn nearest_on_circle(probe: Point3, center: Point3, normal: Point3, radius: f64) -> Option<Point3> {
    let n = normalize(normal);
    let mut d = sub(probe, center);
    d = sub(d, scale(n, dot(d, n)));
    if length(d) < EPS {
        return None;
    }
    Some(add(center, scale(normalize(d), radius)))
}

#[allow(clippy::too_many_arguments)]
fn arc_candidates(
    center: Point3,
    normal: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
    probe: Option<Point3>,
    out: &mut Vec<Raw>,
    prefix: &str,
) {
    if !radius.is_finite() || radius.abs() < EPS || !sweep.is_finite() || sweep.abs() < EPS {
        return;
    }
    let radius = radius.abs();
    let sweep = normalize_sweep(sweep);
    if !start.is_finite() {
        return;
    }
    let (ax, ay, _) = arbitrary_axis(normal);
    let point_at = |t: f64| {
        add(
            center,
            add(scale(ax, t.cos() * radius), scale(ay, t.sin() * radius)),
        )
    };
    let full = sweep.abs() >= TAU - 1e-9;
    let in_sweep = move |theta: f64| {
        full || {
            let t = (theta - start) / sweep;
            (0.0..=1.0).contains(&t)
        }
    };
    push_point(out, SnapKind::Center, center, format!("{prefix}center"));
    push_point(
        out,
        SnapKind::Endpoint,
        point_at(start),
        format!("{prefix}vertex:0"),
    );
    push_point(
        out,
        SnapKind::Endpoint,
        point_at(start + sweep),
        format!("{prefix}vertex:1"),
    );
    push_point(
        out,
        SnapKind::Midpoint,
        point_at(start + sweep * 0.5),
        format!("{prefix}arc"),
    );
    for k in 0..4 {
        let q = k as f64 * FRAC_PI_2;
        if in_sweep(q) {
            push_point(
                out,
                SnapKind::Quadrant,
                point_at(q),
                format!("{prefix}quadrant:{k}"),
            );
        }
    }
    if let Some(p) = probe {
        let d = sub(p, center);
        let angle = dot(d, ay).atan2(dot(d, ax));
        for candidate in [angle, angle + std::f64::consts::PI] {
            if in_sweep(candidate) {
                push_point(
                    out,
                    SnapKind::Perpendicular,
                    point_at(candidate),
                    format!("{prefix}arc"),
                );
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn ellipse_candidates(
    center: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
    probe: Option<Point3>,
    out: &mut Vec<Raw>,
    prefix: &str,
) {
    let major_len = length(major_axis);
    if !major_len.is_finite() || major_len < EPS || !ratio.is_finite() {
        return;
    }
    let ratio = ratio.abs();
    let u = scale(major_axis, 1.0 / major_len);
    let minor_dir = ellipse_minor_dir(u);
    let minor = scale(minor_dir, major_len * ratio);
    let sweep = normalize_sweep(sweep);
    if !start.is_finite() || sweep.abs() < EPS {
        return;
    }
    let point_at = |t: f64| {
        add(
            center,
            add(scale(major_axis, t.cos()), scale(minor, t.sin())),
        )
    };
    let full = sweep.abs() >= TAU - 1e-9;
    let in_sweep = move |theta: f64| {
        full || {
            let t = (theta - start) / sweep;
            (0.0..=1.0).contains(&t)
        }
    };
    push_point(out, SnapKind::Center, center, format!("{prefix}center"));
    push_point(
        out,
        SnapKind::Endpoint,
        point_at(start),
        format!("{prefix}vertex:0"),
    );
    push_point(
        out,
        SnapKind::Endpoint,
        point_at(start + sweep),
        format!("{prefix}vertex:1"),
    );
    push_point(
        out,
        SnapKind::Midpoint,
        point_at(start + sweep * 0.5),
        format!("{prefix}ellipse"),
    );
    for k in 0..4 {
        let q = k as f64 * FRAC_PI_2;
        if in_sweep(q) {
            push_point(
                out,
                SnapKind::Quadrant,
                point_at(q),
                format!("{prefix}quadrant:{k}"),
            );
        }
    }
    if let Some(p) = probe {
        if let Some(t) = nearest_ellipse_param(p, center, major_axis, minor, start, sweep) {
            push_point(
                out,
                SnapKind::Perpendicular,
                point_at(t),
                format!("{prefix}ellipse"),
            );
        }
    }
}

/// Nearest parameter on an elliptic arc to `probe`, by a fixed-resolution scan
/// plus a golden-section refinement. The resolution is independent of the
/// display LOD so the snap does not move when the LOD changes.
fn nearest_ellipse_param(
    probe: Point3,
    center: Point3,
    major: Point3,
    minor: Point3,
    start: f64,
    sweep: f64,
) -> Option<f64> {
    const SAMPLES: usize = 360;
    let point_at = |t: f64| add(center, add(scale(major, t.cos()), scale(minor, t.sin())));
    let dist2 = |t: f64| {
        let d = sub(point_at(t), probe);
        dot(d, d)
    };
    let mut best_t = start;
    let mut best = f64::INFINITY;
    for i in 0..=SAMPLES {
        let t = start + sweep * (i as f64) / (SAMPLES as f64);
        let d = dist2(t);
        if d < best {
            best = d;
            best_t = t;
        }
    }
    let step = sweep / (SAMPLES as f64);
    let lo_bound = start.min(start + sweep);
    let hi_bound = start.max(start + sweep);
    let mut a = (best_t - step).clamp(lo_bound, hi_bound);
    let mut b = (best_t + step).clamp(lo_bound, hi_bound);
    // Golden-section search on the unimodal neighbourhood.
    let gr = (5.0f64.sqrt() - 1.0) / 2.0;
    let mut c = b - gr * (b - a);
    let mut d = a + gr * (b - a);
    for _ in 0..48 {
        if dist2(c) < dist2(d) {
            b = d;
        } else {
            a = c;
        }
        c = b - gr * (b - a);
        d = a + gr * (b - a);
        if (b - a).abs() < EPS {
            break;
        }
    }
    let best_t = 0.5 * (a + b);
    (best_t.is_finite()).then_some(best_t)
}

fn ellipse_minor_dir(major_unit: Point3) -> Point3 {
    let minor = cad_geometry::cross(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        major_unit,
    );
    if length(minor) < 1e-9 {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    } else {
        normalize(minor)
    }
}

fn spline_candidates(
    control_points: &[Point3],
    knots: &[f64],
    weights: &[f64],
    degree: u32,
    probe: Option<Point3>,
    out: &mut Vec<Raw>,
    prefix: &str,
) {
    if control_points.len() < 2 {
        return;
    }
    let params = TessellationParams {
        // The sample count is fixed; `tessellate_spline` ignores the tolerance
        // and uses the source knots, so this is LOD-independent.
        max_segments: 512,
        min_segments: 32,
        ..TessellationParams::default()
    };
    let samples = tessellate_spline(control_points, knots, weights, degree, params);
    if samples.len() < 2 {
        return;
    }
    push_point(
        out,
        SnapKind::Endpoint,
        samples[0],
        format!("{prefix}spline:start"),
    );
    push_point(
        out,
        SnapKind::Endpoint,
        samples[samples.len() - 1],
        format!("{prefix}spline:end"),
    );
    if let Some(mid) = sample_midpoint(&samples) {
        push_point(out, SnapKind::Midpoint, mid, format!("{prefix}spline"));
    }
    if let Some(p) = probe {
        if let Some(near) = samples.iter().copied().min_by(|a, b| {
            distance(*a, p)
                .partial_cmp(&distance(*b, p))
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            push_point(
                out,
                SnapKind::Perpendicular,
                near,
                format!("{prefix}spline"),
            );
        }
    }
}

/// Point at half the polyline arc length.
fn sample_midpoint(samples: &[Point3]) -> Option<Point3> {
    let mut total = 0.0;
    for w in samples.windows(2) {
        total += distance(w[0], w[1]);
    }
    if !total.is_finite() || total < EPS {
        return None;
    }
    let half = total * 0.5;
    let mut walked = 0.0;
    for w in samples.windows(2) {
        let seg = distance(w[0], w[1]);
        if walked + seg >= half {
            let t = if seg > EPS {
                (half - walked) / seg
            } else {
                0.0
            };
            return Some(add(w[0], scale(sub(w[1], w[0]), t)));
        }
        walked += seg;
    }
    samples.last().copied()
}

fn polyline_plane_normal(points: &[Point3]) -> Point3 {
    let world_z = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    let n = points.len();
    if n < 3 {
        return world_z;
    }
    let mut acc = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        acc.x += (a.y - b.y) * (a.z + b.z);
        acc.y += (a.z - b.z) * (a.x + b.x);
        acc.z += (a.x - b.x) * (a.y + b.y);
    }
    if length(acc) < EPS {
        world_z
    } else if is_xy_parallel(points) {
        // 2D data: `+Z`, never the winding (see `cad_geometry::is_xy_parallel`).
        world_z
    } else {
        normalize(acc)
    }
}

/// An arc reconstructed from an AutoCAD bulge, in the polyline's own plane.
struct BulgeArc {
    center: Point3,
    ax: Point3,
    ay: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
}

impl BulgeArc {
    fn point_at(&self, t: f64) -> Point3 {
        add(
            self.center,
            add(
                scale(self.ax, t.cos() * self.radius),
                scale(self.ay, t.sin() * self.radius),
            ),
        )
    }

    fn in_sweep(&self, theta: f64) -> bool {
        let t = (theta - self.start) / self.sweep;
        (0.0..=1.0).contains(&t)
    }

    fn nearest(&self, probe: Point3) -> Option<Point3> {
        let d = sub(probe, self.center);
        let angle = dot(d, self.ay).atan2(dot(d, self.ax));
        for candidate in [angle, angle + std::f64::consts::PI] {
            if self.in_sweep(candidate) {
                return Some(self.point_at(candidate));
            }
        }
        None
    }
}

/// Reconstruct a bulge arc (`bulge = tan(theta/4)`) in the plane with normal
/// `plane_normal`, mirroring the polyline tessellation convention.
fn bulge_arc_params(a: Point3, b: Point3, bulge: f64, plane_normal: Point3) -> Option<BulgeArc> {
    let chord = sub(b, a);
    let chord_len = length(chord);
    if !chord_len.is_finite() || chord_len < EPS || !bulge.is_finite() {
        return None;
    }
    let (ax, ay, _) = arbitrary_axis(plane_normal);
    let d2 = [dot(chord, ax), dot(chord, ay)];
    let chord2 = (d2[0] * d2[0] + d2[1] * d2[1]).sqrt();
    if chord2 < EPS {
        return None;
    }
    let theta = 4.0 * bulge.atan();
    let half = theta * 0.5;
    let half_chord = chord_len * 0.5;
    let sagitta = bulge * half_chord;
    if sagitta.abs() < EPS {
        return None;
    }
    let radius = (half_chord * half_chord + sagitta * sagitta) / (2.0 * sagitta.abs());
    let dir2 = [d2[0] / chord2, d2[1] / chord2];
    let left2 = [-dir2[1], dir2[0]];
    let u2 = if sagitta >= 0.0 {
        left2
    } else {
        [-left2[0], -left2[1]]
    };
    let mid2 = [d2[0] * 0.5, d2[1] * 0.5];
    // Same construction as the polyline tessellator: `+u2 * r * cos(theta/2)`.
    let center2 = [
        mid2[0] + u2[0] * radius * half.cos(),
        mid2[1] + u2[1] * radius * half.cos(),
    ];
    let center = add(a, add(scale(ax, center2[0]), scale(ay, center2[1])));
    let start = (-center2[1]).atan2(-center2[0]);
    Some(BulgeArc {
        center,
        ax,
        ay,
        radius,
        start,
        sweep: theta,
    })
}

/// Normalise an arc/ellipse sweep to a magnitude no larger than a full turn,
/// keeping its sign.
fn normalize_sweep(sweep: f64) -> f64 {
    if sweep.abs() >= TAU - 1e-9 {
        if sweep < 0.0 {
            -TAU
        } else {
            TAU
        }
    } else {
        sweep
    }
}
