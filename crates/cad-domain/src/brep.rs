//! Neutral B-rep data model: placement, analytic surfaces, curves, loops,
//! faces, shells and the per-body container.
//!
//! Every type here is plain data crossing the kernel seam: no ACIS handle, no
//! SAT/SAB parser and no CAD-package type appears. It lives in `cad-domain`
//! (rather than in `cad-kernel-adapter`) because a body's exact geometry is
//! domain data, not a kernel-implementation detail; the adapter owns only the
//! evaluation and tessellation seams that consume it.

use crate::{Point3, Transform3};

/// Unit vector in the direction of `p`; a vanished vector normalises to zero
/// rather than to NaN.
fn normalize(p: Point3) -> Point3 {
    let length = (p.x * p.x + p.y * p.y + p.z * p.z).sqrt();
    if length > 1e-12 {
        Point3 {
            x: p.x / length,
            y: p.y / length,
            z: p.z / length,
        }
    } else {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }
}

/// A body placement in the ACIS convention `world = scale·(p·M) + t`.
///
/// `matrix` is row-major exactly as `SatDocument::placement` returns it, so the
/// same numbers survive the seam unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrepPlacement {
    pub matrix: [[f64; 3]; 3],
    pub translation: Point3,
    pub scale: f64,
}

impl BrepPlacement {
    pub fn identity() -> Self {
        BrepPlacement {
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            scale: 1.0,
        }
    }

    /// Whether this placement is the identity (no transform work needed).
    pub fn is_identity(&self) -> bool {
        self.scale == 1.0
            && self.translation.x == 0.0
            && self.translation.y == 0.0
            && self.translation.z == 0.0
            && self.matrix == [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }

    /// Apply the placement to a point (ACIS row-vector convention).
    pub fn apply(&self, p: Point3) -> Point3 {
        let m = &self.matrix;
        let t = self.translation;
        let s = self.scale;
        Point3 {
            x: s * (p.x * m[0][0] + p.y * m[1][0] + p.z * m[2][0]) + t.x,
            y: s * (p.x * m[0][1] + p.y * m[1][1] + p.z * m[2][1]) + t.y,
            z: s * (p.x * m[0][2] + p.y * m[1][2] + p.z * m[2][2]) + t.z,
        }
    }

    /// Rotate a direction by the placement's linear part and renormalise.
    pub fn apply_direction(&self, d: Point3) -> Point3 {
        let m = &self.matrix;
        normalize(Point3 {
            x: d.x * m[0][0] + d.y * m[1][0] + d.z * m[2][0],
            y: d.x * m[0][1] + d.y * m[1][1] + d.z * m[2][1],
            z: d.x * m[0][2] + d.y * m[1][2] + d.z * m[2][2],
        })
    }
}

/// The analytic supporting surface of a face.
#[derive(Debug, Clone, PartialEq)]
pub enum BrepSurface {
    /// A plane through `origin` with `normal`; `u_dir` fixes the in-plane frame.
    Plane {
        origin: Point3,
        normal: Point3,
        u_dir: Point3,
    },
    /// A sphere; loop-less faces cover the full parameter domain.
    Sphere {
        center: Point3,
        radius: f64,
        u_dir: Point3,
        pole: Point3,
    },
    /// A torus whose axis is `normal`; loop-less faces cover the full domain.
    Torus {
        center: Point3,
        normal: Point3,
        major_radius: f64,
        minor_radius: f64,
        u_dir: Point3,
    },
    /// A circular cylinder. Side faces are bounded by two full circles.
    Cylinder {
        origin: Point3,
        axis: Point3,
        ref_dir: Point3,
        radius: f64,
    },
    /// A circular cone (or truncated cone). `radius` is the reference
    /// cross-section radius at `origin`; the cross-section radius grows with
    /// axial distance at `slope = sin_half_angle / cos_half_angle`. A side face
    /// is bounded by one full circle plus a degenerate apex loop (full cone) or
    /// by two full circles (frustum).
    Cone {
        origin: Point3,
        axis: Point3,
        ref_dir: Point3,
        radius: f64,
        sin_half_angle: f64,
        cos_half_angle: f64,
    },
    /// A surface this build cannot evaluate. Reported as a missing face.
    Unsupported { type_key: String },
}

/// A curve segment of a loop, already oriented in loop direction.
#[derive(Debug, Clone, PartialEq)]
pub enum BrepCurve {
    Line {
        start: Point3,
        end: Point3,
    },
    /// A full circle. `u_dir` is the radius direction at parameter zero.
    Circle {
        center: Point3,
        normal: Point3,
        u_dir: Point3,
        radius: f64,
    },
    /// A curve this build cannot evaluate; the owning face is unsupported.
    Unsupported {
        type_key: String,
    },
}

/// A connected, closed loop of curves in loop order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrepLoop {
    pub edges: Vec<BrepCurve>,
}

/// A face: a supporting surface plus its boundary loops.
#[derive(Debug, Clone, PartialEq)]
pub struct BrepFace {
    /// Source face record index (or a stable sequential id). Reported on failure.
    pub id: u32,
    pub surface: BrepSurface,
    /// `true` when the face sense is reversed relative to the surface normal.
    pub reversed: bool,
    /// Boundary loops; outer vs. inner is derived geometrically at tessellation.
    pub loops: Vec<BrepLoop>,
}

/// A connected shell of faces.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrepShell {
    pub id: u32,
    pub faces: Vec<BrepFace>,
}

/// Neutral B-rep for one body.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrepData {
    pub shells: Vec<BrepShell>,
    pub placement: Option<BrepPlacement>,
}

impl BrepData {
    /// Total face count across all shells.
    pub fn face_count(&self) -> usize {
        self.shells.iter().map(|s| s.faces.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.face_count() == 0
    }
}

impl BrepPlacement {
    /// Compose this placement with `transform`: a model point is mapped by
    /// `self` and then by `transform` (`world' = transform(world)`).
    ///
    /// The result is only geometrically meaningful when `transform` is a
    /// similarity (uniform scale, rotation and/or reflection, translation);
    /// callers must check [`Transform3::is_uniform_scale`] first. A
    /// non-similarity would fold a shear into `matrix`, which this placement
    /// does not represent.
    pub fn composed_with(&self, transform: &Transform3) -> BrepPlacement {
        let m = &transform.matrix;
        let column = |i: usize| (m[0][i] * m[0][i] + m[1][i] * m[1][i] + m[2][i] * m[2][i]).sqrt();
        // Mean column length; only read after `is_uniform_scale` accepted the
        // matrix, so the three columns agree.
        let scale = ((column(0) + column(1) + column(2)) / 3.0).max(1e-12);
        let mut matrix = [[0.0f64; 3]; 3];
        for (row, out_row) in matrix.iter_mut().enumerate() {
            for (out_column, cell) in out_row.iter_mut().enumerate() {
                // `R = L / scale`, so `M'[row][col] = Σ M[row][k]·R[col][k]`.
                *cell = (0..3)
                    .map(|k| self.matrix[row][k] * (m[out_column][k] / scale))
                    .sum();
            }
        }
        let rotated = transform.apply_vector(self.translation);
        BrepPlacement {
            matrix,
            translation: Point3 {
                x: rotated.x + m[0][3],
                y: rotated.y + m[1][3],
                z: rotated.z + m[2][3],
            },
            scale: self.scale * scale,
        }
    }
}
