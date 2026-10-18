//! Small vector helpers and a right-handed frame builder (kept local so the
//! B-rep modules are self-contained).

use cad_domain::Point3;

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

pub(crate) fn mul(a: Point3, s: f64) -> Point3 {
    Point3 {
        x: a.x * s,
        y: a.y * s,
        z: a.z * s,
    }
}

pub(crate) fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub(crate) fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

pub(crate) fn length(a: Point3) -> f64 {
    dot(a, a).sqrt()
}

pub(crate) fn normalize(a: Point3) -> Point3 {
    let l = length(a);
    if l > 1e-12 {
        mul(a, 1.0 / l)
    } else {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }
}

pub(super) fn finite(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

/// Build a right-handed orthonormal frame `(x, y, z)` with `z = axis`.
pub(super) fn frame_from(axis: Point3, preferred_x: Point3) -> Option<(Point3, Point3, Point3)> {
    let z = normalize(axis);
    if length(z) < 0.5 {
        return None;
    }
    let mut x = sub(preferred_x, mul(z, dot(preferred_x, z)));
    if length(x) < 1e-9 {
        // Pick any axis not parallel to z.
        let alt = if z.x.abs() < 0.9 {
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            }
        } else {
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            }
        };
        x = sub(alt, mul(z, dot(alt, z)));
    }
    let x = normalize(x);
    if length(x) < 0.5 {
        return None;
    }
    let y = cross(z, x);
    Some((x, y, z))
}
