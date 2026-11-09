//! MIRROR's explicit `Arc`/`Ellipse` arms.
//!
//! ## Why this module exists
//!
//! A reflection is an **orientation-reversing** transform. `cad_db`'s generic
//! transform surface (`transform_geometry`) deliberately refuses it for `Arc`,
//! `Ellipse`, `Text` and `Shape`: a reflected parameterisation would have to be
//! rewritten by hand there, and that surface's job is the proper (rotation +
//! uniform scale) similarity. Rather than loosening that generic contract, MIRROR
//! handles the two curved kinds **explicitly here** — the one command that really
//! is a reflection — and delegates every other kind to `transform_geometry`
//! unchanged (so `Text`/`Shape`/`Opaque` keep their existing refusals).
//!
//! ## The declared rule: the plane normal is negated
//!
//! The mirrored entity keeps the plane of the reflected curve but stores the
//! **opposite** extrusion normal:
//!
//! ```text
//! centre' = R(centre)      normal' = −R(normal)      radius' = radius
//! ```
//!
//! `R(normal)` alone would point out of the reflected plane on the wrong side.
//! Negating it restores the honest orientation of the reflected *plane*, and — the
//! part that decides the stored parameters — keeps `(R·ax, R·ay)` a **right-handed**
//! frame for `normal'`, exactly like [`cad_geometry::arbitrary_axis`] builds for
//! every other arc in this build:
//!
//! ```text
//! (R·ax) × (R·ay) = det(R)·R·(ax × ay) = −R(normal) = normal'
//! ```
//!
//! ### Arc: the start angle is re-derived, the sweep keeps its sign
//!
//! With `normal'` negated as above, the reflected curve is
//! `R(P(t)) = centre' + radius·(cos(t + α)·ax' + sin(t + α)·ay')` for the fixed
//! frame rotation `α` between the two right-handed frames. So the stored start is
//! the **mirrored start point's angle in the mirrored frame** (never a three-point
//! re-derivation), and the parameter still advances with `+t`: the **sweep keeps
//! its sign**. Its magnitude and sign both stay, which is what makes
//! `mirror(start) → mirror(end)` the stored `start → end` (this is pinned by the
//! endpoint tests). The opposite-sweep alternative only holds if the normal is
//! **not** negated; the two choices cannot be mixed, and the negated-normal one is
//! the one this build implements.
//!
//! ### Ellipse: the parameter is preserved (elliptical arcs are supported)
//!
//! For an ellipse `P(θ) = c + cos θ·A + sin θ·B` with `B` along
//! `ellipse_minor_dir(normal, Â)·|A|·|ratio|`, the reflected axis is `A' = R·A`
//! and the canonical minor direction of the mirrored frame is
//!
//! ```text
//! cross(normal', Â') = cross(−R·n, R·Â) = R·cross(n, Â)
//! ```
//!
//! so `R(P(θ)) = centre' + cos θ·A' + sin θ·B'`: the **same** `θ`. The stored
//! `start`/`sweep` are therefore copied verbatim — a full ellipse and an
//! **elliptical arc** (start/sweep params) are both exact, with **no** `θ' = −θ`
//! mapping needed *because* the normal is negated. This module implements the
//! elliptical arc rather than refusing it; `mirror.elliptical_arc` does not exist.
//! The tests assert that the mirrored start, end **and** mid-parameter points equal
//! the reflections of the originals, which is what proves the parameter mapping.

use super::*;
use cad_geometry::{add, arbitrary_axis, dot, length, normalize, scale, sub};

/// Below this the source normal is treated as unusable and the mirrored normal
/// falls back to `−Z` (the reflected `+Z` default every other kind starts at).
const DEGENERATE_NORMAL: f64 = 1e-12;

/// Reflect one entity's geometry for MIRROR, with the `MIRRTEXT` policy.
///
/// `mirrortext = false` (AutoCAD's default) keeps a reflected text run readable:
/// the position mirrors and the rotation follows the reflected baseline, but the
/// glyph order is never reversed. `mirrortext = true` is a named refusal because
/// the flat-text domain has no mirrored-glyph representation.
///
/// `Arc`/`Ellipse` are handled here; every other kind keeps the generic
/// `transform_geometry` behaviour. A **proper** transform (positive determinant)
/// is not a reflection, so it is delegated too.
pub(super) fn mirror_geometry_with_options(
    geometry: &SemanticGeometry,
    transform: &Transform3,
    mirrortext: bool,
) -> CadResult<SemanticGeometry> {
    if transform.determinant() >= 0.0 {
        return transform_geometry(geometry, transform);
    }
    match geometry {
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => mirror_arc(*center, *normal, *radius, *start, *sweep, transform),
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => mirror_ellipse(
            *center,
            *normal,
            *major_axis,
            *ratio,
            *start,
            *sweep,
            transform,
        ),
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
        } => mirror_text(
            text,
            *position,
            *style,
            *height,
            *rotation,
            font,
            *h_align,
            *v_align,
            *wrap_width,
            transform,
            mirrortext,
        ),
        SemanticGeometry::Dimension(dimension) => mirror_dimension(dimension, transform),
        other => transform_geometry(other, transform),
    }
}

/// `TEXT`/`MTEXT`: mirror the insertion point and keep the run readable
/// (`MIRRTEXT` 0).
///
/// The stored `position` is the insertion/anchor point, so it mirrors exactly.
/// The rotation follows the reflected baseline; when that would run the text
/// backwards relative to the source (`cos θ · cos θ' < 0`, the upstream AutoCAD
/// `MIRRTEXT=0` predicate), a half turn is added so the run stays upright.
/// Horizontal/vertical alignment and wrap width are copied unchanged: this
/// domain stores an anchor, not a measured run, so no width compensation is
/// invented. `MIRRTEXT=1` would need a mirrored-glyph flag the flat-text domain
/// does not have, so it is a named refusal.
#[allow(clippy::too_many_arguments)]
fn mirror_text(
    text: &str,
    position: Point3,
    style: StyleId,
    height: f64,
    rotation: f64,
    font: &Option<String>,
    h_align: TextAlignH,
    v_align: TextAlignV,
    wrap_width: Option<f64>,
    transform: &Transform3,
    mirrortext: bool,
) -> CadResult<SemanticGeometry> {
    if mirrortext {
        return Err(CadError::Unsupported(
            "MIRROR mirror.mirrortext_glyphs: MIRRTEXT=1 would mirror the glyphs themselves, \
             which the flat-text domain cannot represent; use the readable default (MIRRTEXT=0)"
                .into(),
        ));
    }
    let new_position = transform.apply_point(position);
    let reflected = reflected_angle(rotation, transform);
    let new_rotation = readable_rotation(rotation, reflected);
    if !is_finite_point(new_position) || !new_rotation.is_finite() {
        return Err(CadError::InvalidInput(
            "MIRROR cannot mirror TEXT: the reflected parameters are not finite".into(),
        ));
    }
    Ok(SemanticGeometry::Text {
        text: text.to_string(),
        position: new_position,
        style,
        height,
        rotation: new_rotation,
        font: font.clone(),
        h_align,
        v_align,
        wrap_width,
    })
}

/// `DIMENSION`: reflect its definition points and re-synthesize the display.
///
/// Autodesk does not document MIRROR on dimensions, so this is our declared
/// choice: every definition point (including the group-11 text middle point) is
/// reflected, the plane normal is negated exactly like the arc/ellipse arms, and
/// the display is rebuilt through the domain's single synthesizer (so the
/// measurement and drawn number stay in step). The dimension's own text stays
/// readable because the synthesizer draws normal text. A dimension whose display
/// came from an external picture block is re-synthesized too; this command cannot
/// address individual picture members, and the definition is the authority here.
fn mirror_dimension(
    dimension: &DimensionGeometry,
    transform: &Transform3,
) -> CadResult<SemanticGeometry> {
    let source = dimension.definition();
    let definition = DimensionDefinition {
        kind: mirror_dimension_kind(source.kind, transform),
        points: mirror_dimension_points(&source.points, transform),
        text_middle: transform.apply_point(source.text_middle),
        text_rotation: if source.text_rotation == 0.0 {
            0.0
        } else {
            readable_rotation(
                source.text_rotation,
                reflected_angle(source.text_rotation, transform),
            )
        },
        normal: reflected_normal(source.normal, transform),
    };
    let rebuilt = DimensionGeometry::synthesized(
        definition,
        dimension.style_name().to_string(),
        dimension.style().clone(),
        dimension.text_override().map(str::to_string),
    )?;
    Ok(SemanticGeometry::Dimension(Box::new(rebuilt)))
}

/// The reflected subtype: a linear dimension's group-50 rotation follows the
/// reflection; the ordinate's X/Y axis flag and every other subtype are kept.
fn mirror_dimension_kind(kind: DimensionKind, transform: &Transform3) -> DimensionKind {
    match kind {
        DimensionKind::Linear { rotation } => DimensionKind::Linear {
            rotation: reflected_angle(rotation, transform),
        },
        other => other,
    }
}

/// Reflect every definition point of a dimension.
fn mirror_dimension_points(points: &DimensionPoints, transform: &Transform3) -> DimensionPoints {
    let p = |point: Point3| transform.apply_point(point);
    match points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => DimensionPoints::Linear {
            first: p(*first),
            second: p(*second),
            dimension_line: p(*dimension_line),
        },
        DimensionPoints::Radial {
            center,
            point_on_arc,
            leader_length,
        } => DimensionPoints::Radial {
            center: p(*center),
            point_on_arc: p(*point_on_arc),
            leader_length: *leader_length,
        },
        DimensionPoints::Angular2Ln {
            line1_start,
            line1_end,
            line2_start,
            line2_end,
            arc_point,
        } => DimensionPoints::Angular2Ln {
            line1_start: p(*line1_start),
            line1_end: p(*line1_end),
            line2_start: p(*line2_start),
            line2_end: p(*line2_end),
            arc_point: p(*arc_point),
        },
        DimensionPoints::Angular3Pt {
            vertex,
            first,
            second,
            arc_point,
        } => DimensionPoints::Angular3Pt {
            vertex: p(*vertex),
            first: p(*first),
            second: p(*second),
            arc_point: p(*arc_point),
        },
        DimensionPoints::Ordinate {
            feature,
            leader_end,
            base,
        } => DimensionPoints::Ordinate {
            feature: p(*feature),
            leader_end: p(*leader_end),
            base: p(*base),
        },
    }
}

/// The direction angle of a vector reflected by `transform`:
/// `β − θ`, where `β` is the angle of the reflected `+X` direction.
fn reflected_angle(angle: f64, transform: &Transform3) -> f64 {
    let x = transform.apply_vector(Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    });
    let beta = x.y.atan2(x.x);
    beta - angle
}

/// The `MIRRTEXT=0` readable rotation: reflect the baseline, then add a half
/// turn when the reflected run would face backwards (`cos θ · cos θ' < 0`).
fn readable_rotation(original: f64, reflected: f64) -> f64 {
    let adjusted = if original.cos() * reflected.cos() < 0.0 {
        reflected + std::f64::consts::PI
    } else {
        reflected
    };
    super::helpers::normalize_signed(adjusted)
}

/// `ARC`: reflect the centre and the plane, re-derive the start angle from the
/// mirrored start point, keep the sweep (magnitude **and** sign).
fn mirror_arc(
    center: Point3,
    normal: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
    transform: &Transform3,
) -> CadResult<SemanticGeometry> {
    let new_center = transform.apply_point(center);
    let new_normal = reflected_normal(normal, transform);
    let radius = radius.abs();
    let (ax, ay, _) = arbitrary_axis(normal);
    let start_point = arc_point(center, radius, (ax, ay), start);
    let reflected_start = transform.apply_point(start_point);
    let (new_ax, new_ay, _) = arbitrary_axis(new_normal);
    let new_start = plane_angle(reflected_start, new_center, new_ax, new_ay);
    if !is_finite_point(new_center) || !new_start.is_finite() || !sweep.is_finite() {
        return Err(CadError::InvalidInput(
            "MIRROR cannot mirror an ARC: the reflected parameters are not finite".into(),
        ));
    }
    // The mirrored entity is a directed curve: it runs mirror(start) →
    // mirror(end), which is exactly what the re-derived start plus the unchanged
    // sweep describe in the mirrored (right-handed) frame.
    Ok(SemanticGeometry::Arc {
        center: new_center,
        normal: new_normal,
        radius,
        start: new_start,
        sweep,
    })
}

/// `ELLIPSE`: reflect the centre and the major axis, keep the ratio and the
/// parameter (`start`/`sweep`), negate the normal.
///
/// This covers a **full** ellipse (`sweep = τ`) and an **elliptical arc** alike;
/// see the module docs for the proof that the parameter is preserved under the
/// negated-normal convention.
fn mirror_ellipse(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
    transform: &Transform3,
) -> CadResult<SemanticGeometry> {
    let new_center = transform.apply_point(center);
    let new_major = transform.apply_vector(major_axis);
    let new_normal = reflected_normal(normal, transform);
    if !is_finite_point(new_center)
        || !is_finite_point(new_major)
        || !start.is_finite()
        || !sweep.is_finite()
    {
        return Err(CadError::InvalidInput(
            "MIRROR cannot mirror an ELLIPSE: the reflected parameters are not finite".into(),
        ));
    }
    if length(new_major) < DEGENERATE_NORMAL {
        return Err(CadError::InvalidInput(
            "MIRROR cannot mirror an ELLIPSE: the reflected major axis is degenerate".into(),
        ));
    }
    Ok(SemanticGeometry::Ellipse {
        center: new_center,
        normal: new_normal,
        major_axis: new_major,
        ratio,
        start,
        sweep,
    })
}

/// The mirrored plane normal: `−R(normal)`, normalised.
///
/// Negating the reflected normal is what keeps the mirrored frame right-handed
/// (see the module docs); a source normal too short to normalise falls back to the
/// reflected `+Z`, i.e. `−Z`.
fn reflected_normal(normal: Point3, transform: &Transform3) -> Point3 {
    let mapped = transform.apply_vector(normal);
    if !length(mapped).is_finite() || length(mapped) < DEGENERATE_NORMAL {
        return Point3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        };
    }
    scale(normalize(mapped), -1.0)
}

/// One point on the arc: `centre + radius·(cos(angle)·ax + sin(angle)·ay)`.
///
/// Term for term the tessellator's expression over
/// [`cad_geometry::arbitrary_axis`], so the derived start angle measures the same
/// frame the display draws.
fn arc_point(center: Point3, radius: f64, axes: (Point3, Point3), angle: f64) -> Point3 {
    add(
        center,
        add(
            scale(axes.0, angle.cos() * radius),
            scale(axes.1, angle.sin() * radius),
        ),
    )
}

/// The in-plane angle of `point` about `center` in the orthonormal frame
/// `(ax, ay)`: `atan2(dot(d, ay), dot(d, ax))`.
fn plane_angle(point: Point3, center: Point3, ax: Point3, ay: Point3) -> f64 {
    let d = sub(point, center);
    dot(d, ay).atan2(dot(d, ax))
}
