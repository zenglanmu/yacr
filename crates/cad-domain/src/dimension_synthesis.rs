//! Dimension display synthesis: `display = f(definition, style)`.
//!
//! Split out of `dimension.rs` to keep that file under the file-size limit.
//! The synthesis was moved verbatim from the former
//! `cad-import-acadrust::dimension` module, and the numbers it produces are
//! unchanged; the local vector algebra exists because `cad-domain` cannot
//! depend on `cad-geometry` (which depends on this crate).

use crate::dimension::{DimensionDefinition, DimensionKind, DimensionPoints, DimensionStyleValues};
use crate::{CadError, CadResult, Mesh, Point3, SemanticGeometry};

// ---------------------------------------------------------------------------
// Local vector algebra (cad-domain cannot depend on cad-geometry).
// ---------------------------------------------------------------------------

pub(crate) fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

pub(crate) fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

pub(crate) fn scale(a: Point3, s: f64) -> Point3 {
    Point3 {
        x: a.x * s,
        y: a.y * s,
        z: a.z * s,
    }
}

pub(crate) fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub(crate) fn length(a: Point3) -> f64 {
    a.x.hypot(a.y).hypot(a.z)
}

pub(crate) fn distance(a: Point3, b: Point3) -> f64 {
    length(sub(a, b))
}

pub(crate) fn normalize(a: Point3) -> Point3 {
    let len = length(a);
    if len < 1e-12 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    } else {
        scale(a, 1.0 / len)
    }
}

pub(crate) fn is_finite_point(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

// ---------------------------------------------------------------------------
// Synthesis (moved verbatim from cad-import-acadrust::dimension).
// ---------------------------------------------------------------------------

/// Geometry derived from one dimension, before the text primitive is attached.
///
/// Named `SynthesizedDimension` to keep it distinct from the entity type
/// [`DimensionGeometry`] (ADR 0017 A.7).
#[derive(Debug)]
pub struct SynthesizedDimension {
    pub children: Vec<SemanticGeometry>,
    pub measurement: f64,
    /// Default text anchor when the entity has no explicit middle point.
    pub text_position: Point3,
    /// Default text rotation when the entity has no explicit rotation.
    pub text_rotation: f64,
}

pub fn perpendicular(v: Point3) -> Point3 {
    Point3 {
        x: -v.y,
        y: v.x,
        z: 0.0,
    }
}

pub fn project_onto_line(origin: Point3, direction: Point3, point: Point3) -> Point3 {
    let t = dot(sub(point, origin), direction);
    add(origin, scale(direction, t))
}

/// A non-degenerate segment as a `Line`, or `None` when the endpoints coincide.
pub fn line_between(start: Point3, end: Point3) -> Option<SemanticGeometry> {
    (distance(start, end) > 1e-9).then_some(SemanticGeometry::Line { start, end })
}

/// A solid arrowhead: `tip` is the point, `outward` the direction it points.
pub fn arrowhead(tip: Point3, outward: Point3, size: f64) -> SemanticGeometry {
    let outward = normalize(outward);
    let base_center = sub(tip, scale(outward, size));
    let side = scale(perpendicular(outward), size / 6.0);
    let corner_a = add(base_center, side);
    let corner_b = sub(base_center, side);
    SemanticGeometry::Mesh(Mesh {
        vertices: vec![tip, corner_a, corner_b],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    })
}

/// Rotate a text angle into the readable range `(-90°, 90°]`.
pub fn readable_angle(mut angle: f64) -> f64 {
    use std::f64::consts::PI;
    while angle > PI / 2.0 {
        angle -= PI;
    }
    while angle <= -PI / 2.0 {
        angle += PI;
    }
    angle
}

/// Build the linear/aligned dimension geometry (no text).
pub fn linear_synthesis(
    first: Point3,
    second: Point3,
    definition: Point3,
    direction: Point3,
    style: &DimensionStyleValues,
) -> Option<SynthesizedDimension> {
    if !is_finite_point(first)
        || !is_finite_point(second)
        || !is_finite_point(definition)
        || !is_finite_point(direction)
        || length(direction) < 1e-9
    {
        return None;
    }
    let direction = normalize(direction);
    let normal = perpendicular(direction);
    let q1 = project_onto_line(definition, direction, first);
    let q2 = project_onto_line(definition, direction, second);
    let mut children = Vec::new();
    // DIMSE1/DIMSE2 suppress the first/second extension line. The first line is
    // the one from `first` (the first defining point); "suppress" removes the
    // whole extension line, not just part of it.
    for (index, (point, foot)) in [(first, q1), (second, q2)].into_iter().enumerate() {
        let suppressed = match index {
            0 => style.suppress_ext1,
            _ => style.suppress_ext2,
        };
        if suppressed {
            continue;
        }
        let v = sub(foot, point);
        let len = length(v);
        if len > 1e-9 {
            let u = scale(v, 1.0 / len);
            let start = add(point, scale(u, style.ext_offset));
            let end = add(foot, scale(u, style.ext_beyond));
            if let Some(line) = line_between(start, end) {
                children.push(line);
            }
        }
    }
    let mid = scale(add(q1, q2), 0.5);
    // DIMSD1/DIMSD2 suppress the first/second dimension-line segment and its
    // arrow. With neither suppressed the line is one segment (unchanged from
    // P5a); with one suppressed only the other half is drawn, split at the text
    // midpoint (an approximation of AutoCAD's text-gap break, declared).
    match (style.suppress_dim1, style.suppress_dim2) {
        (false, false) => {
            if let Some(line) = line_between(q1, q2) {
                children.push(line);
            }
        }
        (true, false) => {
            if let Some(line) = line_between(mid, q2) {
                children.push(line);
            }
        }
        (false, true) => {
            if let Some(line) = line_between(q1, mid) {
                children.push(line);
            }
        }
        (true, true) => {}
    }
    if style.arrow_size > 1e-9 {
        if !style.suppress_dim1 {
            children.push(arrowhead(q1, scale(direction, -1.0), style.arrow_size));
        }
        if !style.suppress_dim2 {
            children.push(arrowhead(q2, direction, style.arrow_size));
        }
    }
    // Text sits on the side the dimension line is offset toward. DIMTAD picks
    // the vertical placement and DIMTVP the signed offset used when it is 0.
    let measured_mid = scale(add(first, second), 0.5);
    let mut side = dot(sub(measured_mid, mid), normal);
    if side.abs() < 1e-9 {
        side = 1.0;
    } else {
        side = side.signum();
    }
    let text_position = add(mid, scale(normal, text_offset(style, side)));
    let text_rotation = readable_angle(direction.y.atan2(direction.x));
    let measurement = dot(sub(second, first), direction).abs();
    Some(SynthesizedDimension {
        children,
        measurement,
        text_position,
        text_rotation,
    })
}

/// The signed offset of the measurement text from the dimension line, along the
/// dimension line's normal (`+` is the text's "up"), for `DIMTAD`/`DIMTVP`.
///
/// `side` is `+1` when the measured geometry is on the `+normal` side of the
/// dimension line and `-1` otherwise; it is what `DIMTAD = 2` (far side) flips.
///
/// - `0` centred: `DIMTVP * text_height` (signed by `DIMTVP`; `0` sits on the
///   line);
/// - `1` above: `DIMGAP + text_height/2` above the line;
/// - `2` outside (far side): the same distance but on the side away from the
///   measured points;
/// - `3` JIS: the vertical placement equals `1`; the JIS-specific horizontal
///   placement and text-fit rules are **not** modelled (declared);
/// - `4` below: the same distance below the line;
/// - any other code: treated as `1` (declared).
fn text_offset(style: &DimensionStyleValues, side: f64) -> f64 {
    let above = style.text_gap + style.text_height * 0.5;
    match style.text_vertical {
        0 => style.text_vertical_position * style.text_height,
        2 => -side * above,
        // 3 (JIS): the vertical placement equals "above"; the JIS-specific
        // horizontal placement and text-fit rules are not modelled (declared).
        3 => above,
        4 => -above,
        // 1 (above) and any unknown code.
        _ => above,
    }
}

/// Build a radius/diameter leader with arrow(s) (no text).
///
/// `from` is the DXF group-10 slot — the circle centre for a radius dimension,
/// the **opposite chord point** for a diameter dimension (ADR 0017 A.3) — and
/// `to` is group 15, a point on the circle. `center` is the true circle centre:
/// the `DIMCEN` mark is drawn there and the centre-line reach is measured from
/// it. For a radius dimension `center == from`; for a diameter dimension it is
/// the midpoint of `from..to`.
pub fn radial_synthesis(
    from: Point3,
    to: Point3,
    center: Point3,
    two_arrows: bool,
    style: &DimensionStyleValues,
) -> Option<SynthesizedDimension> {
    if !is_finite_point(from) || !is_finite_point(to) || !is_finite_point(center) {
        return None;
    }
    let v = sub(to, from);
    let len = length(v);
    if len < 1e-9 {
        return None;
    }
    let u = scale(v, 1.0 / len);
    let mut children = Vec::new();
    if let Some(line) = line_between(from, to) {
        children.push(line);
    }
    // DIMCEN: a centre mark (> 0) or centre line (< 0) at the circle centre,
    // aligned with the dimension's own axes. The cross arms are `|DIMCEN|` long;
    // a centre line extends to `radius + |DIMCEN|` so it reaches past the circle.
    // The reach is relative to the **radius**, not the leader length, so a
    // diameter dimension (whose leader is the full diameter) uses half of it.
    if style.center_mark.is_finite() && style.center_mark != 0.0 {
        let arm = style.center_mark.abs();
        let radius = distance(center, to);
        let reach = if style.center_mark > 0.0 {
            arm
        } else {
            radius + arm
        };
        let cross = perpendicular(u);
        for axis in [u, cross] {
            if let Some(line) = line_between(
                sub(center, scale(axis, reach)),
                add(center, scale(axis, reach)),
            ) {
                children.push(line);
            }
        }
    }
    if style.arrow_size > 1e-9 {
        children.push(arrowhead(to, u, style.arrow_size));
        if two_arrows {
            children.push(arrowhead(from, scale(u, -1.0), style.arrow_size));
        }
    }
    Some(SynthesizedDimension {
        children,
        measurement: len,
        text_position: scale(add(from, to), 0.5),
        text_rotation: readable_angle(u.y.atan2(u.x)),
    })
}

/// Angular dimension (three-point form): an arc between the two rays
/// `vertex -> point1` and `vertex -> point2`, with extension lines, tangential
/// arrows and the angle text.
///
/// The **arc location** (`arc_point`, DXF group 10 for a three-point angular
/// dimension) selects which of the two angles the two rays bound is dimensioned
/// (ADR 0017 U-1: Autodesk does not document the exact rule, so this is our
/// declared reading): the sector that contains `arc_point` is measured, so a
/// point on the reflex side yields the reflex angle. A selector exactly on one
/// of the rays is treated as belonging to the counter-clockwise sector from that
/// ray (the same boundary convention as [`angular_lines_synthesis`]).
pub fn angular_synthesis(
    vertex: Point3,
    point1: Point3,
    point2: Point3,
    arc_point: Point3,
    normal: Point3,
    style: &DimensionStyleValues,
) -> Option<SynthesizedDimension> {
    angular_synthesis_rays(
        vertex,
        sub(point1, vertex),
        sub(point2, vertex),
        point1,
        point2,
        arc_point,
        normal,
        style,
    )
}

/// Angular dimension between two **lines** (two-line form, ADR 0017 U-8).
///
/// Line 1 is `line1_start -> line1_end` (DXF groups 13/14), line 2 is
/// `line2_start -> line2_end` (groups 15/10). The vertex is the **intersection
/// of the two infinite lines**, and `arc_point` (group 16) selects which of the
/// four angles is measured. This reading is pinned against the committed QCAD
/// fixtures (`entities.dxf` `*D65`, `example01.dxf` `*D6`, …): the group-42
/// radians and the anonymous block's ARC centre both match it, and acadrust's
/// own reader/`measurement_radians` implementation agrees.
///
/// Returns `None` for a degenerate line, parallel/collinear lines (no vertex), or
/// an `arc_point` at the vertex (no sector direction).
#[allow(clippy::too_many_arguments)]
pub fn angular_lines_synthesis(
    line1_start: Point3,
    line1_end: Point3,
    line2_start: Point3,
    line2_end: Point3,
    arc_point: Point3,
    normal: Point3,
    style: &DimensionStyleValues,
) -> Option<SynthesizedDimension> {
    if !is_finite_point(line1_start)
        || !is_finite_point(line1_end)
        || !is_finite_point(line2_start)
        || !is_finite_point(line2_end)
        || !is_finite_point(arc_point)
    {
        return None;
    }
    let d1 = sub(line1_end, line1_start);
    let d2 = sub(line2_end, line2_start);
    if length(d1) < 1e-9 || length(d2) < 1e-9 {
        return None;
    }
    let vertex = line_intersection(line1_start, d1, line2_start, d2)?;
    if !is_finite_point(vertex) {
        return None;
    }
    let selector = sub(arc_point, vertex);
    if length(selector) < 1e-9 {
        return None;
    }
    let u1 = normalize(d1);
    let u2 = normalize(d2);
    let selector_angle = selector.y.atan2(selector.x);
    // Each line contributes two opposite boundary rays. The four angle sectors
    // are the counter-clockwise sweeps of at most PI from one boundary ray to the
    // other line's boundary ray; the sector containing the arc location is the one
    // dimensioned. A sector may start on either line, so both orders are tried. A
    // location on a boundary belongs to the sector that starts at it (the same
    // convention the three-point form uses).
    let rays1 = [u1, scale(u1, -1.0)];
    let rays2 = [u2, scale(u2, -1.0)];
    let mut chosen = None;
    for (start_is_line1, starts, ends) in [(true, &rays1, &rays2), (false, &rays2, &rays1)] {
        for start_ray in starts {
            for end_ray in ends {
                let sweep =
                    wrap_positive(end_ray.y.atan2(end_ray.x) - start_ray.y.atan2(start_ray.x));
                if sweep > std::f64::consts::PI + 1e-9 {
                    continue;
                }
                let delta = wrap_positive(selector_angle - start_ray.y.atan2(start_ray.x));
                if delta + 1e-12 < sweep || delta <= 1e-12 {
                    chosen = Some((*start_ray, *end_ray, start_is_line1));
                    break;
                }
            }
            if chosen.is_some() {
                break;
            }
        }
        if chosen.is_some() {
            break;
        }
    }
    let (ray1, ray2, start_is_line1) = chosen?;
    // The extension lines start at the endpoint of each picked segment that lies
    // furthest along the bounding ray; the segment itself may stop short of the
    // vertex, which AutoCAD also extends.
    let (ext1, ext2) = if start_is_line1 {
        (
            outward_endpoint(line1_start, line1_end, vertex, ray1),
            outward_endpoint(line2_start, line2_end, vertex, ray2),
        )
    } else {
        (
            outward_endpoint(line2_start, line2_end, vertex, ray1),
            outward_endpoint(line1_start, line1_end, vertex, ray2),
        )
    };
    angular_synthesis_rays(vertex, ray1, ray2, ext1, ext2, arc_point, normal, style)
}

/// The shared angular arc builder: `ray1`/`ray2` are the directions of the two
/// bounding rays (from `vertex`), `ext1`/`ext2` the points the extension lines
/// start at, and `arc_point` selects the sector.
#[allow(clippy::too_many_arguments)]
fn angular_synthesis_rays(
    vertex: Point3,
    ray1: Point3,
    ray2: Point3,
    ext1: Point3,
    ext2: Point3,
    arc_point: Point3,
    normal: Point3,
    style: &DimensionStyleValues,
) -> Option<SynthesizedDimension> {
    if !is_finite_point(vertex)
        || !is_finite_point(ext1)
        || !is_finite_point(ext2)
        || !is_finite_point(arc_point)
    {
        return None;
    }
    if length(ray1) < 1e-9 || length(ray2) < 1e-9 {
        return None;
    }
    let selector = sub(arc_point, vertex);
    let r = length(selector);
    if !r.is_finite() || r < 1e-9 {
        return None;
    }
    let u1 = normalize(ray1);
    let u2 = normalize(ray2);
    let a1 = u1.y.atan2(u1.x);
    let a2 = u2.y.atan2(u2.x);
    let sweep_ccw = wrap_positive(a2 - a1);
    if sweep_ccw < 1e-9 || std::f64::consts::TAU - sweep_ccw < 1e-9 {
        return None;
    }
    // Which of the two sectors (counter-clockwise or reflex) contains the arc
    // location. The selector on a boundary picks the counter-clockwise sector.
    let selected = wrap_positive(selector.y.atan2(selector.x) - a1);
    let sweep = if selected <= sweep_ccw + 1e-12 {
        sweep_ccw
    } else {
        sweep_ccw - std::f64::consts::TAU
    };
    let dir_at = |angle: f64| Point3 {
        x: angle.cos(),
        y: angle.sin(),
        z: 0.0,
    };
    let end1 = add(vertex, scale(u1, r));
    let end2 = add(vertex, scale(dir_at(a1 + sweep), r));
    let mut children = vec![SemanticGeometry::Arc {
        center: vertex,
        normal,
        radius: r,
        start: a1,
        sweep,
    }];
    if let Some(line) = line_between(ext1, end1) {
        children.push(line);
    }
    if let Some(line) = line_between(ext2, end2) {
        children.push(line);
    }
    if style.arrow_size > 1e-9 {
        // Tangential arrowheads at both arc ends.
        let sign = if sweep >= 0.0 { -1.0 } else { 1.0 };
        let t1 = Point3 {
            x: -u1.y,
            y: u1.x,
            z: 0.0,
        };
        let t2 = Point3 {
            x: -dir_at(a1 + sweep).y,
            y: dir_at(a1 + sweep).x,
            z: 0.0,
        };
        children.push(arrowhead(end1, scale(t1, sign), style.arrow_size));
        children.push(arrowhead(end2, scale(t2, -sign), style.arrow_size));
    }
    let mid = dir_at(a1 + sweep / 2.0);
    let text_position = add(
        vertex,
        scale(mid, r + style.text_gap + style.text_height * 0.5),
    );
    Some(SynthesizedDimension {
        children,
        measurement: sweep.abs().to_degrees(),
        text_position,
        text_rotation: readable_angle(a1 + sweep / 2.0),
    })
}

/// Normalize an angle to `[0, 2π)`.
fn wrap_positive(angle: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut wrapped = angle % tau;
    if wrapped < 0.0 {
        wrapped += tau;
    }
    if wrapped >= tau {
        wrapped -= tau;
    }
    wrapped
}

/// Intersection of two infinite XY lines through `o1`/`o2` with directions
/// `d1`/`d2`; `None` for parallel/collinear directions.
fn line_intersection(o1: Point3, d1: Point3, o2: Point3, d2: Point3) -> Option<Point3> {
    let cross = d1.x * d2.y - d1.y * d2.x;
    let scale = length(d1) * length(d2);
    if !cross.is_finite() || cross.abs() <= scale * 1e-12 {
        return None;
    }
    let offset = sub(o2, o1);
    let t = (offset.x * d2.y - offset.y * d2.x) / cross;
    Some(Point3 {
        x: o1.x + d1.x * t,
        y: o1.y + d1.y * t,
        z: o1.z,
    })
}

/// The endpoint of `start..end` furthest along `direction` from `vertex`.
fn outward_endpoint(start: Point3, end: Point3, vertex: Point3, direction: Point3) -> Point3 {
    let first = dot(sub(start, vertex), direction);
    let second = dot(sub(end, vertex), direction);
    if second >= first {
        end
    } else {
        start
    }
}

/// Ordinate dimension: a leader from the feature point to the dimension line,
/// with the feature's **datum-relative** coordinate as the measurement.
///
/// The measurement is `(feature - base)` projected on the X or Y axis (per
/// `is_x`), matching the documented AutoCAD semantics and acadrust's writer,
/// which recomputes group 42 as the datum-relative projection
/// (`acadrust .../writer/section_writer.rs` `write_dimension_ordinate` →
/// `entities/dimension.rs` `DimensionOrdinate::measurement`; review B2).
///
/// The magnitude is taken here because the shared [`format_measurement`] and the
/// acadrust writer both use the magnitude; keeping `measurement()` at the
/// magnitude makes `measurement()`, the drawn text and the written group 42
/// agree for every input (a signed-coordinate display is not modelled).
pub fn ordinate_synthesis(
    feature: Point3,
    leader: Point3,
    base: Point3,
    is_x: bool,
) -> Option<SynthesizedDimension> {
    if !is_finite_point(feature) || !is_finite_point(leader) {
        return None;
    }
    let mut children = Vec::new();
    if let Some(line) = line_between(feature, leader) {
        children.push(line);
    }
    let delta = sub(feature, base);
    let projected = if is_x { delta.x } else { delta.y };
    Some(SynthesizedDimension {
        children,
        measurement: projected.abs(),
        text_position: leader,
        text_rotation: 0.0,
    })
}

/// Format a measurement with `DIMRND` rounding, `DIMDEC` decimals, `DIMZIN`
/// leading/trailing zero suppression and the `DIMDSEP` decimal separator.
///
/// `DIMPOST` prefix/suffix is applied by [`resolve_measurement_text`], not here,
/// because a user text override substitutes the fully-posted value into `<>`.
pub fn format_measurement(value: f64, style: &DimensionStyleValues) -> String {
    // DIMRND: round the displayed distance to the nearest multiple (`0`
    // disables it). Group 42 keeps the unrounded measurement.
    let rounded = if style.rounding.is_finite() && style.rounding > 0.0 {
        (value / style.rounding).round() * style.rounding
    } else {
        value
    };
    let mut text = format!("{:.*}", style.decimals, rounded.abs());
    // DIMZIN bit 3: suppress trailing zeros.
    if style.suppress_trailing_zeros() && text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    // DIMZIN bit 2: suppress the leading zero (`0.5` -> `.5`).
    if style.suppress_leading_zeros() {
        if let Some(rest) = text.strip_prefix("0.") {
            text = format!(".{rest}");
        } else if let Some(rest) = text.strip_prefix("-0.") {
            text = format!("-.{rest}");
        }
    }
    // DIMDSEP: replace the decimal point with the configured separator (46 is
    // '.'). A code that is not a character leaves the point in place.
    if style.decimal_separator != 46 && style.decimal_separator != 0 {
        if let Some(separator) = char::from_u32(style.decimal_separator.max(0) as u32) {
            if separator != '.' {
                text = text.replace('.', &separator.to_string());
            }
        }
    }
    text
}

/// Apply `DIMPOST`: the template with `<>` replaced by the formatted value.
///
/// An empty template (or the literal `<>`) renders the bare measurement. A
/// template without a `<>` placeholder is treated as a suffix (our documented
/// reading of a case the variable page does not spell out).
fn apply_postfix(value: &str, postfix: &str) -> String {
    if postfix.is_empty() || postfix == "<>" {
        value.to_string()
    } else if postfix.contains("<>") {
        postfix.replace("<>", value)
    } else {
        format!("{value}{postfix}")
    }
}

/// Resolve the displayed text: a user override with `<>` substituted, or the
/// subtype's default prefix/suffix around the `DIMPOST`-wrapped measurement.
pub fn resolve_measurement_text(
    text_override: Option<&str>,
    prefix: &str,
    suffix: &str,
    measurement: f64,
    style: &DimensionStyleValues,
) -> String {
    let formatted = format_measurement(measurement * style.linear_factor, style);
    let value = apply_postfix(&formatted, &style.postfix);
    match text_override {
        Some(template) => {
            if template.contains("<>") {
                template.replace("<>", &value)
            } else {
                template.to_string()
            }
        }
        None => format!("{prefix}{value}{suffix}"),
    }
}

/// The subtype's default measurement prefix/suffix (`R`, `⌀`, `°`).
pub(crate) fn kind_affix(kind: DimensionKind) -> (&'static str, &'static str) {
    match kind {
        DimensionKind::Radius => ("R", ""),
        DimensionKind::Diameter => ("\u{2300}", ""),
        DimensionKind::Angular2Ln | DimensionKind::Angular3Pt => ("", "\u{00B0}"),
        _ => ("", ""),
    }
}

/// Build the subtype geometry from a definition and style.
///
/// Fails with [`CadError::InvalidInput`] when the subtype and its point slots
/// disagree, a point is non-finite, or the geometry is degenerate.
pub(crate) fn synthesize(
    definition: &DimensionDefinition,
    style: &DimensionStyleValues,
) -> CadResult<SynthesizedDimension> {
    if !definition.is_consistent() {
        return Err(CadError::InvalidInput(
            "dimension definition is inconsistent (subtype/point mismatch or non-finite point)"
                .to_string(),
        ));
    }
    let synthesized = match (definition.kind, &definition.points) {
        (
            DimensionKind::Linear { rotation },
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            },
        ) => linear_synthesis(
            *first,
            *second,
            *dimension_line,
            Point3 {
                x: rotation.cos(),
                y: rotation.sin(),
                z: 0.0,
            },
            style,
        ),
        (
            DimensionKind::Aligned,
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            },
        ) => {
            let direction = normalize(sub(*second, *first));
            linear_synthesis(*first, *second, *dimension_line, direction, style)
        }
        (
            DimensionKind::Radius,
            DimensionPoints::Radial {
                center,
                point_on_arc,
                ..
            },
        ) => radial_synthesis(*center, *point_on_arc, *center, false, style),
        (
            DimensionKind::Diameter,
            DimensionPoints::Radial {
                center,
                point_on_arc,
                ..
            },
        ) => radial_synthesis(
            // Group 10 is the opposite chord point; the true centre (and thus the
            // DIMCEN mark centre) is the midpoint of the two chord points (B1).
            *center,
            *point_on_arc,
            scale(add(*center, *point_on_arc), 0.5),
            true,
            style,
        ),
        (
            DimensionKind::Angular2Ln,
            DimensionPoints::Angular2Ln {
                line1_start,
                line1_end,
                line2_start,
                line2_end,
                arc_point,
            },
        ) => angular_lines_synthesis(
            // U-8 resolved in P5e: line 1 is groups 13->14, line 2 is 15->10, the
            // vertex is the intersection of the two infinite lines, and group 16
            // selects the measured sector. Pinned against the committed QCAD
            // angular fixtures (see `angular_lines_synthesis`).
            *line1_start,
            *line1_end,
            *line2_start,
            *line2_end,
            *arc_point,
            definition.normal,
            style,
        ),
        (
            DimensionKind::Angular3Pt,
            DimensionPoints::Angular3Pt {
                vertex,
                first,
                second,
                arc_point,
            },
        ) => angular_synthesis(
            *vertex,
            *first,
            *second,
            *arc_point,
            definition.normal,
            style,
        ),
        (
            DimensionKind::Ordinate { x_ordinate },
            DimensionPoints::Ordinate {
                feature,
                leader_end,
                base,
            },
        ) => ordinate_synthesis(*feature, *leader_end, *base, x_ordinate),
        _ => None,
    };
    synthesized.ok_or_else(|| CadError::InvalidInput("dimension points are degenerate".to_string()))
}

/// Recompute a dimension's raw measurement from its definition alone.
pub(crate) fn compute_measurement(
    definition: &DimensionDefinition,
    style: &DimensionStyleValues,
) -> CadResult<f64> {
    Ok(synthesize(definition, style)?.measurement)
}
