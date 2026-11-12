//! Geometry and property comparison helpers for `OVERKILL`.
//!
//! Split out of `overkill.rs` so both files stay inside the code-size budget.

use super::*;

/// A tolerance comparison; `tol == 0.0` is exact equality of the stored values.
pub(super) fn within(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

pub(super) fn points_equal(a: Point3, b: Point3, tol: f64) -> bool {
    within(a.x, b.x, tol) && within(a.y, b.y, tol) && within(a.z, b.z, tol)
}

pub(super) fn vsub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

pub(super) fn vdot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub(super) fn vnorm(a: Point3) -> f64 {
    vdot(a, a).sqrt()
}

pub(super) fn vscale(a: Point3, factor: f64) -> Point3 {
    Point3 {
        x: a.x * factor,
        y: a.y * factor,
        z: a.z * factor,
    }
}

/// Whether two geometries are duplicates within `tol`.
///
/// Only the five kinds with a documented comparison can be equal; every other
/// kind (and a kind paired with a different kind) is never a duplicate.
pub(super) fn same_duplicate_geometry(
    a: &SemanticGeometry,
    b: &SemanticGeometry,
    tol: f64,
) -> bool {
    match (a, b) {
        (
            SemanticGeometry::Line { start: a0, end: a1 },
            SemanticGeometry::Line { start: b0, end: b1 },
        ) => {
            (points_equal(*a0, *b0, tol) && points_equal(*a1, *b1, tol))
                || (points_equal(*a0, *b1, tol) && points_equal(*a1, *b0, tol))
        }
        (
            SemanticGeometry::Polyline {
                points: a_points,
                bulges: a_bulges,
                closed: a_closed,
                ..
            },
            SemanticGeometry::Polyline {
                points: b_points,
                bulges: b_bulges,
                closed: b_closed,
                ..
            },
        ) => {
            if a_closed != b_closed || a_points.len() != b_points.len() {
                return false;
            }
            if !a_points
                .iter()
                .zip(b_points)
                .all(|(a, b)| points_equal(*a, *b, tol))
            {
                return false;
            }
            let segments = if *a_closed {
                a_points.len()
            } else {
                a_points.len().saturating_sub(1)
            };
            (0..segments).all(|i| {
                within(
                    a_bulges.get(i).copied().unwrap_or(0.0),
                    b_bulges.get(i).copied().unwrap_or(0.0),
                    tol,
                )
            })
        }
        (
            SemanticGeometry::Circle {
                center: a_center,
                normal: a_normal,
                radius: a_radius,
            },
            SemanticGeometry::Circle {
                center: b_center,
                normal: b_normal,
                radius: b_radius,
            },
        ) => {
            points_equal(*a_center, *b_center, tol)
                && points_equal(*a_normal, *b_normal, tol)
                && within(*a_radius, *b_radius, tol)
        }
        (
            SemanticGeometry::Arc {
                center: a_center,
                normal: a_normal,
                radius: a_radius,
                start: a_start,
                sweep: a_sweep,
            },
            SemanticGeometry::Arc {
                center: b_center,
                normal: b_normal,
                radius: b_radius,
                start: b_start,
                sweep: b_sweep,
            },
        ) => {
            points_equal(*a_center, *b_center, tol)
                && points_equal(*a_normal, *b_normal, tol)
                && within(*a_radius, *b_radius, tol)
                && within(*a_start, *b_start, tol)
                && within(*a_sweep, *b_sweep, tol)
        }
        (
            SemanticGeometry::Spline {
                degree: a_degree,
                knots: a_knots,
                control_points: a_points,
                weights: a_weights,
            },
            SemanticGeometry::Spline {
                degree: b_degree,
                knots: b_knots,
                control_points: b_points,
                weights: b_weights,
            },
        ) => {
            a_degree == b_degree
                && a_knots.len() == b_knots.len()
                && a_points.len() == b_points.len()
                && a_weights.len() == b_weights.len()
                && a_knots
                    .iter()
                    .zip(b_knots)
                    .all(|(a, b)| within(*a, *b, tol))
                && a_points
                    .iter()
                    .zip(b_points)
                    .all(|(a, b)| points_equal(*a, *b, tol))
                && a_weights
                    .iter()
                    .zip(b_weights)
                    .all(|(a, b)| within(*a, *b, tol))
        }
        _ => false,
    }
}

/// The linetype *provenance/name* comparison.
///
/// The explicit variant also carries the entity's linetype scale; that factor is
/// compared by its own toggle, so it is deliberately excluded here.
fn linetype_same(a: &EntityLineType, b: &EntityLineType) -> bool {
    match (a, b) {
        (EntityLineType::ByLayer, EntityLineType::ByLayer) => true,
        (EntityLineType::ByBlock, EntityLineType::ByBlock) => true,
        (
            EntityLineType::Explicit { name: a_name, .. },
            EntityLineType::Explicit { name: b_name, .. },
        ) => a_name.eq_ignore_ascii_case(b_name),
        _ => false,
    }
}

fn attributes(db: &DrawingDatabase, entity: &DbEntity) -> EntityRenderAttributes {
    db.entity_render_attributes(entity.id)
}

/// Whether two entities may be compared under the ignore toggles.
pub(super) fn properties_match(
    db: &DrawingDatabase,
    a: &DbEntity,
    b: &DbEntity,
    settings: &OverkillSettings,
) -> bool {
    if a.space != b.space {
        return false;
    }
    if !settings.ignore_layer && a.layer != b.layer {
        return false;
    }
    let (a_attrs, b_attrs) = (attributes(db, a), attributes(db, b));
    if !settings.ignore_color && a_attrs.color != b_attrs.color {
        return false;
    }
    if !settings.ignore_linetype && !linetype_same(&a_attrs.linetype, &b_attrs.linetype) {
        return false;
    }
    if !settings.ignore_linetype_scale
        && !within(
            a_attrs.linetype_scale,
            b_attrs.linetype_scale,
            settings.tolerance,
        )
    {
        return false;
    }
    if !settings.ignore_lineweight && a_attrs.lineweight != b_attrs.lineweight {
        return false;
    }
    if !settings.ignore_transparency && a_attrs.transparency != b_attrs.transparency {
        return false;
    }
    true
}
