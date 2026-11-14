//! Plane and insertion transforms for DGN shared-cell references.
//!
//! The domain geometry is either world-space (DGN stores world coordinates) or
//! block-local; only a shared-cell instance needs a transform. The composition
//! mirrors the DWG adapter's `insert_array_transform`:
//! `OCS(normal) · T(position) · Rz(rotation) · S(scale) · T(-base)`.

use cad_domain::{Point3, Transform3};

/// Length of a vector.
pub(crate) fn length(v: Point3) -> f64 {
    v.x.hypot(v.y).hypot(v.z)
}

/// Cross product.
pub(crate) fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

/// A unit vector, or `None` for a zero-length / non-finite input.
pub(crate) fn normalize(v: Point3) -> Option<Point3> {
    let len = length(v);
    if !len.is_finite() || len < 1e-24 {
        return None;
    }
    Some(Point3 {
        x: v.x / len,
        y: v.y / len,
        z: v.z / len,
    })
}

/// The DGN plane normal, defaulting a degenerate value to world `+Z`.
pub(crate) fn normal_or_z(normal: Point3) -> Point3 {
    normalize(normal).unwrap_or(Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    })
}

/// Whether `normal` is world `+Z` within a tight tolerance.
pub(crate) fn is_world_z(normal: Point3) -> bool {
    let n = normal_or_z(normal);
    (n.x.abs() < 1e-9) && (n.y.abs() < 1e-9) && (n.z - 1.0).abs() < 1e-9
}

/// AutoCAD's arbitrary-axis algorithm, matching `cad-geometry`.
fn arbitrary_axis(normal: Point3) -> (Point3, Point3, Point3) {
    let n = normal_or_z(normal);
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
    let ax = normalize(ax).unwrap_or(Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    });
    let ay = normalize(cross(n, ax)).unwrap_or(Point3 {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    });
    (ax, ay, n)
}

/// Object-coordinate-system to world transform for `normal`.
fn ocs_to_wcs(normal: Point3) -> Transform3 {
    let (ax, ay, az) = arbitrary_axis(normal);
    Transform3 {
        matrix: [
            [ax.x, ay.x, az.x, 0.0],
            [ax.y, ay.y, az.y, 0.0],
            [ax.z, ay.z, az.z, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

fn rotation_z(angle: f64) -> Transform3 {
    let (s, c) = angle.sin_cos();
    Transform3 {
        matrix: [
            [c, -s, 0.0, 0.0],
            [s, c, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

fn scale_matrix(scale: Point3) -> Transform3 {
    Transform3 {
        matrix: [
            [scale.x, 0.0, 0.0, 0.0],
            [0.0, scale.y, 0.0, 0.0],
            [0.0, 0.0, scale.z, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

/// The block-space-to-world transform of one shared-cell instance.
///
/// `base` is the definition's insertion base point, so a block-space point `p`
/// lands at `insert(OCS · R · S · (p − base))`. A negative scale is kept (DGN
/// supports mirroring); the caller reports a mirrored `z`.
pub(crate) fn insert_transform(
    base: Point3,
    position: Point3,
    scale: Point3,
    rotation: f64,
    normal: Point3,
) -> Transform3 {
    let ocs = ocs_to_wcs(normal);
    let translate = Transform3::translation(position);
    let rotate = rotation_z(rotation);
    let scale = scale_matrix(scale);
    let base = Transform3::translation(Point3 {
        x: -base.x,
        y: -base.y,
        z: -base.z,
    });
    ocs.matrix_mul(&translate)
        .matrix_mul(&rotate)
        .matrix_mul(&scale)
        .matrix_mul(&base)
}

/// A conic sweep from `start` to `end`, in `(0, TAU]` (or `[-TAU, 0)` clockwise).
///
/// `cadkit` reports a full ellipse as `0..2π`; the domain stores a full sweep as
/// `TAU`, so an exact full turn is kept rather than reduced to zero.
pub(crate) fn conic_sweep(start: f64, end: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let raw = end - start;
    if !raw.is_finite() {
        return raw;
    }
    if raw.abs() >= tau - 1e-9 {
        tau.copysign(raw)
    } else {
        raw.rem_euclid(tau)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    #[test]
    fn insert_transform_applies_position_scale_and_base() {
        let transform = insert_transform(
            p(1.0, 0.0, 0.0),
            p(2.0, 3.0, 4.0),
            p(2.0, 2.0, 2.0),
            0.0,
            p(0.0, 0.0, 1.0),
        );
        // (base - base) -> position.
        assert_eq!(transform.apply_point(p(1.0, 0.0, 0.0)), p(2.0, 3.0, 4.0));
        // One unit past the base scales by 2 and then translates.
        assert_eq!(transform.apply_point(p(2.0, 0.0, 0.0)), p(4.0, 3.0, 4.0));
    }

    #[test]
    fn a_full_turn_stays_full() {
        let tau = std::f64::consts::TAU;
        assert!((conic_sweep(0.0, tau) - tau).abs() < 1e-12);
        assert!((conic_sweep(1.0, 1.0 + tau / 2.0) - tau / 2.0).abs() < 1e-12);
        assert!((conic_sweep(4.0, 2.0) - (2.0 - 4.0_f64).rem_euclid(tau)).abs() < 1e-12);
    }
}
