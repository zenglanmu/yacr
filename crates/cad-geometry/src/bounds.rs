//! bounds module.

use super::*;

/// Accumulates an axis-aligned bounding box.
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
        if !is_finite(p) {
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
        for p in tessellate_geometry(
            geometry,
            TessellationParams {
                tolerance: 0.1,
                max_segments: 256,
                min_segments: 4,
            },
        ) {
            self.add_point(p);
        }
        if let SemanticGeometry::Mesh(m) = geometry {
            for p in &m.vertices {
                self.add_point(*p);
            }
        }
    }

    pub fn union_point(&mut self, p: Point3) {
        self.add_point(p);
    }

    pub fn finish(&self) -> Option<Bounds3> {
        if self.any {
            Some(Bounds3 {
                min: self.min,
                max: self.max,
            })
        } else {
            None
        }
    }
}

// ---- Point3 helpers (kept local so no external math crate leaks) ----
