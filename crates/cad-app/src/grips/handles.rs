//! The handle **list** every geometry exposes — the read half of the grip model.
//!
//! Split out of `grips/mod.rs` so both files stay well under the 1000-line cap.
//! The parent module keeps the handle table, the coordinate rules, the
//! hit-testing/tie-break rules and every `grip.*` refusal code; the drag rules
//! live in `grips/drag.rs`. Everything here is a pure function of one entity's
//! stored geometry: no drawing, no session, no GPU, and no marker is invented for
//! a kind this build cannot drag.

use cad_db::DbEntity;
use cad_domain::{Point3, SemanticGeometry, Transform3};
use cad_geometry::{add, arbitrary_axis, cross, dot, length, normalize, scale, sub};

use super::{Grip, GripId, GripKind, QUADRANT_DIRECTIONS};

/// The handles `entity` exposes in plane space.
///
/// `LINE` ⇒ `start`, `end` endpoints then the midpoint; `CIRCLE` ⇒ the centre
/// then quadrants `0..=3`; `ARC` ⇒ the centre, then the start (`Endpoint` `0`)
/// and end (`Endpoint` `1`) points; `ELLIPSE` ⇒ the centre, then quadrants
/// `0..=3` **when the stored parameter range is a full turn** (a partial ellipse
/// exposes the centre grip only, and its quadrant ids are
/// [`super::REASON_PARTIAL_ELLIPSE`]); `POLYLINE` ⇒ one [`GripKind::Vertex`] per
/// stored vertex, in stored order (`0..=255`; a longer polyline exposes the first
/// 256 vertices only, because [`GripId::index`] is a `u8` — the rest are simply
/// not addressable in this build); `SPLINE` ⇒ one [`GripKind::ControlPoint`] per
/// stored control point, in stored order (the same `0..=255` `u8` cap: a spline
/// with more than 256 control points exposes the first 256 and the rest are not
/// addressable), and **no** grip at all when the stored definition is internally
/// inconsistent (`control_points.len() != weights.len()`, or
/// `knots.len() != control_points.len() + degree + 1`) — no marker is invented for
/// a definition this model cannot interpret; `TEXT`/`MTEXT` ⇒ exactly one
/// [`GripKind::Insertion`] grip (`index` 0) at the stored insertion point — the
/// stored `position` verbatim, never offset by `h_align`/`v_align` and never
/// rotated about it by `rotation`; `INSERT` (a block reference) ⇒ exactly one
/// [`GripKind::Insertion`] grip (`index` 0) at the placement's **insertion
/// point**, i.e. the image of the block-space origin under the stored transform
/// (see [`insert_origin`]); `POINT` ⇒ one [`GripKind::Point`]
/// grip; `DIMENSION` ⇒ one [`GripKind::DefinitionPoint`] grip per stored DXF
/// definition point, in the documented per-subtype order, then one for the stored
/// text middle point (group 11) — the order itself, and the drag rule that
/// re-synthesizes the display, live together in `grips/dimension.rs`. Every
/// other geometry — including the whole refusal list in the module doc — yields
/// an empty vector: no marker is invented for a kind this build cannot drag.
#[must_use]
pub fn grips_for_entity(entity: &DbEntity) -> Vec<Grip> {
    let id = |kind, index| GripId {
        entity: entity.id,
        kind,
        index,
    };
    match &entity.geometry {
        SemanticGeometry::Line { start, end } => vec![
            Grip {
                id: id(GripKind::Endpoint, 0),
                position: *start,
            },
            Grip {
                id: id(GripKind::Endpoint, 1),
                position: *end,
            },
            Grip {
                id: id(GripKind::Midpoint, 2),
                position: midpoint(*start, *end),
            },
        ],
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            let axes = plane_axes(*normal);
            let mut grips = Vec::with_capacity(1 + QUADRANT_DIRECTIONS.len());
            grips.push(Grip {
                id: id(GripKind::Center, 0),
                position: *center,
            });
            for (index, direction) in QUADRANT_DIRECTIONS.iter().enumerate() {
                // `QUADRANT_DIRECTIONS` has exactly four entries, so the index
                // always fits the documented `0..=3` range.
                grips.push(Grip {
                    id: id(GripKind::Quadrant, index as u8),
                    position: quadrant_point(*center, *radius, axes, *direction),
                });
            }
            grips
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let axes = plane_axes(*normal);
            vec![
                Grip {
                    id: id(GripKind::Center, 0),
                    position: *center,
                },
                Grip {
                    id: id(GripKind::Endpoint, 0),
                    position: arc_point(*center, *radius, axes, *start),
                },
                Grip {
                    id: id(GripKind::Endpoint, 1),
                    position: arc_point(*center, *radius, axes, *start + *sweep),
                },
            ]
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            sweep,
            ..
        } => {
            let mut grips = Vec::with_capacity(1 + QUADRANT_DIRECTIONS.len());
            grips.push(Grip {
                id: id(GripKind::Center, 0),
                position: *center,
            });
            // A partial ellipse (an elliptical arc) stores a parameter range, and
            // its axis points need not lie on the drawn range: the centre is the
            // only handle offered. A full ellipse whose stored frame is
            // degenerate (`ellipse_axes` returns `None`) also offers the centre
            // only — a quadrant marker with an underivable position is never
            // fabricated.
            if is_full_ellipse(*sweep) {
                if let Some(axes) = ellipse_axes(*normal, *major_axis) {
                    for (index, direction) in QUADRANT_DIRECTIONS.iter().enumerate() {
                        grips.push(Grip {
                            id: id(GripKind::Quadrant, index as u8),
                            position: ellipse_quadrant_point(
                                *center,
                                *major_axis,
                                *ratio,
                                axes,
                                *direction,
                            ),
                        });
                    }
                }
            }
            grips
        }
        SemanticGeometry::Polyline { points, .. } => points
            .iter()
            .take(MAX_GRIP_INDEX + 1)
            .enumerate()
            .map(|(index, position)| Grip {
                // `index < MAX_GRIP_INDEX + 1`, so the `u8` conversion is
                // exact (see the module doc: vertices past 255 are not
                // addressable and are therefore not offered, never wrapped).
                id: id(GripKind::Vertex, index as u8),
                position: *position,
            })
            .collect(),
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            // A definition this model cannot interpret exposes no handle at all:
            // padding a knot/weight vector would silently rewrite the curve, and
            // `cad-db`'s validator is the authority on shape validity.
            if !spline_is_consistent(*degree, knots, control_points, weights) {
                return Vec::new();
            }
            control_points
                .iter()
                .take(MAX_GRIP_INDEX + 1)
                .enumerate()
                .map(|(index, position)| Grip {
                    // Same `u8` cap as the polyline vertices: control points past
                    // 255 are not addressable and are therefore not offered.
                    id: id(GripKind::ControlPoint, index as u8),
                    position: *position,
                })
                .collect()
        }
        SemanticGeometry::Point(position) => vec![Grip {
            id: id(GripKind::Point, 0),
            position: *position,
        }],
        // TEXT and MTEXT are one variant (only `object.type_key` tells the DXF
        // classes apart), so both expose the same single handle: the **stored**
        // insertion point, verbatim. No alignment offset and no rotation is
        // applied — the grip marks where the run is anchored.
        SemanticGeometry::Text { position, .. } => vec![Grip {
            id: id(GripKind::Insertion, 0),
            position: *position,
        }],
        // A block reference exposes the **insertion point** of its placement, i.e.
        // the transformed block-space origin. This is a top-level model-space
        // entity handle only: a block member is reached through an instance path
        // (`SelectionRef::instance`), which a `GripId` cannot even express, and
        // `GRIPSTRETCH` refuses a non-model-space target with
        // `property.not_addressable`. The model therefore describes exactly the
        // geometry it is handed and adds no second addressability rule.
        SemanticGeometry::Insert {
            block: _,
            transform,
        } => vec![Grip {
            id: id(GripKind::Insertion, 0),
            position: insert_origin(transform),
        }],
        // A dimension exposes its stored DXF definition points in the documented
        // per-subtype order, then the stored text middle point. The order and the
        // drag rule live together in `grips/dimension.rs`, so the handle list and
        // the replaceable slots cannot drift; a definition whose subtype and point
        // slots disagree exposes no handle at all (and cannot be stored through the
        // public constructors).
        SemanticGeometry::Dimension(dimension) => {
            super::dimension::definition_point_grips(entity.id, dimension)
        }
        _ => Vec::new(),
    }
}

/// The highest handle index a [`GripId`] can address: `GripId::index` is a `u8`.
///
/// Shared by every index-based handle family (polyline vertices, spline control
/// points) so the `0..=255` cap has exactly one value in this module.
const MAX_GRIP_INDEX: usize = u8::MAX as usize;

/// The world **insertion point** of a block reference: the image of the
/// block-space origin under the stored placement.
///
/// This is the INSERT's own insertion point, and it reuses the existing
/// convention instead of inventing one: the importer folds the block's base point
/// into the placement (`cad-import-acadrust`'s `block_placement` is the
/// translation `-base_point`), so the stored `INSERT` insertion point *is* where
/// the block-space origin lands. `Transform3` is a 4x4 affine whose point mapping
/// has no homogeneous divide ([`Transform3::apply_point`]), so the origin's image
/// is exactly the translation column of the matrix. The column is read directly
/// rather than through `apply_point`, because the linear part's `0.0 *
/// coefficient` terms would turn a non-finite coefficient into a `NaN` and hide
/// the corrupt transform behind a fabricated handle position.
pub fn insert_origin(transform: &Transform3) -> Point3 {
    Point3 {
        x: transform.matrix[0][3],
        y: transform.matrix[1][3],
        z: transform.matrix[2][3],
    }
}

/// Whether a stored spline definition is internally consistent enough to expose
/// control-point handles for.
///
/// Two structural invariants, both pure length arithmetic on the stored vectors:
///
/// * `control_points.len() == weights.len()` — every control point carries its
///   own weight (a non-rational B-spline stores all `1.0`, but the vector is still
///   per control point);
/// * `knots.len() == control_points.len() + degree + 1` — a clamped B-spline of
///   degree `d` with `n + 1` control points has `n + d + 2` knots.
///
/// This is **not** shape validation: `cad-db`'s validator is the authority on what
/// may be stored, and this model only declines to invent a handle for a definition
/// it cannot index into. A mismatch is not repaired here (padding either vector
/// would change the curve behind the user's back); the spline simply exposes no
/// grip, and every control-point id against it is
/// [`super::REASON_STALE_HANDLE`].
pub(super) fn spline_is_consistent(
    degree: u32,
    knots: &[f64],
    control_points: &[Point3],
    weights: &[f64],
) -> bool {
    if control_points.len() != weights.len() {
        return false;
    }
    let expected_knots = control_points.len() + degree as usize + 1;
    knots.len() == expected_knots
}

/// Whether a stored ellipse parameter range covers the whole curve.
///
/// The slack is [`super::ARC_SWEEP_EPSILON`], the same one the tessellator and the
/// representation provider use to call a sweep "full", so the handle set and the
/// drawn curve agree about which ellipses are complete.
pub(super) fn is_full_ellipse(sweep: f64) -> bool {
    sweep.abs() >= std::f64::consts::TAU - super::ARC_SWEEP_EPSILON
}

/// The ellipse's own plane frame `(unit major, unit minor)`.
///
/// `unit_minor = normalize(cross(unit(normal), unit(major_axis)))` is the
/// expression `cad_geometry`'s crate-private `ellipse_minor_dir` evaluates, i.e.
/// the frame the tessellator and the representation provider draw the ellipse
/// with. The axes are the **stored** ones; `arbitrary_axis` is only used for the
/// radially symmetric circle/arc, where every in-plane direction is equivalent.
///
/// `None` when no frame can be derived: a non-finite or zero-length major axis, a
/// non-finite normal, or a normal parallel to the major axis (`cad-db` validation
/// refuses to store such an ellipse, so this is the corrupt case). The caller then
/// offers no quadrant handle rather than a fabricated position.
pub(super) fn ellipse_axes(normal: Point3, major_axis: Point3) -> Option<(Point3, Point3)> {
    let major_len = length(major_axis);
    if !major_len.is_finite() || major_len <= 0.0 {
        return None;
    }
    let unit_major = scale(major_axis, 1.0 / major_len);
    let minor = cross(normalize(normal), unit_major);
    let minor_len = length(minor);
    // `1e-9` is the orthogonality slack `ellipse_minor_dir` uses before its
    // fallback, and `cad-db` validation refuses a stored ellipse below it.
    if !minor_len.is_finite() || minor_len < 1e-9 {
        return None;
    }
    Some((unit_major, scale(minor, 1.0 / minor_len)))
}

/// One ellipse axis handle position: the tessellator's
/// `center + major_axis * cos t + minor * sin t` at `t = 0, π/2, π, 3π/2`, term
/// for term — `centre ± major_axis` for the major axis and
/// `centre ± (|major_axis| * |ratio|) * unit_minor` for the minor axis.
pub(super) fn ellipse_quadrant_point(
    center: Point3,
    major_axis: Point3,
    ratio: f64,
    axes: (Point3, Point3),
    direction: (u8, f64),
) -> Point3 {
    let (which, sign) = direction;
    let vector = if which == 0 {
        major_axis
    } else {
        scale(axes.1, length(major_axis) * ratio.abs())
    };
    add(center, scale(vector, sign))
}

/// The circle/arc plane frame: the `(ax, ay)` of the shared arbitrary-axis frame.
///
/// `ax` and `ay` are orthonormal and lie in the plane whose normal is `normal`;
/// they are the same axes the tessellator, the representation provider and the
/// importer measure circle/arc angles in.
pub(super) fn plane_axes(normal: Point3) -> (Point3, Point3) {
    let (ax, ay, _) = arbitrary_axis(normal);
    (ax, ay)
}

/// The exact f64 midpoint of two stored points: `(start + end) * 0.5`.
pub(super) fn midpoint(start: Point3, end: Point3) -> Point3 {
    scale(add(start, end), 0.5)
}

/// One quadrant handle position: `centre + radius * (sign * axis)`.
pub(super) fn quadrant_point(
    center: Point3,
    radius: f64,
    axes: (Point3, Point3),
    direction: (u8, f64),
) -> Point3 {
    let (which, sign) = direction;
    let axis = if which == 0 { axes.0 } else { axes.1 };
    add(center, scale(axis, sign * radius))
}

/// The distance from `center` to `to`, **projected into the plane** spanned by
/// the orthonormal in-plane axes `axes`.
///
/// This is the circle-quadrant radius rule: the out-of-plane component of
/// `to - center` is discarded, so a drag can never tilt the circle.
pub(super) fn in_plane_distance(center: Point3, axes: (Point3, Point3), to: Point3) -> f64 {
    let offset = sub(to, center);
    dot(offset, axes.0).hypot(dot(offset, axes.1))
}

/// One point on the arc: `centre + radius * (cos(angle) * ax + sin(angle) * ay)`.
///
/// The expression is the tessellator's, term for term, so an arc endpoint grip
/// sits on exactly the vertex the display draws for the same stored `start` /
/// `sweep` (no second angle convention, no rounding).
pub(super) fn arc_point(center: Point3, radius: f64, axes: (Point3, Point3), angle: f64) -> Point3 {
    add(
        center,
        add(
            scale(axes.0, angle.cos() * radius),
            scale(axes.1, angle.sin() * radius),
        ),
    )
}
