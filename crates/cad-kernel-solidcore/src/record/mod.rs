//! The two conversions between [`cad_domain::BrepData`] and the upstream
//! portable record (`solidcore_record::brep::Brep`), plus the placement bake
//! both of them share.
//!
//! The conversion boundary is deliberately thin: exactly
//! `to_record(&BrepData) -> Result<Record, LiftRefusal>` and
//! `from_record(&Record) -> Result<BrepData, LiftRefusal>`. No `Body`, no
//! `Brep`, no typed kernel id ever crosses into a `cad-*` crate.
//!
//! Both directions are lossy and say so:
//!
//! - [`to_record`](to_record::to_record) accepts `Line`, `Arc` and full `Circle`
//!   boundaries on `Plane` faces, and labelled `Cylinder`/`Cone`/`Sphere`/`Torus`
//!   or form faces with their exact NURBS form (one closed, consistently oriented
//!   shell). Anything else is refused with
//!   [`LIFT_REQUIRES_PLANAR`](crate::LIFT_REQUIRES_PLANAR) (a curve/surface family
//!   this build does not carry) or [`LIFT_UNSUPPORTED`](crate::LIFT_UNSUPPORTED)
//!   (the body is structurally unusable);
//! - [`from_record`](from_record::from_record) maps the analytic surfaces
//!   `Plane`, `PlaneThrough`, `Cylinder`, `Cone`, `Sphere` and `Torus`, the
//!   **form** surfaces `Spline`/`RationalSpline` (as
//!   `BrepSurface::Nurbs`) and the curves `Line`, full `Circle` and open `Circle`
//!   (an arc); anything else is refused with
//!   [`LIFT_UNSUPPORTED`](crate::LIFT_UNSUPPORTED) rather than degraded.
//!
//! # Asymmetric refusals (read this before relying on a round trip)
//!
//! The two directions are not the same filter, because the neutral model carries
//! less than the record:
//!
//! | record content | `from_record` | `to_record` |
//! |---|---|---|
//! | `Face.surface` = `Spline` / `RationalSpline` (a **form** surface) | mapped to `BrepSurface::Nurbs`, net/knots/weights verbatim (`Spline` gets weight `1.0`) | written back as the same form label; a boundary with no exact pcurve in its parameters is `solid.pcurve_not_exact` |
//! | `Face.nurbs` on an **analytic** label | **ignored**: mapped from the label (`Plane`/`Cylinder`/…), which the record guarantees that form equals | **written** for every labelled curved face, from `eval::form::surface` over the box the boundary pins |
//! | `Face.domain` (the `(u, v)` box) | **carried verbatim** into `BrepFace.domain` (the only source of it) | planes: derived from the boundary. Curved faces: **required** (a face that states none is refused by name), and the written box is the *form's*, derived from the face's own boundary geometry — the producer's numbers are in the producer's parameterisation, which this build does not re-derive. A `Nurbs` face is written with its stated box |
//! | `Edge.nurbs` (the spline form) | **ignored**: mapped from the curve label (`Line`/`Circle`) | never written (a planar boundary needs none) |
//! | `Shell::Faces.wires` (wire edges embedded in a face shell) | **refused** (no wire-edge slot in `BrepData`; dropping them would lose geometry) | always empty |
//! | `Loop::Vertex` | **accepted only as the degenerate apex** of a full cone (the neutral cone face needs that marker); every other vertex loop is refused | never produced |
//! | `Shell::Wire` / `Shell::Vertex`, void regions | refused | never produced |
//! | **more than one shell** | **accepted** (a disjoint `UNION` is a legitimate body: two closed shells) | **refused** — the kernel is handed geometry, so it needs exactly one closed shell |
//! | pcurves | ignored (only the loop geometry is mapped) | planes: the exact `(u, v)` line/arc image; curved faces: the exact closed form the kernel proves, else `solid.pcurve_not_exact` |
//!
//! So a body that carries a spline form on an analytic face round-trips as the
//! analytic surface (same geometry, the form is re-derived), while a body with
//! embedded wire edges does not round-trip at all — it is refused on the way in.
//! A record whose *label* is the spline reads into `BrepSurface::Nurbs` and is
//! written back as that form; its boundary is the one thing that can still stop
//! it, because a form surface's own parameters rarely have a line for a straight
//! 3D boundary (that is a `solid.pcurve_not_exact`, not a missing writer).
//!
//! The multi-shell row is the one callers actually meet: a boolean over disjoint
//! bodies **succeeds** (the `BrepData` holds both shells), and the refusal arrives
//! on the **next lift** — mass properties or another boolean — not at the
//! boolean that produced it. A void-region (cavity) result is the other way
//! round: it is refused **at the boolean itself**, because the kernel stores a
//! cavity as a void region that this direction does not map.

mod arc;
mod curve;
mod curved;
mod from_record;
mod loops;
mod surface;
mod surfaces;
mod to_record;
mod units;

pub(crate) use from_record::from_record;
pub(crate) use to_record::{to_record, DEFAULT_TOLERANCE};
pub(crate) use units::point;

use cad_domain::{
    BrepCurve, BrepData, BrepFace, BrepLoop, BrepPlacement, BrepShell, BrepSurface, Point3,
};

use crate::error::LiftRefusal;

/// The relative tolerance a baked placement must be a similarity within:
/// columns equal in length to this ratio and perpendicular to this cosine.
const PLACEMENT_EPSILON: f64 = 1e-9;

/// The smallest weight a NURBS control point may carry — the record's own bound.
///
/// Baking a placement divides by the weight, so anything below it (or non-finite)
/// is refused by name instead of producing a NaN coordinate.
const NURBS_MIN_WEIGHT: f64 = 1e-12;

/// Add two vectors.
pub(crate) fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

/// Subtract two vectors (`a - b`).
pub(crate) fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

/// Scale a vector.
pub(crate) fn mul(v: Point3, s: f64) -> Point3 {
    Point3 {
        x: v.x * s,
        y: v.y * s,
        z: v.z * s,
    }
}

/// Dot product.
pub(crate) fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// Cross product.
pub(crate) fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

/// Euclidean length.
pub(crate) fn length(v: Point3) -> f64 {
    dot(v, v).sqrt()
}

/// Unit vector, or the zero vector when `v` vanishes (never a NaN).
pub(crate) fn normalize(v: Point3) -> Point3 {
    let len = length(v);
    if len.is_finite() && len > 1e-12 {
        mul(v, 1.0 / len)
    } else {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }
}

/// Whether every coordinate of `p` is finite.
pub(crate) fn finite_point(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

/// Whether a direction is usable as a frame axis (non-zero after normalising).
pub(crate) fn is_axis(v: Point3) -> bool {
    length(v) > 0.5
}

/// Bake `placement` into a copy of `brep` (the result carries no placement).
///
/// Similarity only: a placement whose scale is non-positive or non-finite, or
/// whose frame mirrors (determinant < 0), is refused with
/// [`PLACEMENT_NOT_SUPPORTED`](crate::PLACEMENT_NOT_SUPPORTED) instead of
/// moving the geometry the wrong way round. Radii **and tolerances** (a length
/// each) are scaled by the uniform scale; directions are renormalised; a cone's
/// half angle is unchanged by a similarity.
pub(crate) fn bake(brep: &BrepData, placement: BrepPlacement) -> Result<BrepData, LiftRefusal> {
    if placement.is_identity() {
        return Ok(brep.clone());
    }
    if !placement.scale.is_finite() || placement.scale <= 0.0 {
        return Err(LiftRefusal::placement(format!(
            "placement scale must be finite and positive, got {}",
            placement.scale
        )));
    }
    if placement.matrix.iter().flatten().any(|v| !v.is_finite()) {
        return Err(LiftRefusal::placement("placement matrix is not finite"));
    }
    let m = &placement.matrix;
    let determinant = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    // A similarity's linear part is an orthonormal frame (determinant ±1). A
    // singular or mirrored frame would bake a collapsed or inside-out body, so
    // it is refused rather than written out.
    if !determinant.is_finite() || determinant <= PLACEMENT_EPSILON {
        return Err(LiftRefusal::placement(format!(
            "placement determinant {determinant} is singular or mirrored; a baked placement \
             must be a proper similarity"
        )));
    }
    // Shear and non-uniform scale do not change the determinant away from 1 when
    // they are mild, so the frame itself is checked: equal column lengths and
    // mutually perpendicular unit columns.
    let mut lengths = [0.0f64; 3];
    let mut columns = [Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    }; 3];
    for (index, slot) in lengths.iter_mut().enumerate() {
        let column = Point3 {
            x: m[0][index],
            y: m[1][index],
            z: m[2][index],
        };
        *slot = length(column);
        columns[index] = column;
    }
    let smallest = lengths.iter().copied().fold(f64::INFINITY, f64::min);
    let largest = lengths.iter().copied().fold(0.0f64, f64::max);
    if !(smallest.is_finite() && smallest > 0.0) || largest / smallest > 1.0 + PLACEMENT_EPSILON {
        return Err(LiftRefusal::placement(format!(
            "placement columns have lengths {lengths:?}; a baked placement must scale all three \
             axes equally (non-uniform scale or shear)"
        )));
    }
    for (a, b) in [(0usize, 1usize), (1, 2), (0, 2)] {
        let dot = dot(normalize(columns[a]), normalize(columns[b])).abs();
        if dot > PLACEMENT_EPSILON {
            return Err(LiftRefusal::placement(format!(
                "placement axes {a} and {b} are not perpendicular (cosine {dot}); shear cannot be \
                 baked"
            )));
        }
    }
    let scale = placement.scale;
    let point = |p: Point3| placement.apply(p);
    let direction = |d: Point3| placement.apply_direction(d);

    let mut shells = Vec::with_capacity(brep.shells.len());
    for shell in &brep.shells {
        let mut faces = Vec::with_capacity(shell.faces.len());
        for face in &shell.faces {
            faces.push(BrepFace {
                id: face.id,
                surface: bake_surface(&face.surface, &point, &direction, scale)?,
                reversed: face.reversed,
                loops: face
                    .loops
                    .iter()
                    .map(|loop_| BrepLoop {
                        edges: loop_
                            .edges
                            .iter()
                            .map(|curve| bake_curve(curve, &point, &direction, scale))
                            .collect(),
                    })
                    .collect(),
                // A face's `(u, v)` box is a parameter rectangle, not a length:
                // moving the geometry does not change the parameters.
                domain: face.domain,
                // A tolerance is a length, so a similarity scales it by the
                // one uniform factor. `None` stays unspecified.
                tolerance: face.tolerance.map(|tolerance| tolerance * scale),
            });
        }
        shells.push(BrepShell {
            id: shell.id,
            faces,
        });
    }
    Ok(BrepData {
        shells,
        placement: None,
        tolerance: brep.tolerance.map(|tolerance| tolerance * scale),
    })
}

/// Bake the placement into one surface.
///
/// A similarity maps a NURBS control **point**, so the net is dehomogenised,
/// moved, and re-homogenised with its weight unchanged (`[x·w, y·w, z·w, w]` is
/// invariant under `p → a·p + t` up to the same weights). Knots are parameters and
/// are left exactly as they are.
fn bake_surface(
    surface: &BrepSurface,
    point: &dyn Fn(Point3) -> Point3,
    direction: &dyn Fn(Point3) -> Point3,
    scale: f64,
) -> Result<BrepSurface, LiftRefusal> {
    Ok(match surface {
        BrepSurface::Plane {
            origin,
            normal,
            u_dir,
        } => BrepSurface::Plane {
            origin: point(*origin),
            normal: direction(*normal),
            u_dir: direction(*u_dir),
        },
        BrepSurface::Sphere {
            center,
            radius,
            u_dir,
            pole,
        } => BrepSurface::Sphere {
            center: point(*center),
            radius: radius * scale,
            u_dir: direction(*u_dir),
            pole: direction(*pole),
        },
        BrepSurface::Torus {
            center,
            normal,
            major_radius,
            minor_radius,
            u_dir,
        } => BrepSurface::Torus {
            center: point(*center),
            normal: direction(*normal),
            major_radius: major_radius * scale,
            minor_radius: minor_radius * scale,
            u_dir: direction(*u_dir),
        },
        BrepSurface::Cylinder {
            origin,
            axis,
            ref_dir,
            radius,
        } => BrepSurface::Cylinder {
            origin: point(*origin),
            axis: direction(*axis),
            ref_dir: direction(*ref_dir),
            radius: radius * scale,
        },
        BrepSurface::Cone {
            origin,
            axis,
            ref_dir,
            radius,
            sin_half_angle,
            cos_half_angle,
        } => BrepSurface::Cone {
            origin: point(*origin),
            axis: direction(*axis),
            ref_dir: direction(*ref_dir),
            radius: radius * scale,
            sin_half_angle: *sin_half_angle,
            cos_half_angle: *cos_half_angle,
        },
        BrepSurface::Nurbs {
            u_knots,
            v_knots,
            points,
        } => {
            // Dehomogenise, move, re-homogenise: a similarity acts on the control
            // point, and the weight is a property of the point, not of its
            // position in space.
            let mut baked = Vec::with_capacity(points.len());
            for (row_index, row) in points.iter().enumerate() {
                let mut baked_row = Vec::with_capacity(row.len());
                for (column_index, coefficient) in row.iter().enumerate() {
                    let w = coefficient[3];
                    if !w.is_finite() || w < NURBS_MIN_WEIGHT {
                        return Err(LiftRefusal::unsupported(format!(
                            "a NURBS control point at ({row_index}, {column_index}) has weight \
                             {w}; baking a placement needs a finite positive weight (>= \
                             {NURBS_MIN_WEIGHT}), so the coordinate is refused rather than \
                             divided by it"
                        )));
                    }
                    let control = point(Point3 {
                        x: coefficient[0] / w,
                        y: coefficient[1] / w,
                        z: coefficient[2] / w,
                    });
                    baked_row.push([control.x * w, control.y * w, control.z * w, w]);
                }
                baked.push(baked_row);
            }
            BrepSurface::Nurbs {
                u_knots: u_knots.clone(),
                v_knots: v_knots.clone(),
                points: baked,
            }
        }
        BrepSurface::Unsupported { type_key } => BrepSurface::Unsupported {
            type_key: type_key.clone(),
        },
    })
}

fn bake_curve(
    curve: &BrepCurve,
    point: &dyn Fn(Point3) -> Point3,
    direction: &dyn Fn(Point3) -> Point3,
    scale: f64,
) -> BrepCurve {
    match curve {
        BrepCurve::Line { start, end } => BrepCurve::Line {
            start: point(*start),
            end: point(*end),
        },
        BrepCurve::Circle {
            center,
            normal,
            u_dir,
            radius,
        } => BrepCurve::Circle {
            center: point(*center),
            normal: direction(*normal),
            u_dir: direction(*u_dir),
            radius: radius * scale,
        },
        BrepCurve::Arc {
            center,
            normal,
            u_dir,
            radius,
            start,
            end,
        } => BrepCurve::Arc {
            center: point(*center),
            normal: direction(*normal),
            u_dir: direction(*u_dir),
            radius: radius * scale,
            start: point(*start),
            end: point(*end),
        },
        BrepCurve::Unsupported { type_key } => BrepCurve::Unsupported {
            type_key: type_key.clone(),
        },
    }
}

#[cfg(test)]
// P2.1b part 1: the NURBS form-surface read path, in its own file because the
// conversion files are close to the 1000-line ceiling.
mod nurbs_tests;

#[cfg(test)]
mod curved_tests;

#[cfg(test)]
mod to_record_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::PLACEMENT_NOT_SUPPORTED;

    fn unit_box() -> BrepData {
        crate::polyhedra::box_brep(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            1.0,
            2.0,
            3.0,
        )
    }

    fn placement(matrix: [[f64; 3]; 3], scale: f64) -> BrepPlacement {
        BrepPlacement {
            matrix,
            translation: Point3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            scale,
        }
    }

    fn refuse(placement: BrepPlacement) -> LiftRefusal {
        bake(&unit_box(), placement).expect_err("expected a placement refusal")
    }

    #[test]
    fn identity_and_translation_are_baked() {
        let brep = unit_box();
        assert_eq!(
            bake(&brep, BrepPlacement::identity()).expect("identity"),
            brep
        );

        let moved = bake(
            &brep,
            placement([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], 1.0),
        )
        .expect("translation bakes");
        assert!(moved.placement.is_none());
        let min = moved.shells[0]
            .faces
            .iter()
            .flat_map(|face| &face.loops)
            .flat_map(|loop_| &loop_.edges)
            .map(|curve| match curve {
                BrepCurve::Line { start, .. } => start.x,
                _ => f64::NAN,
            })
            .fold(f64::INFINITY, f64::min);
        assert!((min - 10.0).abs() < 1e-12, "min x {min}");
    }

    #[test]
    fn a_rotation_and_uniform_scale_are_similarities() {
        // 90 degrees about +Z, scaled by 2. The placement is ACIS row-vector
        // (`world = scale·(p·M) + t`), so this maps (x, y, z) -> (-2y, 2x, 2z) + t.
        let rotation = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let baked = bake(&unit_box(), placement(rotation, 2.0)).expect("similarity bakes");
        // Radii and lengths scale too: the box's 1-wide faces become 2 wide.
        let BrepSurface::Plane { u_dir, .. } = baked.shells[0].faces[0].surface else {
            panic!("a planar face");
        };
        assert!((length(u_dir) - 1.0).abs() < 1e-12, "directions stay unit");
        let x: Vec<f64> = baked.shells[0]
            .faces
            .iter()
            .flat_map(|face| &face.loops)
            .flat_map(|loop_| &loop_.edges)
            .filter_map(|curve| match curve {
                BrepCurve::Line { start, .. } => Some(start.x),
                _ => None,
            })
            .collect();
        let (min, max) = (
            x.iter().copied().fold(f64::INFINITY, f64::min),
            x.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        );
        // x' = -2y + 10 with y in [0, 2] -> x' in [6, 10].
        assert!(
            (min - 6.0).abs() < 1e-12 && (max - 10.0).abs() < 1e-12,
            "{min}..{max}"
        );
    }

    #[test]
    fn baking_a_similarity_scales_body_and_face_tolerances_as_lengths() {
        // Both are model-space lengths: baking a placement into the geometry
        // scales them exactly like the coordinates it also scales. This is the
        // **only** place they are scaled — a transform that merely composes a
        // placement must leave them alone (see `cad-db`'s transform test), or a
        // round trip would scale them twice.
        let mut brep = unit_box();
        brep.tolerance = Some(0.5);
        for face in &mut brep.shells[0].faces {
            face.tolerance = Some(0.25);
        }
        let baked = bake(
            &brep,
            placement([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], 2.0),
        )
        .expect("a uniform scale bakes");
        assert_eq!(baked.tolerance, Some(1.0));
        for face in &baked.shells[0].faces {
            assert_eq!(face.tolerance, Some(0.5));
        }
        // `None` (unspecified) stays unspecified: baking never fills it in.
        brep.tolerance = None;
        for face in &mut brep.shells[0].faces {
            face.tolerance = None;
        }
        let baked = bake(
            &brep,
            placement([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], 2.0),
        )
        .expect("a uniform scale bakes");
        assert_eq!(baked.tolerance, None);
        assert!(baked.shells[0]
            .faces
            .iter()
            .all(|face| face.tolerance.is_none()));
    }

    #[test]
    fn a_mirror_is_refused() {
        let mirror = [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let refusal = refuse(placement(mirror, 1.0));
        assert_eq!(refusal.code, PLACEMENT_NOT_SUPPORTED);
        assert!(
            refusal.detail.contains("singular or mirrored"),
            "{}",
            refusal.detail
        );
    }

    #[test]
    fn a_shear_is_refused() {
        let shear = [[1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let refusal = refuse(placement(shear, 1.0));
        assert_eq!(refusal.code, PLACEMENT_NOT_SUPPORTED);
        assert!(
            refusal.detail.contains("not perpendicular") || refusal.detail.contains("non-uniform"),
            "{}",
            refusal.detail
        );
    }

    #[test]
    fn a_non_uniform_scale_is_refused() {
        let stretched = [[2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let refusal = refuse(placement(stretched, 1.0));
        assert_eq!(refusal.code, PLACEMENT_NOT_SUPPORTED);
        assert!(refusal.detail.contains("non-uniform"), "{}", refusal.detail);
    }

    #[test]
    fn a_near_singular_or_non_finite_placement_is_refused() {
        let near_singular = [[1e-12, 0.0, 0.0], [0.0, 1e-12, 0.0], [0.0, 0.0, 1e-12]];
        let refusal = refuse(placement(near_singular, 1.0));
        assert_eq!(refusal.code, PLACEMENT_NOT_SUPPORTED);
        assert!(refusal.detail.contains("singular"), "{}", refusal.detail);

        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let refusal = refuse(placement(
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                scale,
            ));
            assert_eq!(refusal.code, PLACEMENT_NOT_SUPPORTED, "scale {scale}");
            assert!(refusal.detail.contains("scale"), "{}", refusal.detail);
        }
    }
}
