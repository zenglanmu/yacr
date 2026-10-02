//! Baking an affine [`Transform3`] into an entity's [`SemanticGeometry`].
//!
//! This is the MOVE/transform half of the controlled write path. It exists in
//! `cad-db` rather than reusing `cad-geometry` because the architecture forbids
//! the database from depending on a higher layer (see `scripts/check-architecture.py`);
//! the small amount of linear algebra needed is therefore local.
//!
//! Scope, stated honestly:
//! - `Line`, `Polyline`, `Point`, `Mesh` accept any finite non-singular affine
//!   transform (a straight segment maps to a straight segment).
//! - `Circle` accepts a **similarity** (translation + rotation + uniform scale,
//!   no shear). Under a mirror the linear part flips the stored `normal`, which
//!   is the honest representation of the reflected circle.
//! - `Arc` and `Ellipse` accept a **positive-determinant similarity**. A mirror
//!   would reverse their parameter orientation, so it is **rejected explicitly**
//!   with [`CadError::Unsupported`], never silently dropped. A non-uniform scale
//!   would distort the curve and is likewise rejected.
//! - `Polyline` with non-zero bulges (arc segments) is only exact under a
//!   similarity; under shear it returns `Unsupported` (a mirror negates the
//!   signed bulges so the arc direction is preserved). Straight-segment
//!   polylines accept any affine.
//! - `Insert` composes the transform onto its stored instance transform; the
//!   referenced block geometry is not cloned or modified.
//! - `Text`, `Spline`, `Opaque` and `Compound` are not baked: they return
//!   `Unsupported` so a caller cannot believe a move happened when it did not.
//!
//! Every accepted result is re-validated by [`crate::validate_geometry`], so a
//! singular or collapsing transform is rejected before it reaches the store.

use cad_domain::*;

use crate::math::{
    add, apply_vector, arbitrary_axis, dot, ellipse_minor_dir, length, normalize, plane_angle,
    scale, sub, transform_is_finite,
};

/// Similarity judgement tolerance for `Transform3::is_uniform_scale`.
const SIMILARITY_TOLERANCE: f64 = 1e-9;

/// Apply `transform` to `geometry`, returning the transformed geometry.
///
/// Fails with [`CadError::Unsupported`] when this build cannot represent the
/// exact image of the source kind, and with [`CadError::InvalidInput`] when the
/// transform itself is non-finite/singular.
pub fn transform_geometry(
    geometry: &SemanticGeometry,
    transform: &Transform3,
) -> CadResult<SemanticGeometry> {
    if !transform_is_finite(transform) {
        return Err(CadError::InvalidInput(
            "transform matrix contains a non-finite value".to_string(),
        ));
    }
    if transform.determinant().abs() < 1e-12 {
        return Err(CadError::InvalidInput(
            "transform is singular and cannot be applied".to_string(),
        ));
    }
    let out = match geometry {
        SemanticGeometry::Line { start, end } => SemanticGeometry::Line {
            start: transform.apply_point(*start),
            end: transform.apply_point(*end),
        },
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            if bulges.iter().any(|b| b.abs() > 1e-12) {
                require_similarity(transform, "polyline with bulge segments")?;
                // A mirror reverses arc direction: negate the signed bulges.
                let flip = transform.determinant() < 0.0;
                let bulges = bulges.iter().map(|b| if flip { -*b } else { *b }).collect();
                SemanticGeometry::Polyline {
                    points: points.iter().map(|p| transform.apply_point(*p)).collect(),
                    bulges,
                    closed: *closed,
                }
            } else {
                SemanticGeometry::Polyline {
                    points: points.iter().map(|p| transform.apply_point(*p)).collect(),
                    bulges: bulges.clone(),
                    closed: *closed,
                }
            }
        }
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            require_similarity(transform, "circle")?;
            let s = uniform_scale(transform);
            // A mirror flips the normal through the linear part automatically,
            // which is the honest representation of the reflected circle.
            let new_normal = mapped_normal(transform, *normal);
            SemanticGeometry::Circle {
                center: transform.apply_point(*center),
                normal: new_normal,
                radius: radius.abs() * s,
            }
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            require_similarity(transform, "arc")?;
            require_orientation_preserving(transform, "arc")?;
            let s = uniform_scale(transform);
            let new_center = transform.apply_point(*center);
            let new_normal = mapped_normal(transform, *normal);
            let new_radius = radius.abs() * s;
            // Recompute the start parameter from the transformed start point in
            // the new plane frame; the sweep magnitude is preserved (positive
            // determinant only).
            let (u, v, _) = arbitrary_axis(*normal);
            let start_point = arc_point(*center, radius.abs(), *start, u, v);
            let (u2, v2, _) = arbitrary_axis(new_normal);
            let new_start = plane_angle(transform.apply_point(start_point), new_center, u2, v2);
            SemanticGeometry::Arc {
                center: new_center,
                normal: new_normal,
                radius: new_radius,
                start: new_start,
                sweep: *sweep,
            }
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            require_similarity(transform, "ellipse")?;
            require_orientation_preserving(transform, "ellipse")?;
            let was_center = *center;
            let new_center = transform.apply_point(was_center);
            let new_normal = mapped_normal(transform, *normal);
            let new_major = apply_vector(transform, *major_axis);
            let major_len = length(new_major);
            let major_unit = scale(new_major, 1.0 / major_len);
            let minor_unit = ellipse_minor_dir(new_normal, major_unit);
            // The transformed source start point, expressed in the new frame.
            let major_unit_src = scale(*major_axis, 1.0 / length(*major_axis));
            let minor_src = scale(
                ellipse_minor_dir(*normal, major_unit_src),
                length(*major_axis) * ratio.abs(),
            );
            let src_start = add(
                was_center,
                add(
                    scale(*major_axis, start.cos()),
                    scale(minor_src, start.sin()),
                ),
            );
            let new_minor_len = major_len * ratio.abs();
            let d = sub(transform.apply_point(src_start), new_center);
            let new_start =
                (dot(d, minor_unit) / new_minor_len).atan2(dot(d, major_unit) / major_len);
            SemanticGeometry::Ellipse {
                center: new_center,
                normal: new_normal,
                major_axis: new_major,
                ratio: *ratio,
                start: new_start,
                sweep: *sweep,
            }
        }
        SemanticGeometry::Point(p) => SemanticGeometry::Point(transform.apply_point(*p)),
        SemanticGeometry::Mesh(mesh) => {
            let mut out = mesh.clone();
            out.vertices = mesh
                .vertices
                .iter()
                .map(|p| transform.apply_point(*p))
                .collect();
            out.normals = mesh
                .normals
                .iter()
                .map(|n| normalize(apply_vector(transform, *n)))
                .collect();
            SemanticGeometry::Mesh(out)
        }
        SemanticGeometry::Insert {
            block,
            transform: insert,
        } => SemanticGeometry::Insert {
            block: *block,
            // Apply the move first, then the instance placement.
            transform: transform.matrix_mul(insert),
        },
        SemanticGeometry::Text { .. } => {
            return Err(CadError::Unsupported(
                "transform of Text entities is not supported".to_string(),
            ));
        }
        SemanticGeometry::Spline { .. } => {
            return Err(CadError::Unsupported(
                "transform of Spline entities is not supported".to_string(),
            ));
        }
        SemanticGeometry::Opaque { .. } => {
            return Err(CadError::Unsupported(
                "transform of opaque/proxy entities is not supported".to_string(),
            ));
        }
        SemanticGeometry::Compound(_) => {
            return Err(CadError::Unsupported(
                "transform of compound entities is not supported".to_string(),
            ));
        }
    };
    Ok(out)
}

fn require_similarity(transform: &Transform3, what: &str) -> CadResult<()> {
    if !transform.is_uniform_scale(SIMILARITY_TOLERANCE) {
        return Err(CadError::Unsupported(format!(
            "transform of {what} requires a rotation + uniform scale; \
             non-uniform scale or shear cannot be represented exactly"
        )));
    }
    Ok(())
}

/// Reject a reflection for a kind whose parameterisation would reverse.
///
/// The requirement is that a flip is either represented honestly or refused;
/// for `Arc`/`Ellipse` this build refuses rather than risk an inverted sweep.
fn require_orientation_preserving(transform: &Transform3, what: &str) -> CadResult<()> {
    if transform.determinant() < 0.0 {
        return Err(CadError::Unsupported(format!(
            "transform of {what} would mirror it; a reflected {what} is not \
             represented by this build and is refused rather than dropped"
        )));
    }
    Ok(())
}

fn uniform_scale(transform: &Transform3) -> f64 {
    let m = &transform.matrix;
    let sx = (m[0][0] * m[0][0] + m[1][0] * m[1][0] + m[2][0] * m[2][0]).sqrt();
    let sy = (m[0][1] * m[0][1] + m[1][1] * m[1][1] + m[2][1] * m[2][1]).sqrt();
    let sz = (m[0][2] * m[0][2] + m[1][2] * m[1][2] + m[2][2] * m[2][2]).sqrt();
    ((sx + sy + sz) / 3.0).max(1e-12)
}

fn mapped_normal(transform: &Transform3, normal: Point3) -> Point3 {
    let mapped = apply_vector(transform, normal);
    if length(mapped) < 1e-12 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    } else {
        normalize(mapped)
    }
}

/// Free function so the Arc arm can reconstruct the source start point from the
/// stored parameter without duplicating the frame convention.
fn arc_point(center: Point3, radius: f64, angle: f64, u: Point3, v: Point3) -> Point3 {
    add(
        center,
        add(
            scale(u, radius * angle.cos()),
            scale(v, radius * angle.sin()),
        ),
    )
}
