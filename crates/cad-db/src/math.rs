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

pub(crate) fn scale(a: Point3, s: f64) -> Point3 {
    Point3 {
        x: a.x * s,
        y: a.y * s,
        z: a.z * s,
    }
}

pub(crate) fn length(v: Point3) -> f64 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
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

/// Unit vector in the direction of `v`; returns `v` unchanged when it is too
/// short to normalise (callers must have rejected degenerate input first).
pub(crate) fn normalize(v: Point3) -> Point3 {
    let l = length(v);
    if l < 1e-24 {
        v
    } else {
        scale(v, 1.0 / l)
    }
}

/// Apply only the linear part of `t` to a direction/vector (no translation).
pub(crate) fn apply_vector(t: &Transform3, v: Point3) -> Point3 {
    let m = &t.matrix;
    Point3 {
        x: m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        y: m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        z: m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    }
}

/// Whether every entry of the 4×4 linear+translation matrix is finite.
pub(crate) fn transform_is_finite(t: &Transform3) -> bool {
    t.matrix.iter().flatten().all(|c| c.is_finite())
}

/// The AutoCAD arbitrary-axis algorithm: an orthonormal frame `(ax, ay, normal)`
/// for the plane whose normal is `normal`. Shared by the geometry
/// validation/transform code so angular parameters use one convention.
pub(crate) fn arbitrary_axis(normal: Point3) -> (Point3, Point3, Point3) {
    let n = if length(normal) < 1e-24 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    } else {
        normalize(normal)
    };
    const ONE_64TH: f64 = 1.0 / 64.0;
    let ax = if n.x.abs() < ONE_64TH && n.y.abs() < ONE_64TH {
        cross(
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            n,
        )
    } else {
        cross(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            n,
        )
    };
    let ax = if length(ax) < 1e-24 {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    } else {
        normalize(ax)
    };
    let ay = normalize(cross(n, ax));
    (ax, ay, n)
}

/// Unit minor-axis direction for an ellipse with plane normal `normal` and unit
/// major axis `major_unit`: the in-plane `+90°` rotation `cross(normal, major)`.
pub(crate) fn ellipse_minor_dir(normal: Point3, major_unit: Point3) -> Point3 {
    let n = normalize(normal);
    let m = normalize(major_unit);
    let minor = cross(n, m);
    if length(minor) >= 1e-9 {
        return normalize(minor);
    }
    let fallback = cross(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        m,
    );
    if length(fallback) >= 1e-9 {
        normalize(fallback)
    } else {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    }
}

/// The in-plane angle (radians) of a point relative to `center` in the frame
/// `(u, v)`. Used to recompute an Arc/Ellipse start parameter after a
/// positive-determinant similarity.
pub(crate) fn plane_angle(point: Point3, center: Point3, u: Point3, v: Point3) -> f64 {
    let d = sub(point, center);
    dot(d, v).atan2(dot(d, u))
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
