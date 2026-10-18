use super::*;

/// The near/far planes for a perspective view at `distance`, in world units.
///
/// This is the single source of truth shared by [`Camera::projection_matrix`] and
/// [`Camera::camera3d_params`]: a small fraction of the distance for `near` and a
/// large multiple for `far`, with finite positive floors so the target always
/// stays well inside `[near, far]`.
pub(super) fn perspective_near_far(distance: f64) -> (f64, f64) {
    let near = (distance * 1e-4).max(1e-6);
    let far = (distance * 1e4).max(1.0);
    (near, far)
}

pub(super) fn orthographic_matrix(half_w: f64, half_h: f64, near: f64, far: f64) -> [[f64; 4]; 4] {
    // Column-major; maps x∈[-half_w,half_w], y∈[-half_h,half_h], z∈[near,far]
    // to clip x,y∈[-1,1], z∈[0,1] (WebGPU).
    [
        [1.0 / half_w, 0.0, 0.0, 0.0],
        [0.0, 1.0 / half_h, 0.0, 0.0],
        [0.0, 0.0, 1.0 / (far - near), 0.0],
        [0.0, 0.0, -near / (far - near), 1.0],
    ]
}

pub(super) fn perspective_matrix(
    vertical_fov_radians: f64,
    aspect: f64,
    near: f64,
    far: f64,
) -> CadResult<[[f64; 4]; 4]> {
    if !aspect.is_finite() || aspect <= 0.0 {
        return Err(CadError::InvalidInput(
            "perspective aspect ratio must be positive and finite".into(),
        ));
    }
    if !near.is_finite() || !far.is_finite() || near <= 0.0 || far <= near {
        return Err(CadError::InvalidInput(
            "perspective near/far must be finite with 0 < near < far".into(),
        ));
    }
    let f = 1.0 / (vertical_fov_radians * 0.5).tan();
    let nf = 1.0 / (near - far);
    Ok([
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far * nf, -1.0],
        [0.0, 0.0, near * far * nf, 0.0],
    ])
}

/// Whether a screen-space `[f64; 2]` is finite.
pub(super) fn finite2(v: [f64; 2]) -> bool {
    v[0].is_finite() && v[1].is_finite()
}

// --- small vector helpers (cad-domain Point3 doubles as a vector) ---

/// Squared-free 3D dot product.
pub fn dot3(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// 3D cross product.
pub fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

/// 3D Euclidean length.
pub fn length3(v: Point3) -> f64 {
    dot3(v, v).sqrt()
}

/// Normalise a vector, returning `None` for a non-finite or zero vector.
pub fn normalize3(v: Point3) -> Option<Point3> {
    let l = length3(v);
    if !l.is_finite() || l < 1e-300 {
        return None;
    }
    Some(scale3(v, 1.0 / l))
}

/// Scale a vector.
pub fn scale3(v: Point3, s: f64) -> Point3 {
    Point3 {
        x: v.x * s,
        y: v.y * s,
        z: v.z * s,
    }
}

/// Add two vectors.
pub fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

/// Subtract `b` from `a`.
pub fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

pub(super) fn rotate_about_z(v: Point3, angle: f64) -> Point3 {
    rotate_about_axis(
        v,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        angle,
    )
}

/// Rodrigues rotation of `v` about a unit `axis` by `angle` radians.
pub(super) fn rotate_about_axis(v: Point3, axis: Point3, angle: f64) -> Point3 {
    let axis = match normalize3(axis) {
        Some(a) => a,
        None => return v,
    };
    let (s, c) = angle.sin_cos();
    let k = dot3(axis, v) * (1.0 - c);
    let cr = cross(axis, v);
    Point3 {
        x: v.x * c + cr.x * s + axis.x * k,
        y: v.y * c + cr.y * s + axis.y * k,
        z: v.z * c + cr.z * s + axis.z * k,
    }
}
