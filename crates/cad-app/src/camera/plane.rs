use super::*;

/// Whether a point has only finite coordinates.
pub fn is_finite_point(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

/// A right-handed orthonormal work plane used by planar measurement.
///
/// The plane is `origin + u*a + v*b` with a **right-handed** basis: the outward
/// normal `n = normalize(u × v)` points toward the viewer in the 2D top view
/// (i.e. `+Z`). `u` and `v` are unit vectors. Planar measurement
/// (`cad-measure`) consumes this exact `WorkPlane`, so the app must supply a
/// non-degenerate, orthogonal, unit-scaled basis (audit B24).
pub fn orthonormal_work_plane(origin: Point3, u: Point3, v: Point3) -> CadResult<WorkPlane> {
    let u = normalize3(u).ok_or_else(|| {
        CadError::InvalidInput("work plane u must be a finite non-zero vector".into())
    })?;
    let v = normalize3(v).ok_or_else(|| {
        CadError::InvalidInput("work plane v must be a finite non-zero vector".into())
    })?;
    let n = cross(u, v);
    if length3(n) < 1e-9 {
        return Err(CadError::InvalidInput(
            "work plane basis is degenerate (u and v are parallel)".into(),
        ));
    }
    // Re-orthogonalise v against u so the normalised basis is exactly orthogonal.
    let n_unit = normalize3(n).expect("checked non-degenerate");
    let v_ortho = normalize3(cross(n_unit, u)).expect("cross of orthonormal pair");
    if !is_finite_point(origin) {
        return Err(CadError::InvalidInput(
            "work plane origin is not finite".into(),
        ));
    }
    Ok(WorkPlane {
        origin,
        u,
        v: v_ortho,
    })
}

/// The default XY work plane at `z = 0` with a right-handed `u = +X`, `v = +Y`.
pub fn xy_work_plane(z: f64) -> WorkPlane {
    orthonormal_work_plane(
        Point3 { x: 0.0, y: 0.0, z },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
    )
    .expect("the XY basis is always valid")
}
