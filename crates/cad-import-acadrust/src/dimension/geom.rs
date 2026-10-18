//! Shared geometry primitives for dimension synthesis.
//!
//! Submodule of [`crate::dimension`].

use super::*;

/// Geometry derived from one dimension, before the text primitive is attached.
#[derive(Debug)]
pub(super) struct DimensionGeometry {
    pub(super) children: Vec<SemanticGeometry>,
    pub(super) measurement: f64,
    /// Default text anchor when the entity has no explicit middle point.
    pub(super) text_position: Point3,
    /// Default text rotation when the entity has no explicit rotation.
    pub(super) text_rotation: f64,
}

pub(super) fn perpendicular(v: Point3) -> Point3 {
    Point3 {
        x: -v.y,
        y: v.x,
        z: 0.0,
    }
}

pub(super) fn project_onto_line(origin: Point3, direction: Point3, point: Point3) -> Point3 {
    let t = cad_geometry::dot(cad_geometry::sub(point, origin), direction);
    cad_geometry::add(origin, cad_geometry::scale(direction, t))
}

pub(crate) fn line_between(start: Point3, end: Point3) -> Option<SemanticGeometry> {
    (cad_geometry::distance(start, end) > 1e-9).then_some(SemanticGeometry::Line { start, end })
}

/// A solid arrowhead: `tip` is the point, `outward` the direction it points.
pub(super) fn arrowhead(tip: Point3, outward: Point3, size: f64) -> SemanticGeometry {
    let outward = cad_geometry::normalize(outward);
    let base_center = cad_geometry::sub(tip, cad_geometry::scale(outward, size));
    let side = cad_geometry::scale(perpendicular(outward), size / 6.0);
    let corner_a = cad_geometry::add(base_center, side);
    let corner_b = cad_geometry::sub(base_center, side);
    SemanticGeometry::Mesh(Mesh {
        vertices: vec![tip, corner_a, corner_b],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    })
}

/// Rotate a text angle into the readable range `(-90°, 90°]`.
pub(super) fn readable_angle(mut angle: f64) -> f64 {
    use std::f64::consts::PI;
    while angle > PI / 2.0 {
        angle -= PI;
    }
    while angle <= -PI / 2.0 {
        angle += PI;
    }
    angle
}
