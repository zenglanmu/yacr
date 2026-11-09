//! ALIGN edit command (ADR 0016 decision F).
//!
//! Three input shapes are supported at the command layer:
//!
//! * **one point pair** — a pure move of the selection by `dest1 − source1`
//!   (AutoCAD's "continue" after a single pair);
//! * **two point pairs** — translation + rotation, and a uniform scale when
//!   the session `AlignScale` flag is true (the scale's reference length is the
//!   distance between the two destination points);
//! * **three point pairs** — a **rigid** planar transform (rotation +
//!   translation) solved from the two triangles. AutoCAD 2026's three-pair form
//!   is a 3D plane-to-plane rotation with no scaling available; in this 2D build
//!   it is the rigid subset, so a non-congruent or mirror-order triangle pair is
//!   refused by name (`align.non_congruent` / `align.mirrored`) instead of
//!   being approximated by a similarity.
//!
//! The two-pair and three-pair transforms are applied **in place** (no copies)
//! in one transaction with one undo step. A one-pair move uses the same in-place
//! transform surface. An identity transform is the named no-op
//! `align.unchanged`: no transaction, no revision bump, no undo entry.
//!
//! Every input is validated before any write, so a refusal changes nothing (the
//! database transaction is all-or-nothing and no history is recorded).
//!
//! The extra shapes are accepted through the existing typed `Points` payload
//! (2 points = one pair, 6 points = three pairs) with the selection taken from
//! the session. The interactive capture now emits all three shapes: the session
//! `alignPairs` parameter (`1`/`2`/`3`) fixes the step count at tool start, and
//! the draw tool builds `Points` for one/three pairs and the dedicated `Align`
//! payload for two.
//!
//! NOT IMPLEMENTED: 3D alignment and non-uniform scaling.

use super::*;

use crate::draw_tool::{rotation_transform, scale_transform, ToolParameter, ToolValue};

/// The minimum XY length of a source/destination pair for it to define a
/// direction (and, for the source pair, a non-zero scale denominator).
///
/// A shorter pair has no well-defined angle and is refused rather than
/// fabricating a direction or dividing by zero.
const MIN_ALIGN_PAIR_EXTENT: f64 = 1e-12;

/// The tolerance used to recognise an identity transform and coplanar input.
const ALIGN_TOLERANCE: f64 = 1e-9;

/// The point pairs that drive one ALIGN run.
enum AlignPairs {
    /// One pair: `(source1, dest1)` — a pure translation.
    One(Point3, Point3),
    /// Two pairs: the original translate + rotate (+ optional scale) shape.
    Two(Point3, Point3, Point3, Point3),
    /// Three pairs: a rigid planar transform solved from the two triangles.
    Three([Point3; 3], [Point3; 3]),
}

impl Application {
    /// ALIGN the selected entities from one, two or three point pairs.
    ///
    /// See the module docs for the input shapes and the named refusals.
    pub(crate) fn align_entities(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (refs, pairs) = align_input(session, command)?;
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "AlignEntities needs at least one selected reference".into(),
            ));
        }
        let transform = match pairs {
            AlignPairs::One(source1, dest1) => {
                if !is_finite_point(source1) || !is_finite_point(dest1) {
                    return Err(CadError::InvalidInput(
                        "AlignEntities points must be finite".into(),
                    ));
                }
                Transform3::translation(Point3 {
                    x: dest1.x - source1.x,
                    y: dest1.y - source1.y,
                    z: dest1.z - source1.z,
                })
            }
            AlignPairs::Two(source1, dest1, source2, dest2) => {
                two_pair_transform(source1, dest1, source2, dest2, session)?
            }
            AlignPairs::Three(sources, dests) => three_pair_transform(sources, dests)?,
        };

        if is_identity(&transform) {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "align.unchanged".into(),
                    message: "ALIGN: the point pairs define the identity transform; \
                              nothing was moved"
                        .into(),
                }],
                measurement: None,
                point_report: None,
            });
        }

        self.transform_selection(
            command,
            &refs,
            &transform,
            "align entities",
            "align",
            "AlignEntities",
            "ALIGN",
            // The diagnostic verb is ASCII: the ADR fence forbids new CJK literals
            // on the Rust side, and this action slot only reaches the pre-existing
            // sub-element/instance diagnostic templates.
            "align",
        )
    }
}

/// Read the selection and the point pairs from the two accepted typed payloads.
fn align_input(
    session: &SessionState,
    command: &Command,
) -> CadResult<(Vec<SelectionRef>, AlignPairs)> {
    match &command.payload {
        CommandPayload::Align {
            refs,
            source1,
            dest1,
            source2,
            dest2,
        } => Ok((
            refs.clone(),
            AlignPairs::Two(*source1, *dest1, *source2, *dest2),
        )),
        // The direct-caller form: the session selection is the target set and the
        // points are the captured pairs in AutoCAD's order.
        CommandPayload::Points(points) if points.len() == 2 => Ok((
            session.selection.refs().to_vec(),
            AlignPairs::One(points[0], points[1]),
        )),
        CommandPayload::Points(points) if points.len() == 6 => Ok((
            session.selection.refs().to_vec(),
            AlignPairs::Three(
                [points[0], points[1], points[2]],
                [points[3], points[4], points[5]],
            ),
        )),
        other => Err(CadError::InvalidInput(format!(
            "AlignEntities needs an Align payload or 2/6 typed points, got {} point(s)",
            match other {
                CommandPayload::Points(points) => points.len(),
                _ => 0,
            }
        ))),
    }
}

/// The original two-pair transform: translate ∘ rotate ∘ [uniform scale].
fn two_pair_transform(
    source1: Point3,
    dest1: Point3,
    source2: Point3,
    dest2: Point3,
    session: &SessionState,
) -> CadResult<Transform3> {
    if !is_finite_point(source1)
        || !is_finite_point(dest1)
        || !is_finite_point(source2)
        || !is_finite_point(dest2)
    {
        return Err(CadError::InvalidInput(
            "AlignEntities points must be finite".into(),
        ));
    }
    // A pair whose points coincide has no direction. The source pair also
    // supplies the scale denominator, so both pairs must be non-degenerate
    // (a zero-length destination pair would otherwise divide by zero).
    let source_pair = pair_vector(source1, source2);
    let dest_pair = pair_vector(dest1, dest2);
    if pair_length(source_pair) <= MIN_ALIGN_PAIR_EXTENT {
        return Err(CadError::InvalidInput(
            "ALIGN source pair needs two distinct points".into(),
        ));
    }
    if pair_length(dest_pair) <= MIN_ALIGN_PAIR_EXTENT {
        return Err(CadError::InvalidInput(
            "ALIGN destination pair needs two distinct points".into(),
        ));
    }
    let angle = (source_pair.x * dest_pair.y - source_pair.y * dest_pair.x)
        .atan2(source_pair.x * dest_pair.x + source_pair.y * dest_pair.y);
    if !angle.is_finite() {
        return Err(CadError::InvalidInput(
            "ALIGN source and destination pairs must define a finite angle".into(),
        ));
    }
    let align_scale = match session.tool_parameter(ToolParameter::AlignScale) {
        ToolValue::Flag(value) => value,
        // The store validates on write, so `AlignScale` is always a flag; the
        // number arm is unreachable and falls back to the documented default.
        ToolValue::Number(_) => false,
    };
    let rotate = rotation_transform(source1, angle);
    let translate = Transform3::translation(Point3 {
        x: dest1.x - source1.x,
        y: dest1.y - source1.y,
        z: dest1.z - source1.z,
    });
    // `matrix_mul` applies the right-hand side first, so this composes
    // translate ∘ rotate ∘ scale (scale applied to the point first).
    Ok(if align_scale {
        let factor = pair_length(dest_pair) / pair_length(source_pair);
        let scale = scale_transform(source1, factor);
        translate.matrix_mul(&rotate).matrix_mul(&scale)
    } else {
        translate.matrix_mul(&rotate)
    })
}

/// Solve the **rigid** planar transform mapping the source triangle onto the
/// destination triangle (rotation + translation, no scaling and no reflection).
///
/// The build's ALIGN is the 2D (`+Z`) subset, so both triangles must be
/// horizontal (all three points of each triangle share one `z`). AutoCAD 2026
/// offers no scaling with three pairs (its three-pair form is a 3D
/// plane-to-plane rotation); the 2D equivalent is rigid, so:
///
/// * a triangle pair whose corresponding side lengths differ (beyond the
///   tolerance) is `align.non_congruent`;
/// * a mirror-order triangle pair (opposite signed area) is `align.mirrored` —
///   the only isometry that maps them is a reflection, which is not available;
/// * a degenerate or non-coplanar triangle keeps `align.source_degenerate` /
///   `align.dest_degenerate` / `align.non_coplanar`.
fn three_pair_transform(sources: [Point3; 3], dests: [Point3; 3]) -> CadResult<Transform3> {
    for point in sources.iter().chain(dests.iter()) {
        if !is_finite_point(*point) {
            return Err(CadError::InvalidInput(
                "AlignEntities points must be finite".into(),
            ));
        }
    }
    if !same_z(&sources) || !same_z(&dests) {
        return Err(CadError::InvalidInput(
            "align.non_coplanar: three-pair ALIGN is the planar (+Z) rigid transform; the source \
             and destination triangles must each lie in one horizontal plane"
                .into(),
        ));
    }
    let source01 = pair_vector(sources[0], sources[1]);
    let dest01 = pair_vector(dests[0], dests[1]);
    if pair_length(source01) <= MIN_ALIGN_PAIR_EXTENT {
        return Err(CadError::InvalidInput(
            "align.source_degenerate: the first two source points coincide, so there is no \
             direction"
                .into(),
        ));
    }
    if pair_length(dest01) <= MIN_ALIGN_PAIR_EXTENT {
        return Err(CadError::InvalidInput(
            "align.dest_degenerate: the first two destination points coincide".into(),
        ));
    }
    // A collinear triangle has no plane and no rotation to solve.
    let source_cross = triangle_cross(&sources);
    if source_cross.abs() <= MIN_ALIGN_PAIR_EXTENT * pair_length(source01) {
        return Err(CadError::InvalidInput(
            "align.source_degenerate: the three source points are collinear; no planar \
             rotation exists"
                .into(),
        ));
    }
    let dest_cross = triangle_cross(&dests);
    if dest_cross.abs() <= MIN_ALIGN_PAIR_EXTENT * pair_length(dest01) {
        return Err(CadError::InvalidInput(
            "align.dest_degenerate: the three destination points are collinear; no planar \
             rotation exists"
                .into(),
        ));
    }

    // Every corresponding side must have the same length: a rigid transform
    // preserves the triangle's shape, so a difference is a non-congruent pair.
    let mut scale: f64 = 1.0;
    for (a, b) in [(0usize, 1usize), (1, 2), (2, 0)] {
        scale = scale.max(pair_length(pair_vector(sources[a], sources[b])));
        scale = scale.max(pair_length(pair_vector(dests[a], dests[b])));
    }
    let allowed = ALIGN_TOLERANCE * scale;
    for (a, b) in [(0usize, 1usize), (1, 2), (2, 0)] {
        let source_side = pair_length(pair_vector(sources[a], sources[b]));
        let dest_side = pair_length(pair_vector(dests[a], dests[b]));
        if (source_side - dest_side).abs() > allowed {
            return Err(CadError::InvalidInput(format!(
                "align.non_congruent: side {}→{} is {source_side:.9} long but its destination is \
                 {dest_side:.9}; three-pair ALIGN is a rigid transform with no scaling",
                a + 1,
                b + 1
            )));
        }
    }
    // The triangles have opposite winding: only a reflection would map them,
    // and reflection is not an available three-pair transform.
    if (source_cross > 0.0) != (dest_cross > 0.0) {
        return Err(CadError::InvalidInput(
            "align.mirrored: the destination triangle has the opposite point order (mirror \
             winding); three-pair ALIGN is a rotation + translation with no reflection"
                .into(),
        ));
    }

    let angle = (source01.x * dest01.y - source01.y * dest01.x)
        .atan2(source01.x * dest01.x + source01.y * dest01.y);
    if !angle.is_finite() {
        return Err(CadError::InvalidInput(
            "ALIGN source and destination pairs must define a finite angle".into(),
        ));
    }
    let rotate = rotation_transform(sources[0], angle);
    let translate = Transform3::translation(Point3 {
        x: dests[0].x - sources[0].x,
        y: dests[0].y - sources[0].y,
        z: dests[0].z - sources[0].z,
    });
    let transform = translate.matrix_mul(&rotate);
    // Defence in depth: a triangle that passed the side checks within accumulated
    // tolerance must still map its third pair onto the destination.
    let mapped_third = transform.apply_point(sources[2]);
    let residual = pair_length(pair_vector(mapped_third, dests[2]));
    if !residual.is_finite() || residual > allowed {
        return Err(CadError::InvalidInput(format!(
            "align.non_congruent: the third point pair is {residual:.9} away from the rigid \
             transform fixed by the first two pairs"
        )));
    }
    Ok(transform)
}

/// The signed XY area (twice the triangle area) of a horizontal triplet.
fn triangle_cross(points: &[Point3; 3]) -> f64 {
    let a = pair_vector(points[0], points[1]);
    let b = pair_vector(points[0], points[2]);
    a.x * b.y - a.y * b.x
}

/// Whether every point of a horizontal triplet shares one `z` (within tolerance).
fn same_z(points: &[Point3; 3]) -> bool {
    let z = points[0].z;
    points
        .iter()
        .all(|point| (point.z - z).abs() <= ALIGN_TOLERANCE)
}

/// Whether an affine transform is the identity (within [`ALIGN_TOLERANCE`]).
fn is_identity(transform: &Transform3) -> bool {
    let m = &transform.matrix;
    for (row, values) in m.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            let expected = if row == column { 1.0 } else { 0.0 };
            if (value - expected).abs() > ALIGN_TOLERANCE {
                return false;
            }
        }
    }
    true
}

/// The XY direction vector from `a` to `b` (the `+Z` construction's plane).
fn pair_vector(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: b.x - a.x,
        y: b.y - a.y,
        z: b.z - a.z,
    }
}

/// The XY length of a pair vector (the rotation and scale are `+Z` constructions).
fn pair_length(v: Point3) -> f64 {
    v.x.hypot(v.y)
}
