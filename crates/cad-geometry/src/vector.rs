//! vector module.

use super::*;

pub fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

pub fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

pub fn scale(a: Point3, s: f64) -> Point3 {
    Point3 {
        x: a.x * s,
        y: a.y * s,
        z: a.z * s,
    }
}

pub fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

pub fn length(a: Point3) -> f64 {
    dot(a, a).sqrt()
}

pub fn distance(a: Point3, b: Point3) -> f64 {
    length(sub(a, b))
}

pub fn normalize(a: Point3) -> Point3 {
    let l = length(a);
    if l < 1e-24 {
        a
    } else {
        scale(a, 1.0 / l)
    }
}

pub fn is_finite(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

/// Whether every scalar and point in `geometry` is finite.
///
/// Used after a transform to refuse poisoned output (audit B12/B23).
pub fn geometry_is_finite(geometry: &SemanticGeometry) -> bool {
    use SemanticGeometry as G;
    let finite = |p: Point3| is_finite(p);
    let all_points = |pts: &[Point3]| pts.iter().all(|p| finite(*p));
    match geometry {
        G::Line { start, end } => finite(*start) && finite(*end),
        G::Polyline { points, bulges, .. } => {
            all_points(points) && bulges.iter().all(|b| b.is_finite())
        }
        G::Circle {
            center,
            normal,
            radius,
        } => finite(*center) && finite(*normal) && radius.is_finite(),
        G::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            finite(*center)
                && finite(*normal)
                && radius.is_finite()
                && start.is_finite()
                && sweep.is_finite()
        }
        G::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            finite(*center)
                && finite(*normal)
                && finite(*major_axis)
                && ratio.is_finite()
                && start.is_finite()
                && sweep.is_finite()
        }
        G::Spline {
            knots,
            control_points,
            weights,
            ..
        } => {
            all_points(control_points)
                && knots.iter().all(|k| k.is_finite())
                && weights.iter().all(|w| w.is_finite())
        }
        G::Point(p) => finite(*p),
        G::Mesh(m) => {
            all_points(&m.vertices)
                && m.normals.iter().all(|n| finite(*n))
                && m.triangles
                    .iter()
                    .all(|t| t.iter().all(|i| (*i as usize) < m.vertices.len()))
        }
        G::Insert { transform, .. } => transform.matrix.iter().flatten().all(|v| v.is_finite()),
        G::Text {
            position,
            height,
            rotation,
            ..
        } => finite(*position) && height.is_finite() && rotation.is_finite(),
        G::Shape {
            position,
            size,
            rotation,
            ..
        } => finite(*position) && size.is_finite() && rotation.is_finite(),
        G::Opaque { .. } => true,
        G::Compound(children) => children.iter().all(geometry_is_finite),
    }
}
