//! transform module.

use super::*;

pub(crate) fn apply_point(t: &Transform3, p: Point3) -> Point3 {
    let m = &t.matrix;
    Point3 {
        x: m[0][0] * p.x + m[0][1] * p.y + m[0][2] * p.z + m[0][3],
        y: m[1][0] * p.x + m[1][1] * p.y + m[1][2] * p.z + m[1][3],
        z: m[2][0] * p.x + m[2][1] * p.y + m[2][2] * p.z + m[2][3],
    }
}

pub(crate) fn apply_vector(t: &Transform3, v: Point3) -> Point3 {
    let m = &t.matrix;
    Point3 {
        x: m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        y: m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        z: m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    }
}

pub(crate) fn uniform_scale(t: &Transform3) -> f64 {
    // Mean column length; used only after `is_uniform` has accepted the matrix.
    let m = &t.matrix;
    let sx = (m[0][0] * m[0][0] + m[1][0] * m[1][0] + m[2][0] * m[2][0]).sqrt();
    let sy = (m[0][1] * m[0][1] + m[1][1] * m[1][1] + m[2][1] * m[2][1]).sqrt();
    let sz = (m[0][2] * m[0][2] + m[1][2] * m[1][2] + m[2][2] * m[2][2]).sqrt();
    ((sx + sy + sz) / 3.0).max(1e-12)
}

/// Whether a transform keeps circles circular (similarity, no shear).
pub(crate) fn is_uniform(t: &Transform3) -> bool {
    t.is_uniform_scale(1e-9)
}

pub(crate) fn world_z_axis() -> Point3 {
    Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    }
}

/// A mirror (negative determinant) reverses the OCS handedness, so a traced
/// curve keeps its image only if its parameter range is reversed as well.
pub(crate) fn oriented_after_reflection(t: &Transform3, start: f64, sweep: f64) -> (f64, f64) {
    if t.determinant() < 0.0 {
        (-start, -sweep)
    } else {
        (start, sweep)
    }
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
    // The normal is parallel to the major axis: the ellipse is degenerate. Pick
    // any perpendicular so callers still get a usable frame.
    let fallback = cross(world_z_axis(), m);
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

/// Map a circle through an affine transform as an exact ellipse.
pub(crate) fn circle_to_geometry(
    center: Point3,
    normal: Point3,
    radius: f64,
    t: &Transform3,
) -> SemanticGeometry {
    let radius = radius.abs();
    if radius < 1e-12 {
        return SemanticGeometry::Point(apply_point(t, center));
    }
    let n = normalize(normal);
    let (ax, _, _) = arbitrary_axis(n);
    affine_ellipse_arc(
        center,
        n,
        scale(ax, radius),
        1.0,
        0.0,
        std::f64::consts::TAU,
        t,
    )
}

/// Map a circular arc through an affine transform as an exact elliptical arc.
pub(crate) fn arc_to_geometry(
    center: Point3,
    normal: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
    t: &Transform3,
) -> SemanticGeometry {
    let radius = radius.abs();
    if radius < 1e-12 {
        return SemanticGeometry::Point(apply_point(t, center));
    }
    let n = normalize(normal);
    let (ax, _, _) = arbitrary_axis(n);
    affine_ellipse_arc(center, n, scale(ax, radius), 1.0, start, sweep, t)
}

/// The exact affine image of an elliptical arc.
///
/// An affine map sends the parameterisation
/// `E(t) = c + u·cos t + v·sin t` to `c' + (A u)·cos t + (A v)·sin t`. Writing
/// `M = [A u, A v]`, the 2×2 eigen-decomposition of `MᵀM` yields the principal
/// axes (`s1 ≥ s2`), a rotation angle `θ` and an orthonormal frame `(w1, w2)`
/// with `A u·cos t + A v·sin t = s1 w1·cos(t−θ) + s2 w2·sin(t−θ)`. The result is
/// reported in the domain convention `minor = cross(normal, major)`, with the
/// parameter shifted by `−θ` (audit B23).
#[allow(clippy::too_many_arguments)]
pub(crate) fn affine_ellipse_arc(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
    t: &Transform3,
) -> SemanticGeometry {
    let a = length(major_axis);
    if a < 1e-12 {
        return SemanticGeometry::Point(apply_point(t, center));
    }
    let n = normalize(normal);
    let major_unit = scale(major_axis, 1.0 / a);
    let minor_vec = scale(ellipse_minor_dir(n, major_unit), a * ratio.abs().max(0.0));
    let u = apply_vector(t, major_axis);
    let v = apply_vector(t, minor_vec);
    let c = apply_point(t, center);

    let uu = dot(u, u);
    let vv = dot(v, v);
    let uv = dot(u, v);
    let trace = uu + vv;
    let diff = uu - vv;
    let disc = (diff * diff + 4.0 * uv * uv).sqrt();
    let lam1 = 0.5 * (trace + disc);
    let lam2 = (0.5 * (trace - disc)).max(0.0);
    let s1 = lam1.max(0.0).sqrt();
    let s2 = lam2.sqrt();
    if s1 < 1e-300 {
        // The whole plane collapsed to a point.
        return SemanticGeometry::Point(c);
    }
    // Eigenvector of the larger eigenvalue.
    let (ex, ey) = if uv.abs() > 1e-300 {
        let len = (uv * uv + (lam1 - uu) * (lam1 - uu)).sqrt();
        if len > 1e-300 {
            (uv / len, (lam1 - uu) / len)
        } else {
            (1.0, 0.0)
        }
    } else if uu >= vv {
        (1.0, 0.0)
    } else {
        (0.0, 1.0)
    };
    let (e2x, e2y) = (-ey, ex);
    let w1 = scale(add(scale(u, ex), scale(v, ey)), 1.0 / s1);
    let w2 = if s2 > 1e-300 {
        scale(add(scale(u, e2x), scale(v, e2y)), 1.0 / s2)
    } else {
        Point3::default()
    };
    let theta = ey.atan2(ex);
    let normal_out = if s2 > 1e-300 {
        let nn = cross(w1, w2);
        if length(nn) >= 1e-12 {
            normalize(nn)
        } else {
            affine_normal_fallback(t, n)
        }
    } else {
        affine_normal_fallback(t, n)
    };
    SemanticGeometry::Ellipse {
        center: c,
        normal: normal_out,
        major_axis: scale(w1, s1),
        ratio: s2 / s1,
        start: start - theta,
        sweep,
    }
}

pub(crate) fn affine_normal_fallback(t: &Transform3, n: Point3) -> Point3 {
    let mapped = apply_vector(t, n);
    if length(mapped) >= 1e-12 {
        normalize(mapped)
    } else {
        world_z_axis()
    }
}

/// Recover a bulge segment as an exact circular arc in the polyline's plane.
///
/// `bulge = tan(θ/4)`; the arc is the one bowing to the sign of the bulge. This
/// mirrors the tessellator's construction so the analytic and sampled forms are
/// the same curve (audit B23).
pub(crate) fn bulge_arc_geometry(
    a: Point3,
    b: Point3,
    bulge: f64,
    plane_normal: Point3,
) -> Option<SemanticGeometry> {
    let chord = sub(b, a);
    let chord_len = length(chord);
    if chord_len < 1e-12 {
        return None;
    }
    let theta = 4.0 * bulge.atan();
    let half = theta * 0.5;
    let half_chord = chord_len * 0.5;
    let sagitta = bulge * half_chord;
    if sagitta.abs() < 1e-12 {
        return None;
    }
    let radius_abs = (half_chord * half_chord + sagitta * sagitta) / (2.0 * sagitta.abs());
    let (ax, ay, _) = arbitrary_axis(plane_normal);
    let d2 = [dot(chord, ax), dot(chord, ay)];
    let chord2_len = (d2[0] * d2[0] + d2[1] * d2[1]).sqrt();
    if chord2_len < 1e-12 {
        // The chord is parallel to the plane normal: no in-plane arc exists.
        return None;
    }
    let dir2 = [d2[0] / chord2_len, d2[1] / chord2_len];
    let left2 = [-dir2[1], dir2[0]];
    let u2 = if sagitta >= 0.0 {
        left2
    } else {
        [-left2[0], -left2[1]]
    };
    let mid2 = [d2[0] * 0.5, d2[1] * 0.5];
    let center2 = [
        mid2[0] - u2[0] * radius_abs * half.cos(),
        mid2[1] - u2[1] * radius_abs * half.cos(),
    ];
    let start_angle = (-center2[1]).atan2(-center2[0]);
    let center = add(a, add(scale(ax, center2[0]), scale(ay, center2[1])));
    Some(SemanticGeometry::Arc {
        center,
        normal: plane_normal,
        radius: radius_abs,
        start: start_angle,
        sweep: theta,
    })
}

pub(crate) fn rotation_of(t: &Transform3) -> f64 {
    // Rotation about Z from the transformed X axis.
    t.matrix[1][0].atan2(t.matrix[0][0])
}
