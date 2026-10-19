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
    for (point, foot) in [(first, q1), (second, q2)] {
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
    if let Some(line) = line_between(q1, q2) {
        children.push(line);
    }
    if style.arrow_size > 1e-9 {
        children.push(arrowhead(q1, scale(direction, -1.0), style.arrow_size));
        children.push(arrowhead(q2, direction, style.arrow_size));
    }
    // Text sits on the side the dimension line is offset toward, `DIMGAP` plus
    // half the text height away from the line (matches QCAD/AutoCAD default).
    let mid = scale(add(q1, q2), 0.5);
    let measured_mid = scale(add(first, second), 0.5);
    let mut side = dot(sub(measured_mid, mid), normal);
    if side.abs() < 1e-9 {
        side = 1.0;
    } else {
        side = side.signum();
    }
    let offset = if style.text_above {
        style.text_gap + style.text_height * 0.5
    } else {
        0.0
    };
    let text_position = add(mid, scale(normal, side * offset));
    let text_rotation = readable_angle(direction.y.atan2(direction.x));
    let measurement = dot(sub(second, first), direction).abs();
    Some(SynthesizedDimension {
        children,
        measurement,
        text_position,
        text_rotation,
    })
}

/// Build a radius/diameter leader with arrow(s) (no text).
pub fn radial_synthesis(
    from: Point3,
    to: Point3,
    two_arrows: bool,
    style: &DimensionStyleValues,
) -> Option<SynthesizedDimension> {
    if !is_finite_point(from) || !is_finite_point(to) {
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

/// Angular dimension: an arc between two rays from `vertex`, with extension
/// lines, tangential arrows and the angle text.
pub fn angular_synthesis(
    vertex: Point3,
    point1: Point3,
    point2: Point3,
    radius: f64,
    normal: Point3,
    style: &DimensionStyleValues,
) -> Option<SynthesizedDimension> {
    let d1 = sub(point1, vertex);
    let d2 = sub(point2, vertex);
    if length(d1) < 1e-9 || length(d2) < 1e-9 {
        return None;
    }
    let u1 = normalize(d1);
    let u2 = normalize(d2);
    let a1 = u1.y.atan2(u1.x);
    let a2 = u2.y.atan2(u2.x);
    let mut sweep = a2 - a1;
    use std::f64::consts::PI;
    while sweep > PI {
        sweep -= 2.0 * PI;
    }
    while sweep < -PI {
        sweep += 2.0 * PI;
    }
    let r = if radius.is_finite() && radius > 1e-9 {
        radius
    } else {
        1.0
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
    if let Some(line) = line_between(point1, end1) {
        children.push(line);
    }
    if let Some(line) = line_between(point2, end2) {
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

/// Format a measurement with `DIMDEC` decimals and `DIMZIN` bit-3 zero
/// suppression.
pub fn format_measurement(value: f64, style: &DimensionStyleValues) -> String {
    let mut text = format!("{:.*}", style.decimals, value.abs());
    if style.suppress_trailing_zeros && text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

/// Resolve the displayed text: a user override with `<>` substituted, or the
/// subtype's default prefix/suffix plus the formatted measurement.
pub fn resolve_measurement_text(
    text_override: Option<&str>,
    prefix: &str,
    suffix: &str,
    measurement: f64,
    style: &DimensionStyleValues,
) -> String {
    let value = format_measurement(measurement * style.linear_factor, style);
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
        ) => radial_synthesis(*center, *point_on_arc, false, style),
        (
            DimensionKind::Diameter,
            DimensionPoints::Radial {
                center,
                point_on_arc,
                ..
            },
        ) => radial_synthesis(*center, *point_on_arc, true, style),
        (
            DimensionKind::Angular2Ln,
            DimensionPoints::Angular2Ln {
                line1_start,
                line1_end: _,
                line2_start,
                line2_end,
                arc_point,
            },
        ) => angular_synthesis(
            // Matches the pre-P5a import: the vertex is line 2's start (`15`),
            // the two rays are `13` and `10`. Group `14` is retained in the
            // definition but not consumed (ADR 0017 U-8; resolved in P5e).
            *line2_start,
            *line1_start,
            *line2_end,
            distance(*line2_start, *arc_point),
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
            distance(*vertex, *arc_point),
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
