//! Axis-aligned bounds accumulation over semantic geometry.

use cad_domain::*;

use crate::math::{length, transform_scale};

/// Maximum INSERT nesting the database will expand while computing bounds.
pub const MAX_INSTANCE_DEPTH: usize = 32;

/// Accumulates an axis-aligned box over geometry.
pub struct BoundsAccumulator {
    min: Point3,
    max: Point3,
    any: bool,
}

impl Default for BoundsAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl BoundsAccumulator {
    pub fn new() -> Self {
        BoundsAccumulator {
            min: Point3 {
                x: f64::INFINITY,
                y: f64::INFINITY,
                z: f64::INFINITY,
            },
            max: Point3 {
                x: f64::NEG_INFINITY,
                y: f64::NEG_INFINITY,
                z: f64::NEG_INFINITY,
            },
            any: false,
        }
    }

    pub fn add_point(&mut self, p: Point3) {
        if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
            return;
        }
        self.min = Point3 {
            x: self.min.x.min(p.x),
            y: self.min.y.min(p.y),
            z: self.min.z.min(p.z),
        };
        self.max = Point3 {
            x: self.max.x.max(p.x),
            y: self.max.y.max(p.y),
            z: self.max.z.max(p.z),
        };
        self.any = true;
    }

    pub fn add_geometry(&mut self, geometry: &SemanticGeometry) {
        self.add_geometry_transformed(geometry, &Transform3::identity());
    }

    /// Accumulate bounds of `geometry` after applying `transform`.
    ///
    /// Curved bounds are expanded conservatively by the transform's largest
    /// scale so a rotated/scaled instance still fits the box.
    pub fn add_geometry_transformed(
        &mut self,
        geometry: &SemanticGeometry,
        transform: &Transform3,
    ) {
        match geometry {
            SemanticGeometry::Line { start, end } => {
                self.add_point(transform.apply_point(*start));
                self.add_point(transform.apply_point(*end));
            }
            SemanticGeometry::Polyline { points, .. } => {
                for p in points {
                    self.add_point(transform.apply_point(*p));
                }
            }
            SemanticGeometry::Circle { center, radius, .. }
            | SemanticGeometry::Arc { center, radius, .. } => {
                self.add_sphere_transformed(*center, *radius, transform);
            }
            SemanticGeometry::Ellipse {
                center,
                major_axis,
                ratio,
                ..
            } => {
                let r = length(*major_axis).max(length(*major_axis) * ratio.abs());
                self.add_sphere_transformed(*center, r, transform);
            }
            SemanticGeometry::Spline { control_points, .. } => {
                for p in control_points {
                    self.add_point(transform.apply_point(*p));
                }
            }
            SemanticGeometry::Point(p) => self.add_point(transform.apply_point(*p)),
            SemanticGeometry::Mesh(mesh) => {
                for p in &mesh.vertices {
                    self.add_point(transform.apply_point(*p));
                }
            }
            // Inserts are expanded by `DrawingDatabase::bounds`, which owns the
            // block definitions; a bare insert contributes no bounds here.
            SemanticGeometry::Insert { .. } => {}
            SemanticGeometry::Text {
                position,
                height,
                text,
                ..
            } => {
                let p = transform.apply_point(*position);
                let h = height.abs().max(1e-9) * transform_scale(transform);
                self.add_point(p);
                self.add_point(Point3 {
                    x: p.x + h * text.chars().count() as f64,
                    y: p.y + h,
                    z: p.z,
                });
            }
            SemanticGeometry::Shape { position, size, .. } => {
                let r = size.abs().max(1e-9) * transform_scale(transform);
                self.add_sphere_transformed(*position, r, transform);
            }
            SemanticGeometry::Opaque { .. } => {}
            SemanticGeometry::Compound(children) => {
                for child in children {
                    self.add_geometry_transformed(child, transform);
                }
            }
        }
    }

    fn add_sphere_transformed(&mut self, center: Point3, radius: f64, transform: &Transform3) {
        let c = transform.apply_point(center);
        let r = radius.abs() * transform_scale(transform);
        self.add_point(Point3 {
            x: c.x - r,
            y: c.y - r,
            z: c.z - r,
        });
        self.add_point(Point3 {
            x: c.x + r,
            y: c.y + r,
            z: c.z + r,
        });
    }

    pub fn finish(&self) -> Option<(Point3, Point3)> {
        if self.any {
            Some((self.min, self.max))
        } else {
            None
        }
    }
}
