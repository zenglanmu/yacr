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

/// Stable reason code: a spline's **non-empty** knot vector has a length other
/// than `control_points.len() + degree + 1`, so control points cannot be indexed
/// into the knot vector. An empty knot vector is the sanctioned "clamped uniform"
/// shorthand and is exempt (see [`validate_geometry`]).
pub const REASON_SPLINE_KNOT_COUNT: &str = "spline.knot_count";

/// Stable reason code: a spline's weight vector is non-empty but its length is
/// not `control_points.len()`. An empty weight vector is the sanctioned
/// "all ones" shorthand and is exempt (see [`validate_geometry`]).
pub const REASON_SPLINE_WEIGHT_COUNT: &str = "spline.weight_count";

/// Stable reason code: a spline's knot vector decreases somewhere, so the
/// parameter domain is not a subdivision of one interval.
pub const REASON_SPLINE_KNOT_ORDER: &str = "spline.knot_order";

/// Validate a [`SemanticGeometry`] before it is written.
///
/// Rejects non-finite coordinates and degenerate/unsupported-for-write cases:
/// a zero-length line, a polyline with fewer than two points, a non-positive
/// circle/arc radius, a collapsed ellipse axis, a spline with too few control
/// points or an inconsistent knot/weight definition, a mesh with out-of-range or
/// repeated triangle indices, a text run with a non-positive height, and a
/// singular insert transform.
///
/// `Opaque` payloads and `Compound` are accepted recursively: an opaque entity
/// has no numeric content this build can judge, and rejecting it would block a
/// legitimate style edit of an imported proxy.
///
/// The spline arm additionally enforces the structural relations of a NURBS
/// definition (see [`REASON_SPLINE_KNOT_COUNT`], [`REASON_SPLINE_WEIGHT_COUNT`])
/// and knot non-decreasing order ([`REASON_SPLINE_KNOT_ORDER`]). Both length
/// relations apply to a **non-empty** vector only: `knots: []` is the sanctioned
/// "clamped uniform" shorthand produced by the MULTILEADER leader-path
/// constructors and expanded by `cad-geometry`'s tessellator, and `weights: []` is
/// the sanctioned "all ones" shorthand expanded by `cad-geometry::NurbsCurve::new`.
/// `cad-app`'s grip model (`cad-app/src/grips/mod.rs`, `spline_is_consistent`)
/// already answers "no control-point grips" for a definition it cannot index into
/// rather than guessing a repair; this validation makes that inconsistent state
/// unrepresentable through the database. The grip-side check stays as defence in
/// depth: it also guards geometry that never went through a validated write
/// (import build, derived display) and it refuses per drag instead of per write.
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
            // Structural NURBS relations. Both are pure length arithmetic on the
            // stored vectors, and both are required by every consumer that
            // indexes a control point together with its knot span and weight
            // (`cad-geometry::NurbsCurve`, `cad-app`'s control-point grips). A
            // definition that violates either cannot be evaluated or edited
            // faithfully, so it is refused here instead of being padded,
            // uniformised or otherwise repaired.
            //
            // Two **empty** vectors are sanctioned shorthand rather than malformed
            // data, and each is exempted only in its own respect:
            //
            // * `knots: []` means "clamped uniform". Two shipped constructors
            //   produce it deliberately — `cad-domain/src/multileader.rs`
            //   (`path_geometry`) and `cad-import-acadrust/src/dimension/
            //   multileader.rs` (`multileader_path`) — for a MULTILEADER leader
            //   path of type Spline. `cad-geometry`'s `tessellate_spline` expands
            //   it: `NurbsCurve::new` rejects the wrong-length vector, and the
            //   documented fallback then supplies
            //   `clamped_uniform_knots(n, degree.min(n - 1))` (the behaviour its own
            //   doc comment calls the "clamped-uniform convention"; for every
            //   producer here `degree <= n - 1`, so the degree is preserved).
            //   Refusing the empty vector would make an *imported* MULTILEADER
            //   unwritable, since this validator recurses into
            //   `MultiLeader::display()`.
            // * `weights: []` means "all ones": `NurbsCurve::new` expands it to
            //   `1.0` per control point, and the same constructors store it.
            //
            // So the length relations apply to the **non-empty** vector only; the
            // empty form inherits exactly the semantics the consumer already
            // implements (clamped uniform knots, unit weights) and nothing is
            // invented or filled in here. A *non-empty* vector of the wrong length
            // is still refused, and the two exemptions never cover for each other:
            // each vector is judged on its own.
            let n = control_points.len();
            let expected_knots = n + *degree as usize + 1;
            if !knots.is_empty() && knots.len() != expected_knots {
                return Err(degenerate_reason(
                    REASON_SPLINE_KNOT_COUNT,
                    &format!(
                        "a degree {degree} spline with {n} control points needs \
                         {expected_knots} knots (control points + degree + 1), but stores {}",
                        knots.len()
                    ),
                ));
            }
            if !weights.is_empty() && weights.len() != n {
                return Err(degenerate_reason(
                    REASON_SPLINE_WEIGHT_COUNT,
                    &format!(
                        "a spline with {n} control points needs {n} weights (one per control \
                         point), but stores {}",
                        weights.len()
                    ),
                ));
            }
            // The existing arm already reasons about knots (they must be finite),
            // so the order relation belongs here too: `NurbsCurve::new` refuses a
            // decreasing vector, and a decreasing vector would make the
            // parameter domain meaningless rather than merely unusual. An empty
            // vector has no window to compare and is trivially ordered, so the
            // clamped-uniform shorthand passes; the `position` and indexing below
            // only ever run on a non-empty vector.
            if let Some(index) = knots.windows(2).position(|w| w[1] < w[0]) {
                return Err(degenerate_reason(
                    REASON_SPLINE_KNOT_ORDER,
                    &format!(
                        "spline knots must be non-decreasing, but knot {index} ({}) is followed \
                         by knot {} ({})",
                        knots[index],
                        index + 1,
                        knots[index + 1]
                    ),
                ));
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
            wrap_width,
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
            // A stored boundary must be usable: zero/negative/non-finite would
            // silently degrade to "no wrap" at layout, so reject it here instead.
            if let Some(width) = wrap_width {
                if !width.is_finite() || *width <= 0.0 {
                    return Err(degenerate(
                        "text wrap width must be positive and finite when set",
                    ));
                }
            }
        }
        SemanticGeometry::Shape {
            position,
            size,
            rotation,
            ..
        } => {
            if !finite(*position) {
                return Err(degenerate("shape position is non-finite"));
            }
            if !size.is_finite() || *size <= 0.0 {
                return Err(degenerate("shape size must be positive and finite"));
            }
            if !rotation.is_finite() {
                return Err(degenerate("shape rotation is non-finite"));
            }
        }
        SemanticGeometry::Opaque { .. } => {
            // Opaque geometry has no numeric content to judge; a style-only edit
            // of an imported proxy must remain possible.
        }
        SemanticGeometry::Image {
            origin,
            u,
            v,
            pixels,
            ..
        } => {
            if !finite(*origin) || !finite(*u) || !finite(*v) {
                return Err(degenerate("image has a non-finite placement"));
            }
            if !pixels[0].is_finite()
                || !pixels[1].is_finite()
                || pixels[0] <= 0.0
                || pixels[1] <= 0.0
            {
                return Err(degenerate(
                    "image pixel dimensions must be positive and finite",
                ));
            }
        }
        SemanticGeometry::Mask { boundary, .. } => {
            if boundary.len() < 3 {
                return Err(degenerate("mask boundary has fewer than three points"));
            }
            if !boundary.iter().all(|p| finite(*p)) {
                return Err(degenerate("mask boundary has a non-finite point"));
            }
        }
        SemanticGeometry::Compound(children) => {
            if children.is_empty() {
                return Err(degenerate("compound geometry has no children"));
            }
            for child in children {
                validate_geometry(child)?;
            }
        }
        SemanticGeometry::Dimension(dimension) => {
            // Definition points and the measurement must be finite; an empty
            // display is legal (a degenerate dimension that draws nothing).
            if !dimension.definition().is_consistent() {
                return Err(degenerate(
                    "dimension definition is inconsistent or has a non-finite point",
                ));
            }
            if !dimension.measurement().is_finite() {
                return Err(degenerate("dimension measurement is non-finite"));
            }
            for child in dimension.display() {
                validate_geometry(child)?;
            }
        }
        SemanticGeometry::MultiLeader(multileader) => {
            // The style snapshot, text, roots, lines and plane must be finite; a
            // leader line needs at least one vertex (a source line with fewer is
            // legal only because it draws nothing, so it is not rejected). The
            // derived display is validated like a compound (ADR 0019 C/G).
            let style = multileader.style();
            if !style.landing_distance.is_finite()
                || !style.landing_gap.is_finite()
                || !style.arrowhead_size.is_finite()
                || !style.text_height.is_finite()
                || !style.scale_factor.is_finite()
            {
                return Err(degenerate("multileader style snapshot is non-finite"));
            }
            let text = multileader.text();
            if !finite(text.location)
                || !text.rotation.is_finite()
                || !text.height.is_finite()
                || !text.width.is_finite()
            {
                return Err(degenerate("multileader text is non-finite"));
            }
            for root in multileader.roots() {
                if !finite(root.landing_point)
                    || !finite(root.dogleg_vector)
                    || !root.dogleg_length.is_finite()
                {
                    return Err(degenerate("multileader root is non-finite"));
                }
            }
            for line in multileader.lines() {
                if !line.points.iter().all(|p| finite(*p)) || !line.arrowhead_size.is_finite() {
                    return Err(degenerate("multileader leader line is non-finite"));
                }
            }
            let plane = multileader.plane();
            if !finite(plane.origin) || !finite(plane.u) || !finite(plane.v) {
                return Err(degenerate("multileader plane frame is non-finite"));
            }
            for child in multileader.display() {
                validate_geometry(child)?;
            }
        }
        SemanticGeometry::Hatch(hatch) => {
            // A hatch must have at least one loop with three finite vertices;
            // bulges and seeds are finite; the derived display is validated like
            // a compound. An empty display is legal only when every loop is
            // still drawn (the boundary loops are always part of it).
            let plane = hatch.plane();
            if !finite(plane.normal) || !plane.elevation.is_finite() {
                return Err(degenerate("hatch plane is non-finite"));
            }
            if hatch.loops().is_empty() {
                return Err(degenerate("hatch has no boundary loops"));
            }
            for loop_ in hatch.loops() {
                if loop_.points.len() < 3 {
                    return Err(degenerate(
                        "hatch boundary loop has fewer than three vertices",
                    ));
                }
                if !loop_
                    .points
                    .iter()
                    .all(|p| p[0].is_finite() && p[1].is_finite())
                {
                    return Err(degenerate("hatch boundary loop has a non-finite point"));
                }
                if !loop_.bulges.iter().all(|b| b.is_finite()) {
                    return Err(degenerate("hatch boundary loop has a non-finite bulge"));
                }
            }
            if !hatch
                .seeds()
                .iter()
                .all(|p| p[0].is_finite() && p[1].is_finite())
            {
                return Err(degenerate("hatch seed point is non-finite"));
            }
            for child in hatch.display() {
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

/// A degenerate-geometry refusal that carries a stable reason code.
///
/// The shape is `<category>: <code>: <detail>`, the same "code then colon then
/// message" form the layer refusal codes use (`layer.name_empty`,
/// `layer.duplicate_name`): the code is the machine-stable part, the detail names
/// the actual values so a caller can explain the refusal without guessing.
fn degenerate_reason(code: &str, detail: &str) -> CadError {
    CadError::Invariant(format!("invalid entity geometry: {code}: {detail}"))
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
        // A stored wrap boundary must be usable; `None` means "no wrap".
        let text = |wrap_width| SemanticGeometry::Text {
            text: "A".into(),
            position: p(0.0, 0.0),
            style: cad_domain::StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: cad_domain::TextAlignH::Left,
            v_align: cad_domain::TextAlignV::Baseline,
            wrap_width,
        };
        assert!(validate_geometry(&text(None)).is_ok());
        assert!(validate_geometry(&text(Some(4.0))).is_ok());
        assert!(validate_geometry(&text(Some(0.0))).is_err());
        assert!(validate_geometry(&text(Some(-1.0))).is_err());
        assert!(validate_geometry(&text(Some(f64::NAN))).is_err());
    }

    #[test]
    fn accepts_well_formed_image_and_mask() {
        let image = SemanticGeometry::Image {
            origin: p(1.0, 2.0),
            u: p(0.5, 0.0),
            v: p(0.0, 0.25),
            pixels: [640.0, 480.0],
            file: None,
            clip: None,
            visible: true,
        };
        assert!(validate_geometry(&image).is_ok());

        let mask = SemanticGeometry::Mask {
            boundary: vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)],
            inverted: false,
        };
        assert!(validate_geometry(&mask).is_ok());
    }

    #[test]
    fn rejects_malformed_image_and_mask() {
        // An image with non-positive pixel dimensions cannot be placed.
        assert!(validate_geometry(&SemanticGeometry::Image {
            origin: p(0.0, 0.0),
            u: p(1.0, 0.0),
            v: p(0.0, 1.0),
            pixels: [0.0, 10.0],
            file: None,
            clip: None,
            visible: true,
        })
        .is_err());
        // A non-finite edge vector is rejected.
        assert!(validate_geometry(&SemanticGeometry::Image {
            origin: p(0.0, 0.0),
            u: p(f64::NAN, 0.0),
            v: p(0.0, 1.0),
            pixels: [10.0, 10.0],
            file: None,
            clip: None,
            visible: true,
        })
        .is_err());
        // A mask needs at least three points to bound an area.
        assert!(validate_geometry(&SemanticGeometry::Mask {
            boundary: vec![p(0.0, 0.0), p(1.0, 1.0)],
            inverted: false,
        })
        .is_err());
    }
}
