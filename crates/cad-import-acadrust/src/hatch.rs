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
        BoundaryEdge::Spline(s) => append_spline_edge(s, out, params),
        BoundaryEdge::Polyline(pl) => append_polyline_edge(pl, out),
    }
}

/// Append a HATCH boundary spline edge, honouring its own degree, knots and
/// (rational) weights.
///
/// The source knot vector and weights are authoritative when they are usable:
/// `n + degree + 1` knots and a finite, positive weight per control point. When
/// they are not, this falls back to the clamped-uniform B-spline convention
/// ([`tessellate_bspline`]) rather than inventing geometry. A periodic spline
/// keeps its knot vector too; when the vector cannot be represented as a
/// closed curve the sampled points are kept as-is (no panic, no fabrication).
/// A fit-point-only spline (no control polygon) falls back to its fit points as
/// a polyline.
fn append_spline_edge(
    s: &acadrust::entities::SplineEdge,
    out: &mut Vec<[f64; 2]>,
    params: &TessellationParams,
) {
    let n = s.control_points.len();
    if n < 2 {
        // No control polygon: a fit-point spline still has the sampled points
        // the file stored on the curve, so use them as a fallback polyline.
        if s.fit_points.len() >= 2 {
            for p in &s.fit_points {
                push_hatch_point(out, [p.x, p.y]);
            }
        }
        return;
    }
    let control: Vec<Point3> = s.control_points.iter().map(|p| p3(*p)).collect();
    let degree = s.degree.max(1) as u32;
    if s.knots.len() != n + degree as usize + 1 {
        // The knot vector cannot define this control polygon; fall back to the
        // clamped-uniform convention (documented fallback, not fabricated).
        for p in tessellate_bspline(&control, degree, *params) {
            push_hatch_point(out, [p.x, p.y]);
        }
        return;
    }
    // A rational spline carries the per-control-point weight in `z`; a
    // non-finite or non-positive weight is rejected by `NurbsCurve`, so treat
    // it as 1.0 instead of dropping the whole curve.
    let weights: Vec<f64> = if s.rational {
        s.control_points
            .iter()
            .map(|p| {
                if p.z.is_finite() && p.z > 0.0 {
                    p.z
                } else {
                    1.0
                }
            })
            .collect()
    } else {
        vec![1.0; n]
    };
    // `tessellate_spline` treats the source knots as authoritative (uniform,
    // clamped or periodic) and internally falls back when they are malformed,
    // so a periodic knot vector that cannot close simply keeps its samples.
    for p in cad_geometry::tessellate_spline(&control, &s.knots, &weights, degree, *params) {
        push_hatch_point(out, [p.x, p.y]);
    }
}

/// Append a HATCH boundary polyline edge, expanding per-vertex bulges into
/// circular arcs in the hatch plane.
fn append_polyline_edge(pl: &acadrust::entities::PolylineEdge, out: &mut Vec<[f64; 2]>) {
    let verts = &pl.vertices;
    match verts.len() {
        0 => {}
        1 => push_hatch_point(out, [verts[0].x, verts[0].y]),
        _ => {
            for i in 0..verts.len() - 1 {
                let a = verts[i];
                let b = verts[i + 1];
                push_bulge_segment(out, [a.x, a.y], [b.x, b.y], a.z);
            }
            if pl.is_closed {
                // The wrap segment p_last -> p_first carries the last vertex's
                // bulge; the closing point is pushed and later deduplicated.
                let a = verts[verts.len() - 1];
                let b = verts[0];
                push_bulge_segment(out, [a.x, a.y], [b.x, b.y], a.z);
            }
        }
    }
}

/// Sample one bulged polyline segment from `p0` to `p1` into `out`.
///
/// `bulge` is the DXF bulge (`tan(included_angle / 4)`); a zero, degenerate or
/// non-finite bulge falls back to the straight chord. The arc is sampled
/// adaptively with the same angular step as the other boundary arcs.
fn push_bulge_segment(out: &mut Vec<[f64; 2]>, p0: [f64; 2], p1: [f64; 2], bulge: f64) {
    push_hatch_point(out, p0);
    let dx = p1[0] - p0[0];
    let dy = p1[1] - p0[1];
    let chord = (dx * dx + dy * dy).sqrt();
    if !bulge.is_finite() || bulge.abs() <= HATCH_EPS || chord <= HATCH_EPS {
        push_hatch_point(out, p1);
        return;
    }
    let theta = 4.0 * bulge.atan();
    let half = theta / 2.0;
    let (sin_half, cos_half) = half.sin_cos();
    // A full-turn bulge has no finite circular centre; keep the chord.
    if sin_half.abs() <= HATCH_EPS {
        push_hatch_point(out, p1);
        return;
    }
    let radius = chord / (2.0 * sin_half);
    let mid = [(p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0];
    // Left normal of p0 -> p1; the signed `radius * cos(half)` places the
    // centre on the side selected by the bulge's sign.
    let perp = [-dy / chord, dx / chord];
    let center = [
        mid[0] + perp[0] * radius * cos_half,
        mid[1] + perp[1] * radius * cos_half,
    ];
    if !radius.is_finite() || !center[0].is_finite() || !center[1].is_finite() {
        push_hatch_point(out, p1);
        return;
    }
    let radius = radius.abs();
    let start_angle = (p0[1] - center[1]).atan2(p0[0] - center[0]);
    let n = arc_steps(theta);
    for i in 1..n {
        let t = start_angle + theta * (i as f64) / (n as f64);
        let (sin, cos) = t.sin_cos();
        push_hatch_point(out, [center[0] + radius * cos, center[1] + radius * sin]);
    }
    push_hatch_point(out, p1);
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

/// The outcome of translating a HATCH's gradient pattern.
///
/// `Unsupported` carries a stable reason code so the importer can report an
/// explicit `Partial` for a gradient it will not approximate, rather than
/// silently drawing the boundary or a solid fill.
pub(crate) enum GradientTranslation {
    /// No gradient is enabled on the hatch.
    Disabled,
    /// A supported gradient, ready to bake into per-vertex colours.
    Supported(GradientDef),
    /// A gradient kind outside the implemented subset.
    Unsupported(&'static str),
}

/// Translate acadrust's `HatchGradientPattern` into the geometry-level model.
///
/// Supported kinds are the parametric ones whose value is a function of a
/// single projection/radius: `LINEAR` and `SPHERICAL` (and its `CYLINDER`
/// alias, which is the same radial ramp in the hatch plane). Every other DXF
/// gradient kind is curved or piecewise and is reported unsupported rather than
/// approximated as a straight ramp.
pub(crate) fn translate_gradient(h: &Hatch) -> GradientTranslation {
    let g = &h.gradient_color;
    if !g.is_enabled() {
        return GradientTranslation::Disabled;
    }
    let name = g.name.trim().to_ascii_uppercase();
    let kind = match name.as_str() {
        "LINEAR" => GradientKind::Linear,
        "SPHERICAL" | "CYLINDER" => GradientKind::Spherical,
        // Curved / inversion gradients are not a single radial ramp; refuse
        // them explicitly instead of substituting an approximation.
        "HEMISPHERICAL" | "CURVED" | "INVSPHERICAL" | "INVCYLINDER" => {
            return GradientTranslation::Unsupported("gradient_kind_not_supported");
        }
        "" => {
            // A missing name is ambiguous; fall back to LINEAR only when the
            // file actually carried stops, otherwise report it.
            if g.colors.is_empty() {
                return GradientTranslation::Unsupported("gradient_name_missing");
            }
            GradientKind::Linear
        }
        _ => return GradientTranslation::Unsupported("gradient_kind_not_supported"),
    };
    let stops: Vec<GradientStop> = g
        .colors
        .iter()
        .map(|entry| GradientStop {
            value: entry.value,
            // A symbolic gradient stop has no concrete colour; white is the
            // documented neutral rather than a fabricated hue.
            rgb: concrete_rgb(entry.color).unwrap_or([255, 255, 255]),
        })
        .collect();
    let def = GradientDef {
        kind,
        angle: g.angle,
        shift: g.shift,
        single_color: g.is_single_color,
        tint: g.color_tint,
        stops,
    };
    if def.is_usable() {
        GradientTranslation::Supported(def)
    } else {
        GradientTranslation::Unsupported("gradient_definition_unusable")
    }
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
