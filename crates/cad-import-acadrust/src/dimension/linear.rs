//! Linear/aligned and radius/diameter dimension synthesis plus measurement-text
//! formatting.
//!
//! Submodule of [`crate::dimension`].

use super::*;
use acadrust::entities::DimensionBase;

/// Build the linear/aligned dimension geometry (no text).
pub(super) fn linear_geometry(
    first: Point3,
    second: Point3,
    definition: Point3,
    direction: Point3,
    style: &DimStyleValues,
) -> Option<DimensionGeometry> {
    if !cad_geometry::is_finite(first)
        || !cad_geometry::is_finite(second)
        || !cad_geometry::is_finite(definition)
        || !cad_geometry::is_finite(direction)
        || cad_geometry::length(direction) < 1e-9
    {
        return None;
    }
    let direction = cad_geometry::normalize(direction);
    let normal = perpendicular(direction);
    let q1 = project_onto_line(definition, direction, first);
    let q2 = project_onto_line(definition, direction, second);
    let mut children = Vec::new();
    for (point, foot) in [(first, q1), (second, q2)] {
        let v = cad_geometry::sub(foot, point);
        let len = cad_geometry::length(v);
        if len > 1e-9 {
            let u = cad_geometry::scale(v, 1.0 / len);
            let start = cad_geometry::add(point, cad_geometry::scale(u, style.ext_offset));
            let end = cad_geometry::add(foot, cad_geometry::scale(u, style.ext_beyond));
            if let Some(line) = line_between(start, end) {
                children.push(line);
            }
        }
    }
    if let Some(line) = line_between(q1, q2) {
        children.push(line);
    }
    if style.arrow_size > 1e-9 {
        children.push(arrowhead(
            q1,
            cad_geometry::scale(direction, -1.0),
            style.arrow_size,
        ));
        children.push(arrowhead(q2, direction, style.arrow_size));
    }
    // Text sits on the side the dimension line is offset toward, `DIMGAP` plus
    // half the text height away from the line (matches QCAD/AutoCAD default).
    let mid = cad_geometry::scale(cad_geometry::add(q1, q2), 0.5);
    let measured_mid = cad_geometry::scale(cad_geometry::add(first, second), 0.5);
    let mut side = cad_geometry::dot(cad_geometry::sub(measured_mid, mid), normal);
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
    let text_position = cad_geometry::add(mid, cad_geometry::scale(normal, side * offset));
    let text_rotation = readable_angle(direction.y.atan2(direction.x));
    let measurement = cad_geometry::dot(cad_geometry::sub(second, first), direction).abs();
    Some(DimensionGeometry {
        children,
        measurement,
        text_position,
        text_rotation,
    })
}

/// Build a radius/diameter leader with arrow(s) (no text).
pub(super) fn radial_geometry(
    from: Point3,
    to: Point3,
    two_arrows: bool,
    style: &DimStyleValues,
) -> Option<DimensionGeometry> {
    if !cad_geometry::is_finite(from) || !cad_geometry::is_finite(to) {
        return None;
    }
    let v = cad_geometry::sub(to, from);
    let len = cad_geometry::length(v);
    if len < 1e-9 {
        return None;
    }
    let u = cad_geometry::scale(v, 1.0 / len);
    let mut children = Vec::new();
    if let Some(line) = line_between(from, to) {
        children.push(line);
    }
    if style.arrow_size > 1e-9 {
        children.push(arrowhead(to, u, style.arrow_size));
        if two_arrows {
            children.push(arrowhead(
                from,
                cad_geometry::scale(u, -1.0),
                style.arrow_size,
            ));
        }
    }
    Some(DimensionGeometry {
        children,
        measurement: len,
        text_position: cad_geometry::scale(cad_geometry::add(from, to), 0.5),
        text_rotation: readable_angle(u.y.atan2(u.x)),
    })
}

pub(super) fn format_measurement(value: f64, style: &DimStyleValues) -> String {
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
pub(super) fn resolve_measurement_text(
    base: &DimensionBase,
    prefix: &str,
    suffix: &str,
    measurement: f64,
    style: &DimStyleValues,
) -> String {
    let value = format_measurement(measurement * style.linear_factor, style);
    match base.text_override() {
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
