//! HATCH boundary tessellation and pattern extraction.

use super::*;

pub(crate) const HATCH_EPS: f64 = 1e-9;

/// A tessellation tolerance scaled to the hatch's own coordinate magnitude.
///
/// Hatch spline edges are tessellated eagerly (the pattern needs 2D loops), so
/// a fixed world tolerance would explode for drawings in the tens of thousands
/// of units. A relative tolerance keeps the segment count bounded.
pub(crate) fn hatch_tolerance(h: &Hatch) -> f64 {
    let mut magnitude = 0.0f64;
    let mut bump = |x: f64, y: f64| {
        magnitude = magnitude.max(x.abs()).max(y.abs());
    };
    for path in &h.paths {
        for edge in &path.edges {
            match edge {
                BoundaryEdge::Line(l) => {
                    bump(l.start.x, l.start.y);
                    bump(l.end.x, l.end.y);
                }
                BoundaryEdge::CircularArc(a) => {
                    bump(a.center.x, a.center.y);
                    bump(a.center.x + a.radius, a.center.y + a.radius);
                }
                BoundaryEdge::EllipticArc(a) => {
                    bump(a.center.x, a.center.y);
                    bump(
                        a.center.x + a.major_axis_endpoint.x,
                        a.center.y + a.major_axis_endpoint.y,
                    );
                }
                BoundaryEdge::Spline(s) => {
                    for p in &s.control_points {
                        bump(p.x, p.y);
                    }
                }
                BoundaryEdge::Polyline(pl) => {
                    for v in &pl.vertices {
                        bump(v.x, v.y);
                    }
                }
            }
        }
    }
    (magnitude * 1e-4).max(1e-6)
}

/// Append one HATCH boundary edge, sampled into the hatch plane (2D).
pub(crate) fn append_boundary_edge(
    edge: &BoundaryEdge,
    out: &mut Vec<[f64; 2]>,
    params: &TessellationParams,
) {
    match edge {
        BoundaryEdge::Line(l) => {
            push_hatch_point(out, [l.start.x, l.start.y]);
            push_hatch_point(out, [l.end.x, l.end.y]);
        }
        BoundaryEdge::CircularArc(a) => {
            let sweep = directed_sweep(a.start_angle, a.end_angle, a.counter_clockwise);
            let n = arc_steps(sweep);
            for i in 0..=n {
                let t = a.start_angle + sweep * (i as f64) / (n as f64);
                push_hatch_point(
                    out,
                    [
                        a.center.x + a.radius * t.cos(),
                        a.center.y + a.radius * t.sin(),
                    ],
                );
            }
        }
        BoundaryEdge::EllipticArc(a) => {
            let major = [a.major_axis_endpoint.x, a.major_axis_endpoint.y];
            let major_len = (major[0] * major[0] + major[1] * major[1]).sqrt();
            if major_len < HATCH_EPS {
                return;
            }
            let u = [major[0] / major_len, major[1] / major_len];
            let minor = [
                -u[1] * major_len * a.minor_axis_ratio,
                u[0] * major_len * a.minor_axis_ratio,
            ];
            let sweep = directed_sweep(a.start_angle, a.end_angle, a.counter_clockwise);
            let n = arc_steps(sweep);
            for i in 0..=n {
                let t = a.start_angle + sweep * (i as f64) / (n as f64);
                let (sin, cos) = t.sin_cos();
                push_hatch_point(
                    out,
                    [
                        a.center.x + u[0] * major_len * cos + minor[0] * sin,
                        a.center.y + u[1] * major_len * cos + minor[1] * sin,
                    ],
                );
            }
        }
        BoundaryEdge::Spline(s) => {
            if s.control_points.len() < 2 {
                return;
            }
            let control: Vec<Point3> = s.control_points.iter().map(|p| p3(*p)).collect();
            let degree = s.degree.max(1) as u32;
            for p in tessellate_bspline(&control, degree, *params) {
                push_hatch_point(out, [p.x, p.y]);
            }
        }
        BoundaryEdge::Polyline(pl) => {
            for v in &pl.vertices {
                push_hatch_point(out, [v.x, v.y]);
            }
            if pl.is_closed {
                if let Some(first) = pl.vertices.first() {
                    push_hatch_point(out, [first.x, first.y]);
                }
            }
        }
    }
}

pub(crate) fn push_hatch_point(out: &mut Vec<[f64; 2]>, p: [f64; 2]) {
    if let Some(last) = out.last() {
        if (last[0] - p[0]).abs() <= HATCH_EPS && (last[1] - p[1]).abs() <= HATCH_EPS {
            return;
        }
    }
    out.push(p);
}

/// Drop a duplicated closing vertex (loops are closed implicitly).
pub(crate) fn dedup_loop(points: &mut Vec<[f64; 2]>) {
    if points.len() >= 2 {
        let first = points[0];
        let last = points[points.len() - 1];
        if (first[0] - last[0]).abs() <= HATCH_EPS && (first[1] - last[1]).abs() <= HATCH_EPS {
            points.pop();
        }
    }
}

pub(crate) fn directed_sweep(start: f64, end: f64, counter_clockwise: bool) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut sweep = end - start;
    if counter_clockwise {
        while sweep <= 0.0 {
            sweep += tau;
        }
    } else {
        while sweep >= 0.0 {
            sweep -= tau;
        }
    }
    sweep
}

pub(crate) fn arc_steps(sweep: f64) -> usize {
    ((sweep.abs() / 0.05).ceil() as usize).clamp(2, 4096)
}

/// Build the (possibly doubled) pattern line families, applying the hatch's
/// pattern angle and scale.
pub(crate) fn pattern_families(h: &Hatch) -> Vec<PatternLine> {
    let (sin, cos) = h.pattern_angle.sin_cos();
    let mut families = Vec::new();
    for line in &h.pattern.lines {
        let base_raw = [line.base_point.x, line.base_point.y];
        let base = [
            cos * base_raw[0] - sin * base_raw[1],
            sin * base_raw[0] + cos * base_raw[1],
        ];
        let offset = [
            line.offset.x * h.pattern_scale,
            line.offset.y * h.pattern_scale,
        ];
        let dashes = line
            .dash_lengths
            .iter()
            .map(|d| d * h.pattern_scale)
            .collect::<Vec<_>>();
        let angle = line.angle + h.pattern_angle;
        families.push(PatternLine {
            angle,
            base,
            offset,
            dashes: dashes.clone(),
        });
        if h.is_double {
            families.push(PatternLine {
                angle: angle + std::f64::consts::FRAC_PI_2,
                base,
                offset: [-offset[1], offset[0]],
                dashes,
            });
        }
    }
    families
}
