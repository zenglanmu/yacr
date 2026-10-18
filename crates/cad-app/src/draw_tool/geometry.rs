//! Pure geometry helpers for the drawing tools.
//!
//! Split out of [`super`] to keep the state-machine module inside the code-size
//! limit. Every item here is a pure function or value type over `Point3` /
//! `Transform3`: no capture state, no database handle, no UI type. The re-exports
//! in [`super`] keep the historical `crate::draw_tool::<name>` paths (and the
//! public `cad_app::circle_radius`, re-exported by `lib.rs`) valid.

use cad_domain::{CadError, CadResult, Point3, Transform3};

/// Rotation about `base` by `angle` radians in the XY plane (about the +Z axis).
///
/// Built as `T(base) ∘ R(angle) ∘ T(−base)` through `matrix_mul` (right-hand side
/// applied first) and `from_basis` for the orthonormal rotation.
pub(crate) fn rotation_transform(base: Point3, angle: f64) -> Transform3 {
    let (sin, cos) = angle.sin_cos();
    let to_origin = Transform3::translation(Point3 {
        x: -base.x,
        y: -base.y,
        z: -base.z,
    });
    let back = Transform3::translation(base);
    let rotation = Transform3::from_basis(
        Point3 {
            x: cos,
            y: sin,
            z: 0.0,
        },
        Point3 {
            x: -sin,
            y: cos,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    );
    back.matrix_mul(&rotation).matrix_mul(&to_origin)
}

/// Uniform scale about `base` by `factor`, built as
/// `T(base) ∘ S(factor) ∘ T(−base)`.
pub(crate) fn scale_transform(base: Point3, factor: f64) -> Transform3 {
    let to_origin = Transform3::translation(Point3 {
        x: -base.x,
        y: -base.y,
        z: -base.z,
    });
    let back = Transform3::translation(base);
    back.matrix_mul(&Transform3::scale(factor))
        .matrix_mul(&to_origin)
}

/// Reflection across the line through `p1` and `p2` (in the XY plane).
///
/// The reflection is a negative-determinant affine, so it cannot be expressed
/// with `from_basis` (which is always right-handed); the 2×2 reflection is built
/// directly and translated to pass through `p1`. A zero-length axis is refused.
pub(crate) fn mirror_transform(p1: Point3, p2: Point3) -> CadResult<Transform3> {
    let dx = p2.x - p1.x;
    let dy = p2.y - p1.y;
    let len = dx.hypot(dy);
    if !len.is_finite() || len <= 1e-12 {
        return Err(CadError::InvalidInput(
            "mirror axis needs two distinct points".into(),
        ));
    }
    let (ux, uy) = (dx / len, dy / len);
    let linear = [
        [ux * ux - uy * uy, 2.0 * ux * uy, 0.0],
        [2.0 * ux * uy, uy * uy - ux * ux, 0.0],
        [0.0, 0.0, 1.0],
    ];
    let mut matrix = [[0.0f64; 4]; 4];
    for r in 0..3 {
        for c in 0..3 {
            matrix[r][c] = linear[r][c];
        }
    }
    matrix[3][3] = 1.0;
    // Translation column = p1 − M·p1, so the reflection passes through p1.
    let mapped = [
        linear[0][0] * p1.x + linear[0][1] * p1.y + linear[0][2] * p1.z,
        linear[1][0] * p1.x + linear[1][1] * p1.y + linear[1][2] * p1.z,
        linear[2][0] * p1.x + linear[2][1] * p1.y + linear[2][2] * p1.z,
    ];
    matrix[0][3] = p1.x - mapped[0];
    matrix[1][3] = p1.y - mapped[1];
    matrix[2][3] = p1.z - mapped[2];
    Ok(Transform3 { matrix })
}

/// The centre and radius point of a CIRCLE intent, if the radius is positive.
///
/// A zero-radius circle is refused here so the UI can show the explicit
/// invalid-input reason before emitting a command; the database validates it
/// again.
pub fn circle_radius(center: Point3, edge: Point3) -> CadResult<f64> {
    let dx = edge.x - center.x;
    let dy = edge.y - center.y;
    let dz = edge.z - center.z;
    let radius = (dx * dx + dy * dy + dz * dz).sqrt();
    if !radius.is_finite() || radius <= 0.0 {
        return Err(CadError::InvalidInput(
            "circle radius must be positive and finite".into(),
        ));
    }
    Ok(radius)
}

/// The centre, radius and signed angular span of the arc through three XY points.
///
/// The arc starts at `start`, passes through `through` and ends at `end`, on the
/// work-plane `+Z` normal. `sweep` is signed (negative means clockwise) so a
/// reflected capture is not silently re-drawn as its complement. Non-finite,
/// non-coplanar (different z) or collinear/coincident points are an explicit
/// refusal so no degenerate or silently-wrong 3D arc is ever created.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcParameters {
    pub center: Point3,
    pub radius: f64,
    /// Start angle in radians about `+Z`.
    pub start: f64,
    /// Signed sweep in radians; `|sweep| < 2π`.
    pub sweep: f64,
}

/// Compute the arc (centre/radius/start/sweep) through three points.
///
/// The circumcircle is solved in a frame translated by `start`, which makes both
/// the degeneracy test and the centre arithmetic translation- and
/// scale-invariant: a valid arc at large world coordinates is neither refused
/// nor computed with catastrophic cancellation. The operation is the documented
/// flat XY subset: points whose `z` differs from `start.z` beyond tolerance are
/// refused rather than projected into a silently-wrong arc. Collinear, coincident
/// or non-finite points are refused; the caller must not fabricate a centre or a
/// default sweep.
pub fn arc_parameters(start: Point3, through: Point3, end: Point3) -> CadResult<ArcParameters> {
    if !finite(start) || !finite(through) || !finite(end) {
        return Err(CadError::InvalidInput("arc points must be finite".into()));
    }
    // XY-only subset: a 3-point arc is a planar construction. Refuse a
    // non-coplanar pickup rather than silently projecting it.
    let z_tol = 1e-9 * (1.0 + start.z.abs() + through.z.abs() + end.z.abs());
    if (through.z - start.z).abs() > z_tol || (end.z - start.z).abs() > z_tol {
        return Err(CadError::InvalidInput(
            "arc points must lie in one XY plane".into(),
        ));
    }
    // Translate to `start` so the circumcircle is computed in a local frame.
    let a = (through.x - start.x, through.y - start.y);
    let b = (end.x - start.x, end.y - start.y);
    let a_len = a.0.hypot(a.1);
    let b_len = b.0.hypot(b.1);
    if a_len <= 1e-12 || b_len <= 1e-12 {
        return Err(CadError::InvalidInput("arc points are coincident".into()));
    }
    // Translation- and scale-invariant collinearity test (relative cross product).
    let cross = a.0 * b.1 - a.1 * b.0;
    if !cross.is_finite() || cross.abs() <= 1e-12 * a_len * b_len {
        return Err(CadError::InvalidInput("arc points are collinear".into()));
    }
    // Local circumcentre: `|C| = |a - C| = |b - C|` gives `a·C = |a|²/2`,
    // `b·C = |b|²/2`; solving by Cramer's rule yields the offset from `start`.
    let a_sq = a.0 * a.0 + a.1 * a.1;
    let b_sq = b.0 * b.0 + b.1 * b.1;
    let denom = 2.0 * cross;
    let offset_x = (a_sq * b.1 - b_sq * a.1) / denom;
    let offset_y = (a.0 * b_sq - b.0 * a_sq) / denom;
    let center = Point3 {
        x: start.x + offset_x,
        y: start.y + offset_y,
        z: start.z,
    };
    let radius = offset_x.hypot(offset_y);
    if !radius.is_finite() || radius <= 0.0 {
        return Err(CadError::InvalidInput(
            "arc radius must be positive and finite".into(),
        ));
    }
    let angle = |p: Point3| (p.y - center.y).atan2(p.x - center.x);
    let start_angle = angle(start);
    let through_angle = angle(through);
    let end_angle = angle(end);
    let norm = |x: f64| {
        let tau = std::f64::consts::TAU;
        let x = x % tau;
        if x < 0.0 {
            x + tau
        } else {
            x
        }
    };
    let ccw = norm(end_angle - start_angle);
    let to_through = norm(through_angle - start_angle);
    // If the middle point is not on the counter-clockwise path, the arc runs
    // clockwise: a negative sweep, never the complementary CCW arc.
    let sweep = if to_through <= ccw {
        ccw
    } else {
        ccw - std::f64::consts::TAU
    };
    if !sweep.is_finite() || sweep.abs() <= 1e-12 {
        return Err(CadError::InvalidInput(
            "arc points are collinear or coincident".into(),
        ));
    }
    Ok(ArcParameters {
        center,
        radius,
        start: start_angle,
        sweep,
    })
}

/// The smallest distance (world units) for a first-axis span or an off-axis
/// half-length to be a real ellipse axis rather than a degenerate pick.
///
/// Like [`MIN_RECTANGLE_EXTENT`], this is a tiny relative-free world epsilon so
/// an exactly-degenerate capture is refused while any real span is accepted.
const MIN_ELLIPSE_AXIS_EXTENT: f64 = 1e-12;

/// The centre, major semi-axis vector and axis ratio of an ELLIPSE built by
/// AutoCAD's **axis-endpoint method** (ADR 0015 decision A/D).
///
/// `p1` and `p2` are the two endpoints of the **first axis** (the centre is their
/// midpoint and `a = |p1p2|/2`); `p3` gives the **other** half-axis length as its
/// perpendicular distance `b` to the line through `p1`/`p2`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EllipseParameters {
    /// Midpoint of the first axis.
    pub center: Point3,
    /// Semi-axis vector of the **longer** axis, relative to `center`, on the
    /// work-plane `+Z` construction (the domain/DXF convention; see below).
    pub major_axis: Point3,
    /// The **shorter** semi-axis over the longer one, in `(0, 1]`.
    pub ratio: f64,
}

/// Compute the ellipse frame for three axis-endpoint picks.
///
/// The stored `major_axis` is a **centre-relative** semi-axis vector (in world
/// orientation), not an absolute endpoint: `cad-geometry::tessellate` maps the
/// ellipse as `center + major_axis·cos t + minor·sin t`, and the DXF ELLIPSE
/// group 11 is likewise "endpoint of major axis, relative to the center". The
/// minor direction is implied by the `+Z` normal.
///
/// **Axis-swap rule (ADR 0015 decision D — a labelled, unverified storage
/// assumption)**: the *longer* of the two semi-axes is stored as `major_axis`, so
/// `ratio <= 1` holds by construction — this is what keeps the DXF group 40
/// legal (`cad-db` only validates `ratio > 0`, so the create path is the sole
/// guarantee). For a full ellipse the swap is shape-preserving: the same point
/// set, only the stored parameterisation changes.
///
/// Explicit refusals (never a fabricated or silently-projected ellipse): a
/// non-finite pick; a degenerate first axis (`|p1p2| ≈ 0`); a third pick **on**
/// the first-axis line (`ratio ≈ 0`); and a non-coplanar `+Z` pick (the flat
/// work-plane subset, ADR 0015 "NOT IMPLEMENTED: non-planar input").
pub fn ellipse_parameters(p1: Point3, p2: Point3, p3: Point3) -> CadResult<EllipseParameters> {
    if !finite(p1) || !finite(p2) || !finite(p3) {
        return Err(CadError::InvalidInput(
            "ellipse picks must be finite".into(),
        ));
    }
    // Flat `+Z` subset: a first axis plus an off-axis distance is a planar
    // construction, so a pick off the first two points' plane is refused rather
    // than silently projected.
    let z_tol = 1e-9 * (1.0 + p1.z.abs() + p2.z.abs() + p3.z.abs());
    if (p2.z - p1.z).abs() > z_tol || (p3.z - p1.z).abs() > z_tol {
        return Err(CadError::InvalidInput(
            "ellipse picks must lie in one +Z plane".into(),
        ));
    }
    let axis = (p2.x - p1.x, p2.y - p1.y, p2.z - p1.z);
    let axis_len = (axis.0 * axis.0 + axis.1 * axis.1 + axis.2 * axis.2).sqrt();
    if !axis_len.is_finite() || axis_len <= MIN_ELLIPSE_AXIS_EXTENT {
        return Err(CadError::InvalidInput(
            "ellipse first axis is degenerate (the two endpoints coincide)".into(),
        ));
    }
    let unit = (axis.0 / axis_len, axis.1 / axis_len, axis.2 / axis_len);
    let center = Point3 {
        x: (p1.x + p2.x) * 0.5,
        y: (p1.y + p2.y) * 0.5,
        z: (p1.z + p2.z) * 0.5,
    };
    let a = axis_len * 0.5;
    // Perpendicular distance of `p3` from the (infinite) first-axis line.
    let to_p3 = (p3.x - p1.x, p3.y - p1.y, p3.z - p1.z);
    let along = to_p3.0 * unit.0 + to_p3.1 * unit.1 + to_p3.2 * unit.2;
    let perp = (
        to_p3.0 - along * unit.0,
        to_p3.1 - along * unit.1,
        to_p3.2 - along * unit.2,
    );
    let b = (perp.0 * perp.0 + perp.1 * perp.1 + perp.2 * perp.2).sqrt();
    if !b.is_finite() || b <= MIN_ELLIPSE_AXIS_EXTENT {
        return Err(CadError::InvalidInput(
            "ellipse third pick lies on the first axis (degenerate ratio)".into(),
        ));
    }
    // Axis swap: the longer semi-axis becomes `major_axis`. `perp` has length
    // `b`, and `(p2 - p1)/2 = unit·a`, so both branches are the same two semi-axis
    // lengths `{a, b}` with `ratio = min/max <= 1`.
    let (major_axis, ratio) = if b > a {
        (
            Point3 {
                x: perp.0,
                y: perp.1,
                z: perp.2,
            },
            a / b,
        )
    } else {
        (
            Point3 {
                x: axis.0 * 0.5,
                y: axis.1 * 0.5,
                z: axis.2 * 0.5,
            },
            b / a,
        )
    };
    Ok(EllipseParameters {
        center,
        major_axis,
        ratio,
    })
}

fn finite(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}
