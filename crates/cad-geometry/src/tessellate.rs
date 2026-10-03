//! tessellate module.

use super::*;

/// Discretise any curve-like geometry into a point polyline.
///
/// Meshes, inserts and opaque payloads yield an empty vector: they are handled
/// by the representation layer, not as polylines.
pub fn tessellate_geometry(geometry: &SemanticGeometry, params: TessellationParams) -> Vec<Point3> {
    use SemanticGeometry as G;
    match geometry {
        G::Line { start, end } => vec![*start, *end],
        G::Polyline {
            points,
            bulges,
            closed,
        } => polyline_with_bulges(points, bulges, *closed, params),
        G::Circle {
            center,
            normal,
            radius,
        } => {
            let (ax, ay, _) = arbitrary_axis(*normal);
            let mut out = Vec::new();
            let n = arc_segments_for_tolerance(*radius, std::f64::consts::TAU, params);
            for i in 0..=n {
                let t = std::f64::consts::TAU * (i as f64) / (n as f64);
                out.push(add(
                    *center,
                    add(scale(ax, t.cos() * radius), scale(ay, t.sin() * radius)),
                ));
            }
            out
        }
        G::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let (ax, ay, _) = arbitrary_axis(*normal);
            // Preserve the sweep's sign: a reflected/clockwise arc (for example
            // from a mirror) must not be re-drawn as its complement (audit B23).
            let sweep = if sweep.abs() >= std::f64::consts::TAU - 1e-9 {
                std::f64::consts::TAU
            } else {
                *sweep
            };
            let n = arc_segments_for_tolerance(*radius, sweep, params);
            let mut out = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = start + sweep * (i as f64) / (n as f64);
                out.push(add(
                    *center,
                    add(scale(ax, t.cos() * radius), scale(ay, t.sin() * radius)),
                ));
            }
            out
        }
        G::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            let major_len = length(*major_axis);
            if major_len < 1e-12 {
                return vec![*center];
            }
            let u = scale(*major_axis, 1.0 / major_len);
            // The minor axis lies in the ellipse's own plane, defined by the
            // stored extrusion normal: `minor = cross(normal, major)` (audit
            // B23). This keeps an ellipse on an arbitrary OCS plane in that
            // plane instead of folding it onto world XY.
            let minor = scale(ellipse_minor_dir(*normal, u), major_len * ratio.abs());
            let sweep = if sweep.abs() >= std::f64::consts::TAU - 1e-9 {
                std::f64::consts::TAU
            } else {
                *sweep
            };
            let r = major_len.max(major_len * ratio.abs());
            let n = arc_segments_for_tolerance(r, sweep, params);
            let mut out = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = start + sweep * (i as f64) / (n as f64);
                out.push(add(
                    *center,
                    add(scale(*major_axis, t.cos()), scale(minor, t.sin())),
                ));
            }
            out
        }
        G::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => tessellate_spline(control_points, knots, weights, *degree, params),
        G::Point(p) => vec![*p, *p],
        G::Text {
            position,
            height,
            text,
            ..
        } => {
            // A text placeholder box edge, sufficient for picking bounds.
            let h = height.abs().max(1e-9);
            vec![
                *position,
                Point3 {
                    x: position.x + h * text.chars().count() as f64,
                    y: position.y + h,
                    z: position.z,
                },
            ]
        }
        G::Mesh(_) | G::Insert { .. } | G::Opaque { .. } | G::Shape { .. } => Vec::new(),
        G::Compound(children) => {
            let mut out = Vec::new();
            for child in children {
                out.extend(tessellate_geometry(child, params));
            }
            out
        }
    }
}

/// Segments needed so an arc keeps its sagitta under the tolerance.
pub fn arc_segments_for_tolerance(radius: f64, sweep: f64, params: TessellationParams) -> usize {
    let sweep = sweep.abs();
    if sweep <= 1e-12 {
        return 1;
    }
    let radius = radius.abs().max(1e-12);
    let ratio = 1.0 - (params.tolerance / radius);
    let step = if ratio <= -1.0 {
        std::f64::consts::FRAC_PI_2
    } else {
        2.0 * ratio.clamp(-1.0, 1.0).acos()
    };
    let step = step.max(1e-6);
    let n = (sweep / step).ceil() as usize;
    n.clamp(params.min_segments.max(1), params.max_segments.max(1))
}

pub(crate) fn polyline_with_bulges(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    params: TessellationParams,
) -> Vec<Point3> {
    if points.is_empty() {
        return Vec::new();
    }
    // Bulge arcs lie in the polyline's own plane, which need not be world XY
    // (an OCS/tilted polyline from the importer). Derive that plane from the
    // vertices so a tilted bulge is not silently flattened (audit B23).
    let plane_normal = polyline_plane_normal(points);
    let mut out: Vec<Point3> = Vec::with_capacity(points.len() * 4);
    let count = points.len();
    let last = if closed {
        count
    } else {
        count.saturating_sub(1)
    };
    for i in 0..last {
        let a = points[i];
        let b = points[(i + 1) % count];
        let bulge = bulges.get(i).copied().unwrap_or(0.0);
        // `a` is always the previous segment's endpoint, so the first point of
        // each segment is pushed by whichever branch runs (audit B23).
        if bulge.abs() < 1e-12 {
            if out.is_empty() {
                out.push(a);
            }
            out.push(b);
        } else {
            append_bulge_arc(a, b, bulge, plane_normal, params, &mut out);
        }
    }
    out
}

/// The best-fit plane normal of a polyline, using Newell's method.
///
/// The closing edge is always included so the polygon normal is well defined
/// even for an open polyline. Falls back to world Z for degenerate input
/// (fewer than three points, or collinear points).
pub(crate) fn polyline_plane_normal(points: &[Point3]) -> Point3 {
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
    if length(acc) < 1e-12 {
        world_z
    } else {
        normalize(acc)
    }
}

/// Append the arc described by a bulge (`bulge = tan(theta/4)`).
///
/// The point `a` is always emitted first so the returned polyline is
/// continuous even when a straight segment precedes this arc (audit B23). The
/// arc is computed in the polyline's own plane (`plane_normal`); world-Z
/// polylines reduce to the original XY math.
pub(crate) fn append_bulge_arc(
    a: Point3,
    b: Point3,
    bulge: f64,
    plane_normal: Point3,
    params: TessellationParams,
    out: &mut Vec<Point3>,
) {
    let chord = sub(b, a);
    let chord_len = length(chord);
    if chord_len < 1e-12 {
        if !ends_at(out, b) {
            out.push(b);
        }
        return;
    }
    let theta = 4.0 * bulge.atan();
    let half = theta * 0.5;
    let half_chord = chord_len * 0.5;
    let sagitta = bulge * half_chord;
    if sagitta.abs() < 1e-12 {
        if !ends_at(out, a) {
            out.push(a);
        }
        out.push(b);
        return;
    }
    let radius_abs = (half_chord * half_chord + sagitta * sagitta) / (2.0 * sagitta.abs());
    // Orthonormal in-plane axes; `a` is the 2D origin, the chord is `d2`.
    let (ax, ay, _) = arbitrary_axis(plane_normal);
    let d2 = [dot(chord, ax), dot(chord, ay)];
    let chord2_len = (d2[0] * d2[0] + d2[1] * d2[1]).sqrt();
    if chord2_len < 1e-12 {
        // The chord is parallel to the plane normal: no in-plane arc exists.
        if !ends_at(out, a) {
            out.push(a);
        }
        out.push(b);
        return;
    }
    let dir2 = [d2[0] / chord2_len, d2[1] / chord2_len];
    let left2 = [-dir2[1], dir2[0]];
    let u2 = if sagitta >= 0.0 {
        left2
    } else {
        [-left2[0], -left2[1]]
    };
    let mid2 = [d2[0] * 0.5, d2[1] * 0.5];
    let center2 = [
        mid2[0] - u2[0] * radius_abs * half.cos(),
        mid2[1] - u2[1] * radius_abs * half.cos(),
    ];
    // `a` is the origin, so its angle is measured against the reflected centre.
    let start_angle = (-center2[1]).atan2(-center2[0]);
    let n = arc_segments_for_tolerance(radius_abs, theta, params);
    if !ends_at(out, a) {
        out.push(a);
    }
    for i in 1..=n {
        let t = start_angle + theta * (i as f64) / (n as f64);
        let px = center2[0] + t.cos() * radius_abs;
        let py = center2[1] + t.sin() * radius_abs;
        out.push(add(a, add(scale(ax, px), scale(ay, py))));
    }
    // Snap the final sample exactly onto the arc endpoint so a polyline that
    // closes on itself shares the vertex exactly (audit B23).
    if let Some(last) = out.last_mut() {
        *last = b;
    }
}

/// Whether the polyline `out` already ends at `p` (within a small tolerance).
pub(crate) fn ends_at(out: &[Point3], p: Point3) -> bool {
    out.last().map(|l| distance(*l, p) < 1e-9).unwrap_or(false)
}

/// Tessellate a clamped uniform B-spline from control points.
///
/// Convenience wrapper used by hatch boundaries that carry no source knots;
/// it synthesises clamped uniform knots. Source splines with explicit knots or
/// weights go through [`tessellate_spline`] instead (audit B23).
pub fn tessellate_bspline(
    control: &[Point3],
    degree: u32,
    params: TessellationParams,
) -> Vec<Point3> {
    let n = control.len();
    if n == 0 {
        return Vec::new();
    }
    let k = (degree as usize).max(1).min(n.saturating_sub(1));
    let knots = clamped_uniform_knots(n, k);
    let weights = vec![1.0; n];
    tessellate_spline(control, &knots, &weights, degree, params)
}

/// Tessellate a rational B-spline using its source knots and weights.
///
/// The source knot vector and weights are authoritative: uniform/periodic/
/// clamped and rational arcs all keep their true shape, and the sampling is
/// adaptive to the chord tolerance. Malformed input (wrong-length or
/// non-monotone knots, bad degree) falls back to the clamped-uniform
/// convention rather than silently producing a garbled curve (audit B23).
pub fn tessellate_spline(
    control: &[Point3],
    knots: &[f64],
    weights: &[f64],
    degree: u32,
    params: TessellationParams,
) -> Vec<Point3> {
    let n = control.len();
    if n == 0 {
        return Vec::new();
    }
    let k = (degree as usize).max(1);
    if n <= k {
        return control.to_vec();
    }
    let curve =
        NurbsCurve::new(k, control.to_vec(), knots.to_vec(), weights.to_vec()).or_else(|_| {
            NurbsCurve::new(
                k.min(n - 1),
                control.to_vec(),
                clamped_uniform_knots(n, k.min(n - 1)),
                if weights.len() == n {
                    weights.to_vec()
                } else {
                    Vec::new()
                },
            )
        });
    match curve {
        Ok(curve) => curve.discretize(params.tolerance, &params),
        Err(_) => control.to_vec(),
    }
}

/// The AutoCAD arbitrary axis algorithm.
pub fn arbitrary_axis(normal: Point3) -> (Point3, Point3, Point3) {
    let n = if length(normal) < 1e-24 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    } else {
        normalize(normal)
    };
    const ONE_64TH: f64 = 1.0 / 64.0;
    let ax = if n.x.abs() < ONE_64TH && n.y.abs() < ONE_64TH {
        cross(
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            n,
        )
    } else {
        cross(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            n,
        )
    };
    let ax = if length(ax) < 1e-24 {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    } else {
        normalize(ax)
    };
    let ay = normalize(cross(n, ax));
    (ax, ay, n)
}
