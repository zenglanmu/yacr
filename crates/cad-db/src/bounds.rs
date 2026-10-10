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
                let r = size.abs().max(1e-9);
                self.add_sphere_transformed(*position, r, transform);
            }
            SemanticGeometry::Opaque { .. } => {}
            // A raster image spans the rectangle from `origin` along `u` for
            // `pixels.0` pixels and along `v` for `pixels.1` pixels; accumulate
            // its four corners (transformed as points).
            SemanticGeometry::Image {
                origin,
                u,
                v,
                pixels,
                ..
            } => {
                let corner = |du: f64, dv: f64| Point3 {
                    x: origin.x + u.x * du + v.x * dv,
                    y: origin.y + u.y * du + v.y * dv,
                    z: origin.z + u.z * du + v.z * dv,
                };
                self.add_point(transform.apply_point(corner(0.0, 0.0)));
                self.add_point(transform.apply_point(corner(pixels[0], 0.0)));
                self.add_point(transform.apply_point(corner(0.0, pixels[1])));
                self.add_point(transform.apply_point(corner(pixels[0], pixels[1])));
            }
            // A mask is a closed world-space polygon: every boundary point is
            // part of its extent.
            SemanticGeometry::Mask { boundary, .. } => {
                for p in boundary {
                    self.add_point(transform.apply_point(*p));
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_bounds_apply_instance_scale_once() {
        let shape = SemanticGeometry::Shape {
            shape_name: "synthetic".to_string(),
            code: 1,
            position: Point3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            size: 4.0,
            rotation: 0.0,
            font: None,
        };
        for scale in [0.25, 2.0] {
            let mut transform = Transform3::identity();
            for axis in 0..3 {
                transform.matrix[axis][axis] = scale;
            }
            transform.matrix[0][3] = 10.0;
            let mut bounds = BoundsAccumulator::new();
            bounds.add_geometry_transformed(&shape, &transform);
            let (min, max) = bounds.finish().unwrap();
            let center = transform.apply_point(Point3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            });
            // The shared sphere helper uses the conservative Frobenius norm,
            // not the exact similarity scale.
            let radius = 4.0 * (3.0_f64).sqrt() * scale;
            assert_eq!(
                min,
                Point3 {
                    x: center.x - radius,
                    y: center.y - radius,
                    z: center.z - radius
                }
            );
            assert_eq!(
                max,
                Point3 {
                    x: center.x + radius,
                    y: center.y + radius,
                    z: center.z + radius
                }
            );
        }
    }

    #[test]
    fn image_bounds_include_all_four_corners() {
        // origin = (1,2,0); u = (0.5,0,0) per pixel, v = (0,0.25,0) per pixel;
        // 4x2 pixels -> corners (1,2), (3,2), (1,2.5), (3,2.5).
        let image = SemanticGeometry::Image {
            origin: Point3 {
                x: 1.0,
                y: 2.0,
                z: 0.0,
            },
            u: Point3 {
                x: 0.5,
                y: 0.0,
                z: 0.0,
            },
            v: Point3 {
                x: 0.0,
                y: 0.25,
                z: 0.0,
            },
            pixels: [4.0, 2.0],
            file: None,
            clip: None,
            visible: true,
        };
        let mut bounds = BoundsAccumulator::new();
        bounds.add_geometry(&image);
        let (min, max) = bounds.finish().unwrap();
        assert_eq!(min.x, 1.0);
        assert_eq!(min.y, 2.0);
        assert_eq!(max.x, 3.0);
        assert_eq!(max.y, 2.5);
    }

    #[test]
    fn mask_bounds_include_every_boundary_point() {
        let mask = SemanticGeometry::Mask {
            boundary: vec![
                Point3 {
                    x: -1.0,
                    y: 4.0,
                    z: 0.0,
                },
                Point3 {
                    x: 5.0,
                    y: 4.0,
                    z: 0.0,
                },
                Point3 {
                    x: 5.0,
                    y: -2.0,
                    z: 0.0,
                },
            ],
            inverted: false,
        };
        let mut bounds = BoundsAccumulator::new();
        bounds.add_geometry(&mask);
        let (min, max) = bounds.finish().unwrap();
        assert_eq!(min.x, -1.0);
        assert_eq!(min.y, -2.0);
        assert_eq!(max.x, 5.0);
        assert_eq!(max.y, 4.0);
    }
}
