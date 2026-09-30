//! OCS (object coordinate system) to WCS transforms (spec v2.0 §3.2, §7.1).

use glam::DVec3;

/// The AutoCAD arbitrary axis algorithm.
///
/// Returns `(ax, ay, az)` with `az` the normalised normal.
pub fn arbitrary_axis(normal: DVec3) -> (DVec3, DVec3, DVec3) {
    let n = if normal.length_squared() < 1e-24 { DVec3::Z } else { normal.normalize() };
    const ONE_64TH: f64 = 1.0 / 64.0;
    let ax = if n.x.abs() < ONE_64TH && n.y.abs() < ONE_64TH {
        DVec3::Y.cross(n)
    } else {
        DVec3::Z.cross(n)
    };
    let ax = if ax.length_squared() < 1e-24 { DVec3::X } else { ax.normalize() };
    let ay = n.cross(ax).normalize();
    (ax, ay, n)
}

/// Alias used by the geometry API.
pub fn ocs_to_wcs(normal: DVec3) -> (DVec3, DVec3, DVec3) {
    arbitrary_axis(normal)
}

/// Transform an OCS point relative to an entity origin into WCS.
pub fn ocs_point_to_wcs(normal: DVec3, origin: DVec3, p: DVec3) -> DVec3 {
    let (ax, ay, az) = arbitrary_axis(normal);
    origin + ax * p.x + ay * p.y + az * p.z
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_z_is_identity_basis() {
        let (ax, ay, az) = arbitrary_axis(DVec3::Z);
        assert!((ax - DVec3::X).length() < 1e-9);
        assert!((ay - DVec3::Y).length() < 1e-9);
        assert!((az - DVec3::Z).length() < 1e-9);
    }

    #[test]
    fn basis_is_orthonormal() {
        for n in [DVec3::new(0.3, 0.4, 0.5), DVec3::new(0.0, 0.01, 1.0), DVec3::new(1.0, 1.0, 0.0)] {
            let (ax, ay, az) = arbitrary_axis(n);
            assert!((ax.length() - 1.0).abs() < 1e-9);
            assert!((ay.length() - 1.0).abs() < 1e-9);
            assert!(ax.dot(ay).abs() < 1e-9);
            assert!(ax.dot(az).abs() < 1e-9);
        }
    }
}
