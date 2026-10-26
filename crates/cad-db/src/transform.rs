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
//!   would distort the curve and is likewise rejected. This surface stays
//!   proper-similarity only on purpose: the one command that really is a
//!   reflection (MIRROR) handles `Arc`/`Ellipse` with its own explicit arms in
//!   `cad-app` (`app_drawing::mirror_curves`), so a mirror reached through *this*
//!   function keeps refusing them.
//! - `Polyline` with non-zero bulges (arc segments) is only exact under a
//!   similarity; under shear it returns `Unsupported` (a mirror negates the
//!   signed bulges so the arc direction is preserved). Straight-segment
//!   polylines accept any affine.
//! - `Insert` composes the transform onto its stored instance transform; the
//!   referenced block geometry is not cloned or modified.
//! - `Image` transforms its placement (`origin` as a point, `u`/`v` as vectors)
//!   and keeps the pixel size, resource key, clip and visibility unchanged.
//! - `Mask` transforms every boundary point as a point and keeps `inverted`.
//! - `Text` accepts a **positive-determinant similarity** (ADR 0019 decision B):
//!   `position` moves as a point, `rotation` accumulates the in-plane rotation
//!   and `height` scales uniformly; the string, style, font and alignment are
//!   preserved. A non-uniform scale (stretched glyphs) or a mirror (reflected
//!   glyphs) cannot be represented by the flat-text domain, so both are refused
//!   explicitly with [`CadError::Unsupported`]. `cad-geometry`'s snap face
//!   applies the same guards.
//! - `MultiLeader` accepts a **translation + rotation only** (ADR 0019 C.5):
//!   its roots, lines, text and plane frame move together and the display is
//!   rebuilt. A scale (even uniform) or a mirror is refused for the whole
//!   entity, because it would change text height/arrowhead size/dogleg length
//!   while the raw `90` override mask keeps its bits.
//! - `Spline` accepts **any finite non-singular affine** transform. A NURBS curve
//!   is **affinely invariant**: mapping every control point through the transform
//!   while copying `degree`, `knots` and `weights` verbatim yields exactly the
//!   affine image of the curve. Translation, rotation, uniform **and non-uniform**
//!   scale, mirror and shear are therefore all exact — no refitting, no
//!   reparameterisation and no weight change. [`Transform3::apply_point`] is
//!   strictly affine (it reads only rows 0..2 and performs no homogeneous divide),
//!   so the whole affine group applies and no projective row can sneak in. The
//!   structural relations `crate::validate_geometry` enforces
//!   (`knots.len() == control_points.len() + degree + 1`,
//!   `weights.len() == control_points.len()`, non-decreasing knots) are preserved
//!   **by construction** because those vectors are copied, never recomputed.
//! - `Shape`, `Opaque` and `Compound` are still refused. `Shape` stores one glyph
//!   as `position` + `size` + `rotation`, so only a similarity is representable at
//!   all (a shear or a non-uniform scale has no field to live in) and its arm is
//!   not implemented; `Opaque` holds proxy bytes with no geometry to map; a
//!   `Compound` would have to map every member, and its members may include kinds
//!   whose image this build refuses. None of the three is the conservative
//!   "control hulls are not modelled" reason that used to cover splines, so none
//!   of them is widened here.
//!
//! Every accepted result is re-validated by [`crate::validate_geometry`], so a
//! singular or collapsing transform is rejected before it reaches the store.

use cad_domain::*;

use crate::math::{
    add, apply_vector, arbitrary_axis, dot, ellipse_minor_dir, length, normalize, plane_angle,
    rotation_of, scale, sub, transform_is_finite,
};

/// Similarity judgement tolerance for `Transform3::is_uniform_scale`.
const SIMILARITY_TOLERANCE: f64 = 1e-9;

/// Apply `transform` to `geometry`, returning the transformed geometry.
///
/// Fails with [`CadError::Unsupported`] when this build cannot represent the
/// exact image of the source kind, and with [`CadError::InvalidInput`] when the
/// transform itself is non-finite/singular or when a finite transform overflows a
/// finite coordinate into a non-finite one (a spline's mapped control points are
/// checked, since the result would be unevaluable).
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
        // A raster image is a rigid placement: `origin` transforms as a point,
        // while the edge vectors `u`/`v` transform through the linear part only
        // (they are directions, one pixel wide/high). The pixel size and the
        // logical resource key are not geometry and are preserved verbatim.
        SemanticGeometry::Image {
            origin,
            u,
            v,
            pixels,
            file,
            clip,
            visible,
        } => SemanticGeometry::Image {
            origin: transform.apply_point(*origin),
            u: apply_vector(transform, *u),
            v: apply_vector(transform, *v),
            pixels: *pixels,
            file: file.clone(),
            clip: clip.clone(),
            visible: *visible,
        },
        // A mask is a world-space polygon: every boundary point moves as a point.
        SemanticGeometry::Mask { boundary, inverted } => SemanticGeometry::Mask {
            boundary: boundary.iter().map(|p| transform.apply_point(*p)).collect(),
            inverted: *inverted,
        },
        // A standalone TEXT is a similarity-only entity (ADR 0019 decision B):
        // translation moves `position`, rotation accumulates into `rotation` and a
        // uniform scale multiplies `height`. A non-uniform scale would stretch the
        // glyphs into a shape the flat-text domain cannot express, and a mirror
        // would reflect them, so **both refuse the whole entity** with a named
        // `Unsupported` diagnostic. `cad-geometry`'s snap face applies exactly the
        // same guards so the two transform surfaces cannot disagree.
        SemanticGeometry::Text {
            text,
            position,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
            wrap_width,
        } => {
            require_similarity(transform, "text")?;
            require_orientation_preserving(transform, "text")?;
            SemanticGeometry::Text {
                text: text.clone(),
                position: transform.apply_point(*position),
                style: *style,
                height: height * uniform_scale(transform),
                rotation: rotation + rotation_of(transform),
                font: font.clone(),
                h_align: *h_align,
                v_align: *v_align,
                // The boundary is a world-space length, so a similarity scales
                // it with the text instead of dropping it.
                wrap_width: wrap_width.map(|width| width * uniform_scale(transform)),
            }
        }
        // A NURBS curve is affinely invariant: the affine image of the curve is
        // the curve through the mapped control points with the very same degree,
        // knot vector and weights. Every vector is copied (never recomputed), so
        // the counts `validate_geometry` relates to each other cannot change and
        // the transformed definition is as indexable as the source. This is an
        // exact map under *any* non-singular affine transform — including a
        // non-uniform scale, a shear and a mirror — because `apply_point` is
        // strictly affine (rows 0..2, no homogeneous divide).
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            let mapped: Vec<Point3> = control_points
                .iter()
                .map(|point| transform.apply_point(*point))
                .collect();
            // A finite transform can still overflow a finite control point
            // (`1e308 * 1e10`). Refuse that here rather than returning a
            // definition whose coordinates are infinite: no consumer can
            // evaluate or bound it, and the caller asked for an exact image.
            if !mapped.iter().all(|point| finite_point(*point)) {
                return Err(CadError::InvalidInput(
                    "transformed spline control point is non-finite".to_string(),
                ));
            }
            SemanticGeometry::Spline {
                degree: *degree,
                knots: knots.clone(),
                control_points: mapped,
                weights: weights.clone(),
            }
        }
        SemanticGeometry::Shape { .. } => {
            return Err(CadError::Unsupported(
                "transform of Shape entities is not supported".to_string(),
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
        // A dimension transforms its definition points, its display children
        // (including the internal `Text`) and its measurement together, so a
        // MOVE/COPY/ARRAY/ALIGN/STRETCH keeps them in sync (ADR 0017 A.5). A
        // standalone `Text` entity is still refused above; a dimension's own
        // text is not a standalone entity.
        SemanticGeometry::Dimension(dimension) => {
            SemanticGeometry::Dimension(Box::new(dimension.transformed(transform)?))
        }
        // A hatch transforms its loops, plane, seeds and pattern definition and
        // rebuilds its display; a non-similarity is refused for the whole
        // entity (ADR 0018 E).
        SemanticGeometry::Hatch(hatch) => {
            SemanticGeometry::Hatch(Box::new(hatch.transformed(transform)?))
        }
        // A multileader transforms its roots, lines, text and plane frame and
        // rebuilds its display. Only a translation + rotation is accepted: a
        // scale (even uniform) or a mirror is refused for the whole entity,
        // because it would change text height/arrowhead size/dogleg length while
        // the raw `90` override mask keeps its bits (ADR 0019 C.5).
        SemanticGeometry::MultiLeader(multileader) => {
            SemanticGeometry::MultiLeader(Box::new(multileader.transformed(transform)?))
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
/// for `Arc`/`Ellipse`/`Text` this build refuses rather than risk an inverted
/// sweep or a silently unreflected text run (ADR 0019 decision B: the flat-text
/// domain has no reflection).
fn require_orientation_preserving(transform: &Transform3, what: &str) -> CadResult<()> {
    if transform.determinant() < 0.0 {
        return Err(CadError::Unsupported(format!(
            "transform of {what} would mirror it; a reflected {what} is not \
             represented by this build and is refused rather than dropped"
        )));
    }
    Ok(())
}

/// Whether all three coordinates of `point` are finite.
///
/// [`transform_is_finite`] only judges the matrix; a finite matrix applied to a
/// finite point can still overflow to infinity, so a transform that must return
/// an exact, evaluable image checks the result too.
fn finite_point(point: Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
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

#[cfg(test)]
mod dimension_transform_tests {
    use super::*;
    use cad_domain::{
        DimensionDefinition, DimensionGeometry, DimensionKind, DimensionPoints,
        DimensionStyleValues, StyleId,
    };

    fn style() -> DimensionStyleValues {
        DimensionStyleValues {
            arrow_size: 2.5,
            ext_offset: 0.625,
            ext_beyond: 1.25,
            text_height: 2.5,
            text_gap: 0.625,
            center_mark: 0.0,
            dim_line_increment: 3.75,
            text_vertical: 1,
            text_vertical_position: 0.0,
            suppress_ext1: false,
            suppress_ext2: false,
            suppress_dim1: false,
            suppress_dim2: false,
            linear_factor: 1.0,
            decimals: 4,
            rounding: 0.0,
            postfix: String::new(),
            zero_suppression: 0,
            linear_unit_format: 2,
            decimal_separator: 46,
            text_style_name: "Standard".into(),
            text_style: StyleId(0),
            text_font: None,
        }
    }

    fn definition() -> DimensionDefinition {
        DimensionDefinition {
            kind: DimensionKind::Aligned,
            points: DimensionPoints::Linear {
                first: Point3 {
                    x: -21.0,
                    y: 0.0,
                    z: 0.0,
                },
                second: Point3 {
                    x: 21.0,
                    y: 0.0,
                    z: 0.0,
                },
                dimension_line: Point3 {
                    x: -21.0,
                    y: -30.0,
                    z: 0.0,
                },
            },
            text_middle: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            text_rotation: 0.0,
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        }
    }

    fn dimension() -> SemanticGeometry {
        let geometry =
            DimensionGeometry::synthesized(definition(), "Standard".into(), style(), None)
                .expect("synthesized");
        SemanticGeometry::Dimension(Box::new(geometry))
    }

    /// ADR 0017 A.4 invariant 2 / A.5: a MOVE keeps definition and display in
    /// sync (this is the transform arm that used to refuse a compound).
    #[test]
    fn dimension_translates_definition_and_display_together() {
        let moved = transform_geometry(
            &dimension(),
            &Transform3::translation(Point3 {
                x: 100.0,
                y: -5.0,
                z: 0.0,
            }),
        )
        .expect("dimension transforms");
        let SemanticGeometry::Dimension(moved) = moved else {
            panic!("expected a dimension");
        };
        // Measurement is translation-invariant.
        assert!((moved.measurement() - 42.0).abs() < 1e-9);
        match moved.definition().points {
            DimensionPoints::Linear { first, .. } => {
                assert!((first.x - 79.0).abs() < 1e-9);
                assert!((first.y + 5.0).abs() < 1e-9);
            }
            ref other => panic!("expected linear points, got {other:?}"),
        }
    }

    /// ADR 0017 A.5: a mirror or non-uniform scale refuses the whole entity
    /// rather than producing distorted geometry.
    #[test]
    fn dimension_refuses_mirror_and_non_uniform_scale() {
        let mirror = Transform3::scale(-1.0);
        assert!(matches!(
            transform_geometry(&dimension(), &mirror),
            Err(CadError::Unsupported(_))
        ));
        let mut matrix = Transform3::identity().matrix;
        matrix[0][0] = 2.0;
        matrix[1][1] = 3.0;
        assert!(matches!(
            transform_geometry(&dimension(), &Transform3 { matrix }),
            Err(CadError::Unsupported(_))
        ));
    }
}

#[cfg(test)]
mod hatch_transform_tests {
    use super::*;
    use cad_domain::{
        HatchFill, HatchGeometry, HatchLoop, HatchLoopFlags, HatchPatternType, HatchPlane,
        HatchStyle,
    };

    fn square(size: f64) -> HatchLoop {
        HatchLoop::new(
            vec![[0.0, 0.0], [size, 0.0], [size, size], [0.0, size]],
            Vec::new(),
            HatchLoopFlags::default(),
        )
    }

    fn hatch() -> SemanticGeometry {
        let geometry = HatchGeometry::new(
            vec![square(10.0)],
            HatchPlane::default(),
            HatchStyle::Normal,
            HatchFill::Solid,
            Vec::new(),
            None,
            HatchPatternType::UserDefined,
        )
        .expect("valid hatch");
        SemanticGeometry::Hatch(Box::new(geometry))
    }

    /// ADR 0018 E: a MOVE transforms the loops/plane and rebuilds the display.
    #[test]
    fn hatch_translates_loops_plane_and_display() {
        let moved = transform_geometry(
            &hatch(),
            &Transform3::translation(Point3 {
                x: 10.0,
                y: -2.0,
                z: 1.5,
            }),
        )
        .expect("hatch transforms");
        let SemanticGeometry::Hatch(moved) = moved else {
            panic!("expected a hatch");
        };
        assert!((moved.plane().elevation - 1.5).abs() < 1e-9);
        let SemanticGeometry::Polyline { points, .. } = &moved.display()[0] else {
            panic!("expected a boundary polyline");
        };
        assert!((points[0].x - 10.0).abs() < 1e-9, "{:?}", points[0]);
        assert!((points[0].y + 2.0).abs() < 1e-9, "{:?}", points[0]);
        assert!((points[0].z - 1.5).abs() < 1e-9, "{:?}", points[0]);
    }

    /// ADR 0018 E: a non-uniform scale refuses the whole entity.
    #[test]
    fn hatch_refuses_a_non_uniform_transform() {
        let mut matrix = Transform3::identity().matrix;
        matrix[0][0] = 2.0;
        matrix[1][1] = 0.5;
        assert!(matches!(
            transform_geometry(&hatch(), &Transform3 { matrix }),
            Err(CadError::Unsupported(_))
        ));
    }
}
