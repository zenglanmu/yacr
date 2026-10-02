//! Small vector helpers shared by the database modules.

use cad_domain::*;

pub(crate) fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

pub(crate) fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

pub(crate) fn length(v: Point3) -> f64 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

pub(crate) fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// Conservative upper bound on a transform's linear scale (Frobenius norm of
/// the 3×3 part), used to grow circular bounds under scale/rotation.
pub(crate) fn transform_scale(t: &Transform3) -> f64 {
    let m = &t.matrix;
    let mut sum = 0.0;
    for row in m.iter().take(3) {
        for value in row.iter().take(3) {
            sum += value * value;
        }
    }
    sum.sqrt().max(1e-12)
}
