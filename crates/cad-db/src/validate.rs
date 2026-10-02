//! Focused validation for drawing writes.
//!
//! The import builder is deliberately permissive: it records whatever the file
//! holds and reports completeness. The **write** path is different — a new or
//! edited entity must be safe to store, render and measure, so it is validated
//! before any state changes ([`crate::DrawingDatabase::apply_drawing_changes`]).
//!
//! There was no reusable geometry validator in `cad-domain` or `cad-db`, so this
//! module adds one. It is intentionally conservative: it rejects non-finite
//! coordinates and obvious degenerate cases (zero radius, collapsed axes, empty
//! polylines, malformed meshes, singular insert transforms) rather than storing
//! geometry that cannot be drawn. It does not attempt to be a full geometric
//! kernel check.
//!
//! Thresholds are fixed and documented rather than configurable: they are
//! existence checks (`> 0`, not "close to zero"), so a tolerance parameter would
//! only make the contract harder to reason about. `TolerancePolicy` is the
//! policy knob for *computation* (snapping, intersection, join); it is not the
//! right instrument for deciding whether a value is representable.

use cad_domain::*;

use crate::entity::DbEntity;
use crate::math::{cross, length, transform_is_finite};

/// Smallest length/radius a stored curve may have. Chosen well above f64 noise
/// and below any real drawing unit, so legitimate tiny geometry survives while a
/// nominal zero is rejected.
pub const MIN_GEOMETRY_EXTENT: f64 = 1e-12;

/// Validate a [`SemanticGeometry`] before it is written.
///
/// Rejects non-finite coordinates and degenerate/unsupported-for-write cases:
/// a zero-length line, a polyline with fewer than two points, a non-positive
/// circle/arc radius, a collapsed ellipse axis, a spline with too few control
/// points, a mesh with out-of-range or repeated triangle indices, a text run
/// with a non-positive height, and a singular insert transform.
///
/// `Opaque` payloads and `Compound` are accepted recursively: an opaque entity
/// has no numeric content this build can judge, and rejecting it would block a
/// legitimate style edit of an imported proxy.
pub fn validate_geometry(geometry: &SemanticGeometry) -> CadResult<()> {
    let finite = |p: Point3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
    let finite_vec = |v: &Point3| finite(*v);
    match geometry {
        SemanticGeometry::Line { start, end } => {
            if !finite(*start) || !finite(*end) {
                return Err(degenerate("line has a non-finite endpoint"));
            }
            if length(Point3 {
                x: end.x - start.x,
                y: end.y - start.y,
                z: end.z - start.z,
            }) < MIN_GEOMETRY_EXTENT
            {
                return Err(degenerate("line is zero length"));
            }
        }
        SemanticGeometry::Polyline { points, bulges, .. } => {
            if points.len() < 2 {
                return Err(degenerate("polyline has fewer than two points"));
            }
            if !points.iter().all(|p| finite(*p)) {
                return Err(degenerate("polyline has a non-finite point"));
            }
            if !bulges.iter().all(|b| b.is_finite()) {
                return Err(degenerate("polyline has a non-finite bulge"));
            }
        }
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            if !finite(*center) || !finite(*normal) {
                return Err(degenerate("circle has a non-finite centre or normal"));
            }
            if length(*normal) < MIN_GEOMETRY_EXTENT {
                return Err(degenerate("circle has a zero-length normal"));
            }
            if !radius.is_finite() || *radius <= 0.0 {
                return Err(degenerate("circle radius must be positive and finite"));
            }
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            if !finite(*center) || !finite(*normal) {
                return Err(degenerate("arc has a non-finite centre or normal"));
            }
            if length(*normal) < MIN_GEOMETRY_EXTENT {
                return Err(degenerate("arc has a zero-length normal"));
            }
            if !radius.is_finite() || *radius <= 0.0 {
                return Err(degenerate("arc radius must be positive and finite"));
            }
            if !start.is_finite() || !sweep.is_finite() {
                return Err(degenerate("arc has a non-finite start or sweep"));
            }
            if sweep.abs() < MIN_GEOMETRY_EXTENT {
                return Err(degenerate("arc has a zero sweep"));
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
            if !finite(*center) || !finite(*normal) || !finite_vec(major_axis) {
                return Err(degenerate("ellipse has a non-finite frame"));
            }
            if length(*normal) < MIN_GEOMETRY_EXTENT {
                return Err(degenerate("ellipse has a zero-length normal"));
            }
            if length(*major_axis) < MIN_GEOMETRY_EXTENT {
                return Err(degenerate("ellipse has a collapsed major axis"));
            }
            if !ratio.is_finite() || *ratio <= 0.0 {
                return Err(degenerate("ellipse ratio must be positive and finite"));
            }
            if !start.is_finite() || !sweep.is_finite() {
                return Err(degenerate("ellipse has a non-finite start or sweep"));
            }
            // The normal must not be parallel to the major axis, or the plane is
            // undefined. `ellipse_minor_dir` has a fallback, but storing such an
            // ellipse would be silently ambiguous.
            let unit_major = crate::math::scale(*major_axis, 1.0 / length(*major_axis));
            if length(cross(crate::math::normalize(*normal), unit_major)) < MIN_GEOMETRY_EXTENT {
                return Err(degenerate("ellipse normal is parallel to its major axis"));
            }
        }
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            if *degree < 1 {
                return Err(degenerate("spline degree must be at least 1"));
            }
            if control_points.len() < (*degree as usize + 1) {
                return Err(degenerate(
                    "spline has fewer control points than its degree requires",
                ));
            }
            if !control_points.iter().all(|p| finite(*p)) {
                return Err(degenerate("spline has a non-finite control point"));
            }
            if !knots.iter().all(|k| k.is_finite()) {
                return Err(degenerate("spline has a non-finite knot"));
            }
            if !weights.iter().all(|w| w.is_finite() && *w > 0.0) {
                return Err(degenerate("spline has a non-positive or non-finite weight"));
            }
        }
        SemanticGeometry::Point(p) => {
            if !finite(*p) {
                return Err(degenerate("point coordinate is non-finite"));
            }
        }
        SemanticGeometry::Mesh(mesh) => {
            validate_mesh(mesh)?;
        }
        SemanticGeometry::Insert { transform, .. } => {
            if !transform_is_finite(transform) {
                return Err(degenerate("insert transform is non-finite"));
            }
            if transform.determinant().abs() < MIN_GEOMETRY_EXTENT {
                return Err(degenerate("insert transform is singular"));
            }
        }
        SemanticGeometry::Text {
            position,
            height,
            rotation,
            ..
        } => {
            if !finite(*position) {
                return Err(degenerate("text position is non-finite"));
            }
            if !height.is_finite() || *height <= 0.0 {
                return Err(degenerate("text height must be positive and finite"));
            }
            if !rotation.is_finite() {
                return Err(degenerate("text rotation is non-finite"));
            }
        }
        SemanticGeometry::Opaque { .. } => {
            // Opaque geometry has no numeric content to judge; a style-only edit
            // of an imported proxy must remain possible.
        }
        SemanticGeometry::Compound(children) => {
            if children.is_empty() {
                return Err(degenerate("compound geometry has no children"));
            }
            for child in children {
                validate_geometry(child)?;
            }
        }
    }
    Ok(())
}

fn validate_mesh(mesh: &Mesh) -> CadResult<()> {
    let finite = |p: &Point3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
    if !mesh.vertices.iter().all(finite) {
        return Err(degenerate("mesh has a non-finite vertex"));
    }
    if !mesh.normals.iter().all(finite) {
        return Err(degenerate("mesh has a non-finite normal"));
    }
    if !mesh.colors.is_empty() && mesh.colors.len() != mesh.vertices.len() {
        return Err(degenerate(
            "mesh per-vertex colours do not match the vertex count",
        ));
    }
    let count = mesh.vertices.len() as u32;
    for tri in &mesh.triangles {
        if tri.iter().any(|i| *i >= count) {
            return Err(degenerate("mesh triangle references an unknown vertex"));
        }
        if tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] {
            return Err(degenerate("mesh triangle is degenerate"));
        }
    }
    Ok(())
}

/// Validate a whole entity before it is written.
///
/// Beyond [`validate_geometry`] this enforces the database invariants:
/// - the change key (`id`) and the entity's own `id` agree, and
///   `object.id.0 == id.0` — the documented identity relationship is that an
///   entity's `ObjectId` and `EntityId` share the same numeric value (the
///   importer allocates both from the same counter);
/// - the entity's layer exists;
/// - the entity's space is a real space: `Model`, an existing `Paper` layout, or
///   an existing `Block` definition.
pub fn validate_entity(
    db: &crate::DrawingDatabase,
    id: EntityId,
    entity: &DbEntity,
) -> CadResult<()> {
    if entity.id != id {
        return Err(CadError::Invariant(format!(
            "change key {id:?} does not match entity id {:?}",
            entity.id
        )));
    }
    if entity.object.id.0 != id.0 {
        return Err(CadError::Invariant(format!(
            "entity {id:?} has object id {:?}; the object and entity id must share the same value",
            entity.object.id
        )));
    }
    validate_geometry(&entity.geometry)?;
    if !db.layers.contains_key(&entity.layer) {
        return Err(CadError::Invariant(format!(
            "entity {id:?} references missing layer {:?}",
            entity.layer
        )));
    }
    match &entity.space {
        SpaceId::Model => {}
        SpaceId::Paper(layout) => {
            if !db.layouts.contains_key(layout) {
                return Err(CadError::Invariant(format!(
                    "entity {id:?} references missing layout {layout:?}"
                )));
            }
        }
        SpaceId::Block(block) => {
            if !db.blocks.contains_key(block) {
                return Err(CadError::Invariant(format!(
                    "entity {id:?} references missing block {block:?}"
                )));
            }
        }
    }
    Ok(())
}

fn degenerate(message: &str) -> CadError {
    CadError::Invariant(format!("invalid entity geometry: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn rejects_non_finite_and_degenerate_geometry() {
        assert!(validate_geometry(&SemanticGeometry::Line {
            start: p(0.0, 0.0),
            end: p(1.0, 0.0)
        })
        .is_ok());
        assert!(validate_geometry(&SemanticGeometry::Line {
            start: p(0.0, 0.0),
            end: p(0.0, 0.0)
        })
        .is_err());
        assert!(validate_geometry(&SemanticGeometry::Line {
            start: p(f64::NAN, 0.0),
            end: p(1.0, 0.0)
        })
        .is_err());
        assert!(validate_geometry(&SemanticGeometry::Circle {
            center: p(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0
            },
            radius: 0.0
        })
        .is_err());
        assert!(validate_geometry(&SemanticGeometry::Polyline {
            points: vec![p(0.0, 0.0)],
            bulges: Vec::new(),
            closed: false
        })
        .is_err());
    }
}
