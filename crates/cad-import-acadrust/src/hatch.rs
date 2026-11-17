//! HATCH boundary tessellation, pattern/gradient translation and source edges.

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
            let sweep = boundary_arc_sweep(a.start_angle, a.end_angle);
            let n = arc_steps(sweep);
            for i in 0..=n {
                let t = boundary_arc_parameter(
                    a.start_angle,
                    sweep,
                    i as f64 / n as f64,
                    a.counter_clockwise,
                );
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
            let sweep = boundary_arc_sweep(a.start_angle, a.end_angle);
            let n = arc_steps(sweep);
            for i in 0..=n {
                let t = boundary_arc_parameter(
                    a.start_angle,
                    sweep,
                    i as f64 / n as f64,
                    a.counter_clockwise,
                );
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
pub(crate) fn push_bulge_segment(out: &mut Vec<[f64; 2]>, p0: [f64; 2], p1: [f64; 2], bulge: f64) {
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

pub(crate) fn arc_steps(sweep: f64) -> usize {
    ((sweep.abs() / 0.05).ceil() as usize).clamp(2, 4096)
}

/// The positive parameter sweep of a HATCH boundary arc.
///
/// A boundary arc's `counter_clockwise` flag is not only a direction: **a
/// clockwise edge stores its angles mirrored** (the complementary angle,
/// `360° - θ`), so the edge really runs from `-start` to `-end`. Read as literal
/// angles, a flat 1° edge sweeps 359° — a 3 500-unit arc out of a 27-unit chord,
/// which is the "one hatch explodes the drawing bounds" failure this build had.
///
/// Evidence (ADR 0018 §B.7, U-14): it is the de-facto convention every
/// implementation follows. ezdxf mirrors clockwise hatch angles on load
/// (<https://github.com/mozman/ezdxf/issues/175>, "no official documentation on
/// this"); GDAL negates them (`ogrdxf_hatch.cpp`); acadrust's own boundary code
/// does the same (`count.rs`'s `s = -1` axis, `explode/mod.rs`'s
/// `(-end, -start)`); and acadrust issue #241 measured 44 production DWGs where
/// the mirrored reading joins the boundary edges to 1e-7 in 40 of them while the
/// literal reading misses in **all 44**. The Autodesk DXF/DWG references state
/// only "start angle" / "is counterclockwise" and never spell out the
/// interaction, so this is a de-facto convention, not a documented one.
///
/// The sweep is the **stored difference** (always positive); the direction is
/// applied by [`boundary_arc_parameter`], not by wrapping the angles.
pub(crate) fn boundary_arc_sweep(start: f64, end: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let raw = end - start;
    let mut sweep = raw.rem_euclid(tau);
    // `start = 0, end = 2π` normalises to zero but means a whole turn. Only a
    // really identical angle pair is a degenerate zero-length edge.
    if sweep.abs() <= HATCH_EPS && raw.abs() > HATCH_EPS {
        sweep = tau;
    }
    sweep
}

/// The parameter of one boundary-arc sample, in the frame the edge stores.
///
/// A clockwise edge is traversed in the mirrored frame, which is the same point
/// set as negating an ellipse's minor axis (ADR 0018 §B.7).
pub(crate) fn boundary_arc_parameter(
    start: f64,
    sweep: f64,
    fraction: f64,
    counter_clockwise: bool,
) -> f64 {
    let stored = start + sweep * fraction;
    if counter_clockwise {
        stored
    } else {
        -stored
    }
}

/// Largest junction gap a boundary path may have, relative to its own extent.
///
/// A closed boundary chain joins end to end. Real files round their vertices
/// (this fixture quantises to 0.1–0.5 units against 5–30-unit features, ~1.5 % of
/// a path), so a small residual is data, not a misreading. A junction that
/// misses by a tenth of the path's own diagonal is a broken chain and is
/// reported; nothing is ever dropped or re-read on the strength of it.
const MAX_JOIN_GAP_FRACTION: f64 = 0.1;

/// How well one boundary path's edges join, measured after sampling.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct BoundaryJoinReport {
    /// The path has a junction missing by more than
    /// [`MAX_JOIN_GAP_FRACTION`] of its own diagonal.
    pub broken: bool,
    /// Largest junction gap, in plane units.
    pub max_gap: f64,
    /// The path's own bounding-box diagonal, in plane units.
    pub extent: f64,
}

/// Sample one HATCH boundary path and measure how its edges join.
///
/// Every edge goes through [`append_boundary_edge`], so the geometry is exactly
/// the per-edge sampling; this only records the residuals the chain implies. A
/// path whose stored parameters are read wrongly (or whose file genuinely does
/// not close) shows up as a gap comparable to the path itself — reported by the
/// caller, never silently re-read (ADR 0018 §B.7 / U-14).
pub(crate) fn sample_boundary_path(
    edges: &[BoundaryEdge],
    out: &mut Vec<[f64; 2]>,
    params: &TessellationParams,
) -> BoundaryJoinReport {
    let start_index = out.len();
    let mut report = BoundaryJoinReport::default();
    let mut previous: Option<[f64; 2]> = None;
    let mut edge_points: Vec<[f64; 2]> = Vec::new();
    for edge in edges {
        // Sample the edge on its own: the loop dedups a repeated vertex, so the
        // edge's own first sample (not the first point actually appended) is what
        // the junction is measured against.
        edge_points.clear();
        append_boundary_edge(edge, &mut edge_points, params);
        let Some((&edge_start, &edge_end)) = edge_points.first().zip(edge_points.last()) else {
            continue;
        };
        if let Some(previous) = previous {
            report.max_gap = report.max_gap.max(distance(previous, edge_start));
        }
        for &point in &edge_points {
            push_hatch_point(out, point);
        }
        previous = Some(edge_end);
    }
    report.extent = extent(&out[start_index..]);
    if let (Some(&first), Some(&last)) = (out.get(start_index), out.last()) {
        report.max_gap = report.max_gap.max(distance(first, last));
    }
    report.broken = report.extent > 0.0 && report.max_gap > report.extent * MAX_JOIN_GAP_FRACTION;
    report
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// The bounding-box diagonal of a point set.
fn extent(points: &[[f64; 2]]) -> f64 {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for p in points {
        min[0] = min[0].min(p[0]);
        min[1] = min[1].min(p[1]);
        max[0] = max[0].max(p[0]);
        max[1] = max[1].max(p[1]);
    }
    if !min[0].is_finite() {
        return 0.0;
    }
    ((max[0] - min[0]).powi(2) + (max[1] - min[1]).powi(2)).sqrt()
}

// ---------------------------------------------------------------------------
// Style / fill translation (ADR 0018 B).
// ---------------------------------------------------------------------------

pub(crate) fn hatch_style(style: acadrust::entities::HatchStyleType) -> HatchStyle {
    use acadrust::entities::HatchStyleType as S;
    match style {
        S::Normal => HatchStyle::Normal,
        S::Outer => HatchStyle::Outer,
        S::Ignore => HatchStyle::Ignore,
    }
}

pub(crate) fn hatch_pattern_type(
    pattern_type: acadrust::entities::HatchPatternType,
) -> HatchPatternType {
    use acadrust::entities::HatchPatternType as P;
    match pattern_type {
        P::UserDefined => HatchPatternType::UserDefined,
        P::Predefined => HatchPatternType::Predefined,
        P::Custom => HatchPatternType::Custom,
    }
}

/// Where a HATCH's pattern-definition rows come from.
///
/// The classification lives in one place so [`import_pattern`] and the
/// provenance diagnostic in [`crate::entity`] can never disagree about whether
/// the drawn rows were the file's or ours.
pub(crate) enum PatternRowSource<'a> {
    /// `78 > 0`: the file carries rows. They win **verbatim** — even when they
    /// disagree with the built-in table (see [`import_pattern`]).
    File,
    /// `76=0` with no rows and a usable spacing: one family synthesized from
    /// angle/spacing/double ([`import_pattern`]).
    UserDefinedSynthesis,
    /// `76=1` with no rows and a name the built-in table implements.
    Table(&'static cad_domain::hatch::PredefinedPattern),
    /// `76=1` with no rows and a name outside the implemented table: nothing is
    /// fabricated, the hatch stays `Partial`.
    Unresolved(&'a str),
    /// Nothing to resolve: `76=2` (custom) with no rows, or a `76=0` hatch
    /// whose spacing is unusable.
    Nothing,
}

/// Classify where this hatch's rows come from (see [`PatternRowSource`]).
///
/// The name lookup is [`cad_domain::hatch::predefined_pattern`], which already
/// trims and compares case-insensitively; no second normalisation happens here.
/// A `76=2` (`Custom`) name is **never** resolved: a custom pattern is defined
/// by the file, so substituting table rows would be a guess.
pub(crate) fn pattern_row_source(h: &Hatch) -> PatternRowSource<'_> {
    use acadrust::entities::HatchPatternType as AcPatternType;
    if !h.pattern.lines.is_empty() {
        return PatternRowSource::File;
    }
    match h.pattern_type {
        AcPatternType::UserDefined if h.pattern_scale.is_finite() && h.pattern_scale > 0.0 => {
            PatternRowSource::UserDefinedSynthesis
        }
        AcPatternType::Predefined => match cad_domain::hatch::predefined_pattern(&h.pattern.name) {
            Some(table) => PatternRowSource::Table(table),
            None => PatternRowSource::Unresolved(h.pattern.name.as_str()),
        },
        _ => PatternRowSource::Nothing,
    }
}

/// Translate the pattern definition, including the row-resolution rules.
///
/// **File rows always win.** A hatch whose `78 > 0` definition rows are replayed
/// as-is, verbatim, even when they disagree with the built-in predefined table
/// and even for `76=0` (ADR U-11: the priority is unverified, and this keeps the
/// committed `flange.dxf` result). There is no diagnostic and no rewriting in
/// that case — the file is the definition.
///
/// **User-defined synthesis.** A `76=0` hatch does not have to carry definition
/// rows: the file may store only angle (`52`), spacing (`41`) and double (`77`).
/// When `78 == 0` this synthesizes one family at offset `(0, 1)` in pattern
/// space, so the hatch-level scale is exactly the spacing; the double flag adds
/// the perpendicular family in [`HatchPattern::families`]. The generated
/// geometry is **our implementation** (ADR U-1/N4), not a documented AutoCAD
/// formula.
///
/// **Predefined table rows.** A `76=1` hatch whose name resolves in
/// [`cad_domain::hatch::predefined_pattern`] and whose file carries no rows is
/// imported with that table's rows (rotated into the hatch-plane frame by
/// [`cad_domain::hatch::PredefinedPattern::to_hatch_pattern`]) and the file's
/// own scale (`41`)/angle (`52`)/double (`77`). Those rows are **our hand
/// transcription (ADR U-1), not vendor-verified values**, so the caller records
/// a provenance diagnostic (see [`crate::entity`]); the name is kept exactly as
/// the file wrote it, so a name round-trip is not rewritten. A name outside the
/// implemented table (`ANSI33`, a vendor pattern) keeps today's honest result:
/// no rows at all, and a `Partial` reason that names the unresolved pattern.
pub(crate) fn import_pattern(h: &Hatch) -> HatchPattern {
    let lines: Vec<HatchPatternLine> = match pattern_row_source(h) {
        PatternRowSource::File => h
            .pattern
            .lines
            .iter()
            .map(|line| unevaluated_pattern_row(line, h.pattern_angle, h.pattern_scale))
            .collect(),
        PatternRowSource::UserDefinedSynthesis => vec![HatchPatternLine {
            angle: 0.0,
            base_x: 0.0,
            base_y: 0.0,
            offset_x: 0.0,
            offset_y: 1.0,
            dashes: Vec::new(),
        }],
        // Only the rows are taken from the table: the definition angle is
        // degrees there and radians here, and the name/scale/double of the
        // hatch stay exactly what the file carried.
        PatternRowSource::Table(table) => {
            table
                .to_hatch_pattern(h.pattern_scale, h.pattern_angle.to_degrees(), h.is_double)
                .lines
        }
        PatternRowSource::Unresolved(_) | PatternRowSource::Nothing => Vec::new(),
    };
    HatchPattern {
        name: h.pattern.name.clone(),
        angle: h.pattern_angle,
        scale: h.pattern_scale,
        double: h.is_double,
        lines,
    }
}

/// Translate one stored pattern row into the domain's **pattern-space** row.
///
/// A file's pattern-definition rows are stored **already evaluated**: the hatch
/// record carries `41` (scale) and `52` (angle) *and* rows whose base points are
/// rotated by that angle and whose offsets and dash lengths are multiplied by
/// that scale. Measured on a real AC1018 file (ADR 0018 §B.7): the same
/// pattern row reads `base = (5.656, 0), angle = 45°` at hatch angle 0 and
/// `base = (0, 5.656), angle = 135°` at hatch angle 90°, with dash lengths
/// `1.6 = 0.2 × 8` at scale 8 — the angle is baked into both the row angle and
/// the base point, and the scale into both the offset and the dashes.
///
/// Our domain model stores pattern-space rows and re-applies angle/scale in
/// [`HatchPattern::families`](cad_domain::HatchPattern::families) (ADR A.1), so
/// copying the stored rows verbatim applies both **twice**: a measured ZBM22
/// hatch drew every 64 units at 225° instead of every 8 units at 135°, i.e.
/// almost no pattern lines at all. This undoes exactly that composition, so
/// `families()` reproduces the file's rows and export writes them back
/// unchanged.
fn unevaluated_pattern_row(
    line: &acadrust::entities::HatchPatternLine,
    angle: f64,
    scale: f64,
) -> HatchPatternLine {
    // A zero or non-finite scale cannot be divided out; the rows are then taken
    // as the pattern-space definition (nothing is fabricated beyond the identity).
    let scale = if scale.is_finite() && scale.abs() > HATCH_EPS {
        scale
    } else {
        1.0
    };
    let (sin, cos) = angle.sin_cos();
    let unrotate = |x: f64, y: f64| [(cos * x + sin * y) / scale, (-sin * x + cos * y) / scale];
    let base = unrotate(line.base_point.x, line.base_point.y);
    let offset = unrotate(line.offset.x, line.offset.y);
    HatchPatternLine {
        angle: line.angle - angle,
        base_x: base[0],
        base_y: base[1],
        offset_x: offset[0],
        offset_y: offset[1],
        dashes: line.dash_lengths.iter().map(|d| d / scale).collect(),
    }
}

/// Translate the gradient block into the domain model.
///
/// `None` means no gradient is enabled. Every other gradient is stored verbatim
/// (including a kind this build cannot bake), so the definition round-trips; the
/// display builder reports the unsupported kind and draws the boundary only.
pub(crate) fn import_gradient(h: &Hatch) -> Option<HatchGradient> {
    let g = &h.gradient_color;
    if !g.is_enabled() {
        return None;
    }
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
    let name = g.name.clone();
    let reserved = g.reserved;
    let angle_raw = g.angle;
    let shift = g.shift;
    Some(if g.is_single_color {
        HatchGradient::SingleColor {
            name,
            reserved,
            angle_raw,
            shift,
            tint: g.color_tint,
            stops,
        }
    } else {
        HatchGradient::TwoColor {
            name,
            reserved,
            angle_raw,
            shift,
            stops,
        }
    })
}

/// Keep the analytic source edges of a boundary path for export (ADR B4).
///
/// Returns `None` when the path is a DXF polyline edge (or carries any
/// non-analytic edge): the loop's sampled points/bulges are then the only
/// boundary representation, and an export degrades to a sampled polyline path.
/// This loss is declared, never silent.
pub(crate) fn import_source_edges(
    path: &acadrust::entities::BoundaryPath,
) -> Option<Vec<cad_domain::hatch_entity::BoundaryEdge>> {
    use acadrust::entities::BoundaryEdge as AcEdge;
    use cad_domain::hatch_entity::BoundaryEdge as Edge;
    let mut out = Vec::new();
    for edge in &path.edges {
        out.push(match edge {
            AcEdge::Line(l) => Edge::Line {
                start: [l.start.x, l.start.y],
                end: [l.end.x, l.end.y],
            },
            AcEdge::CircularArc(a) => Edge::CircularArc {
                center: [a.center.x, a.center.y],
                radius: a.radius,
                start_angle: a.start_angle,
                end_angle: a.end_angle,
                ccw: a.counter_clockwise,
            },
            AcEdge::EllipticArc(a) => Edge::EllipticArc {
                center: [a.center.x, a.center.y],
                major_axis: [a.major_axis_endpoint.x, a.major_axis_endpoint.y],
                ratio: a.minor_axis_ratio,
                start_angle: a.start_angle,
                end_angle: a.end_angle,
                ccw: a.counter_clockwise,
            },
            AcEdge::Spline(s) => Edge::Spline {
                degree: s.degree.max(0) as u32,
                rational: s.rational,
                periodic: s.periodic,
                knots: s.knots.clone(),
                control_points: s.control_points.iter().map(|p| [p.x, p.y, p.z]).collect(),
                fit_points: s.fit_points.iter().map(|p| [p.x, p.y]).collect(),
                start_tangent: [s.start_tangent.x, s.start_tangent.y],
                end_tangent: [s.end_tangent.x, s.end_tangent.y],
            },
            AcEdge::Polyline(_) => return None,
        });
    }
    (!out.is_empty()).then_some(out)
}
