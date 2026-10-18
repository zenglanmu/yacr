use super::*;

/// Squared Euclidean distance between two world-space points.
///
/// Squared distance is monotonic in distance, so it orders identically while
/// avoiding the square root.
pub fn squared_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

pub(super) fn mat4_mul(a: &[[f64; 4]; 4], b: &[[f64; 4]; 4]) -> [[f32; 4]; 4] {
    // Column-major (WGSL): (a*b)[col][row] = sum_k a[k][row] * b[col][k].
    let mut out = [[0.0f32; 4]; 4];
    for (col, out_col) in out.iter_mut().enumerate() {
        for (row, cell) in out_col.iter_mut().enumerate() {
            let mut sum = 0.0f64;
            for k in 0..4 {
                sum += a[k][row] * b[col][k];
            }
            *cell = sum as f32;
        }
    }
    out
}

pub(super) fn vector_is_usable(v: [f64; 3]) -> bool {
    v.iter().all(|c| c.is_finite()) && (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) > 1e-24
}

pub(super) fn normalize3(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = ((v[0] as f64).powi(2) + (v[1] as f64).powi(2) + (v[2] as f64).powi(2)).sqrt();
    if !len.is_finite() || len < 1e-12 {
        return None;
    }
    Some([
        (v[0] as f64 / len) as f32,
        (v[1] as f64 / len) as f32,
        (v[2] as f64 / len) as f32,
    ])
}

pub(super) fn point_is_finite(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

pub(super) fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

pub(super) fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

pub(super) fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub(super) fn length(a: Point3) -> f64 {
    dot(a, a).sqrt()
}

pub(super) fn normalize(a: Point3) -> Point3 {
    let l = length(a);
    if l <= 0.0 {
        a
    } else {
        Point3 {
            x: a.x / l,
            y: a.y / l,
            z: a.z / l,
        }
    }
}
