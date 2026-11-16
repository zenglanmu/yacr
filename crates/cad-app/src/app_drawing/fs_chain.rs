//! The two `FS` contact passes: `FSMODE=OFF` (direct) and `FSMODE=ON` (chain).
//!
//! `FS` is the Express "add every object that touches the selected object" tool.
//! Both passes are **read-only**: the caller mutates only the session selection, so
//! there is no transaction, no revision bump and no undo entry to keep to one step.
//!
//! # `FSMODE=OFF` (the default, and the pre-`FSMODE` behaviour)
//!
//! Every visible, directly addressable model-space curve that touches the seed
//! **directly** is added. A touch is either an intersection (crossing or
//! tangency) or a pair of coincident endpoints; nothing is transitive.
//!
//! # `FSMODE=ON` (the tangent chain)
//!
//! Starting at the seed, entities are added while each new entity is connected to
//! the current frontier by a **tangent** contact, so a chain of tangent Fillet-like
//! contacts is selected as one set. The pass is deliberately strict — an
//! ambiguous or non-tangent continuation is never guessed:
//!
//! * a contact pair whose tangents at the contact point are not parallel within
//!   [`TANGENT_ANGLE_TOLERANCE`] is the named `fs.chain_nontangent` refusal;
//! * two distinct candidates tangent to the same frontier entity at the same
//!   contact point are the named `fs.chain_ambiguous` refusal (which one continues
//!   the chain is undefined);
//! * a chain longer than [`FS_CHAIN_MAX_ENTITIES`] entities is the named
//!   `fs.chain_bound` refusal instead of a silently truncated selection.
//!
//! Only links to entities that are **not yet** in the chain are classified, so an
//! ordinary crossing between two already-chained entities does not abort the pass.
//!
//! # Declared limits
//!
//! * the contact set is the same one the direct pass uses (engine intersection +
//!   coincident endpoints), so `FS` never claims a contact its intersection path
//!   cannot see;
//! * the tangent direction is computed analytically per supported kind (`LINE`,
//!   `CIRCLE`, `ARC`, `ELLIPSE`, `LWPOLYLINE` including bulges, `SPLINE` through
//!   the exact NURBS derivative), never by sampling a display tessellation;
//! * parallel **and** antiparallel tangents count as one tangent line: the test is
//!   "the contact is tangential", not "the traversal directions agree", so an
//!   orientation-reversing cusp is not distinguished from a smooth continuation;
//! * only tangent contacts continue the chain; any other contact of a candidate is
//!   invisible to the pass (the chain is a subgraph, not a replacement selection).

use std::collections::BTreeSet;

use super::*;
use crate::picking::ref_visibility;
use crate::selection::model_ref;

use cad_domain::TolerancePolicy;
use cad_geometry::{
    bulge_arc_geometry, cross, distance, dot, length, normalize, scale, sub, DefaultGeometryEngine,
    GeometryEngine,
};

/// The documented `FSMODE=ON` chain bound: at most this many entities.
///
/// The bound exists so a pathological tangent network cannot build an unbounded
/// selection through repeated expansion; reaching it is a named refusal, never a
/// truncated chain.
pub(crate) const FS_CHAIN_MAX_ENTITIES: usize = 64;

/// Two tangents count as the same tangent line within this angle (radians).
///
/// `1e-3` rad (~0.057°) is far below any drawn corner and far above the direction
/// error of an analytically evaluated tangent, so a real tangency is accepted and a
/// 1° corner is not.
const TANGENT_ANGLE_TOLERANCE: f64 = 1e-3;

/// The result of one contact pass.
pub(crate) struct ContactPass {
    /// The visible entities to add, in drawing (chain: breadth-first) order.
    pub(crate) refs: Vec<SelectionRef>,
    /// Number of contacts the pass used (chain) or entities it touched (direct).
    pub(crate) contacts: usize,
    /// Number of entities in the pass including the seed.
    pub(crate) entities: usize,
}

/// `FSMODE=OFF`: every entity the seed touches directly.
pub(crate) fn direct_touching(
    drawing: &DrawingDatabase,
    session: &SessionState,
    document: DocumentId,
    root: cad_db::LayerScope,
    seed: &DbEntity,
    engine: &DefaultGeometryEngine,
    policy: &TolerancePolicy,
) -> CadResult<ContactPass> {
    let mut refs = Vec::new();
    for candidate in drawing.model_space() {
        if candidate.id == seed.id || !fs_supported(&candidate.geometry) {
            continue;
        }
        let reference = model_ref(document, candidate.id);
        if !ref_visibility(
            drawing,
            &session.layer_overrides,
            &session.hidden_entities,
            root,
            &reference,
        )
        .unwrap_or(false)
        {
            continue;
        }
        let points = contact_points(
            engine,
            &seed.geometry,
            &candidate.geometry,
            seed.id,
            candidate.id,
            policy,
        )?;
        if points.is_empty() {
            continue;
        }
        refs.push(reference);
    }
    let contacts = refs.len();
    Ok(ContactPass {
        refs,
        contacts,
        entities: contacts + 1,
    })
}

/// `FSMODE=ON`: the transitive tangent chain that starts at the seed.
pub(crate) fn tangent_chain(
    drawing: &DrawingDatabase,
    session: &SessionState,
    document: DocumentId,
    root: cad_db::LayerScope,
    seed: &DbEntity,
    engine: &DefaultGeometryEngine,
    policy: &TolerancePolicy,
) -> CadResult<ContactPass> {
    let tolerance = contact_tolerance(policy);
    let mut visited: BTreeSet<EntityId> = BTreeSet::new();
    visited.insert(seed.id);
    let mut frontier = vec![seed.id];
    let mut refs = Vec::new();
    let mut contacts = 0usize;

    while !frontier.is_empty() {
        let mut next = Vec::new();
        for frontier_id in std::mem::take(&mut frontier) {
            let geometry = &drawing
                .entity(frontier_id)
                .ok_or_else(|| {
                    refusal(
                        "fs.chain_not_addressable",
                        format!("chain entity {} is no longer present", frontier_id.0),
                    )
                })?
                .geometry;
            let mut continuations: Vec<(EntityId, SelectionRef, Vec<Point3>)> = Vec::new();
            for candidate in drawing.model_space() {
                if visited.contains(&candidate.id) || !fs_supported(&candidate.geometry) {
                    continue;
                }
                let reference = model_ref(document, candidate.id);
                if !ref_visibility(
                    drawing,
                    &session.layer_overrides,
                    &session.hidden_entities,
                    root,
                    &reference,
                )
                .unwrap_or(false)
                {
                    continue;
                }
                let points = contact_points(
                    engine,
                    geometry,
                    &candidate.geometry,
                    frontier_id,
                    candidate.id,
                    policy,
                )?;
                if points.is_empty() {
                    continue;
                }
                for point in &points {
                    ensure_tangent(
                        geometry,
                        &candidate.geometry,
                        *point,
                        frontier_id,
                        candidate.id,
                    )?;
                }
                continuations.push((candidate.id, reference, points));
            }
            for (index, left) in continuations.iter().enumerate() {
                for right in continuations.iter().skip(index + 1) {
                    if shares_contact(&left.2, &right.2, tolerance) {
                        return Err(refusal(
                            "fs.chain_ambiguous",
                            format!(
                                "entities {} and {} are both tangent to entity {} at the same \
                                 contact point; FSMODE=ON refuses an ambiguous branch",
                                left.0 .0, right.0 .0, frontier_id.0
                            ),
                        ));
                    }
                }
            }
            for (id, reference, points) in continuations {
                contacts += points.len();
                if visited.insert(id) {
                    refs.push(reference);
                    next.push(id);
                }
            }
        }
        if visited.len() > FS_CHAIN_MAX_ENTITIES {
            return Err(refusal(
                "fs.chain_bound",
                format!(
                    "the FSMODE=ON chain reached {} entities, past the documented bound of \
                     {FS_CHAIN_MAX_ENTITIES}; nothing was selected",
                    visited.len()
                ),
            ));
        }
        frontier = next;
    }
    Ok(ContactPass {
        refs,
        entities: visited.len(),
        contacts,
    })
}

/// The geometry kinds `FS` intersects.
///
/// Other kinds (text, hatch, dimensions, blocks, images, opaque data, meshes,
/// compounds) are not curve-touch candidates and are skipped, never silently
/// truncated inside a supported kind.
pub(crate) fn fs_supported(geometry: &SemanticGeometry) -> bool {
    matches!(
        geometry,
        SemanticGeometry::Line { .. }
            | SemanticGeometry::Circle { .. }
            | SemanticGeometry::Arc { .. }
            | SemanticGeometry::Ellipse { .. }
            | SemanticGeometry::Polyline { .. }
            | SemanticGeometry::Spline { .. }
    )
}

/// Every point at which `a` and `b` touch, de-duplicated.
///
/// The engine intersection is the truth for a crossing or tangential crossing; a
/// pair of coincident endpoints that the intersection path does not report (two
/// collinear segments sharing only an endpoint, for example) is added explicitly,
/// which is exactly the union the direct pass always used.
fn contact_points(
    engine: &DefaultGeometryEngine,
    a: &SemanticGeometry,
    b: &SemanticGeometry,
    a_id: EntityId,
    b_id: EntityId,
    policy: &TolerancePolicy,
) -> CadResult<Vec<Point3>> {
    let mut points = engine.intersect_local(a, b, policy).map_err(|error| {
        refusal(
            "fs.intersect",
            format!(
                "entities {} and {} could not be intersected: {error}",
                a_id.0, b_id.0
            ),
        )
    })?;
    let tolerance = contact_tolerance(policy);
    let a_ends = curve_endpoints(engine, a, policy)?;
    if !a_ends.is_empty() {
        let b_ends = curve_endpoints(engine, b, policy)?;
        for start in &a_ends {
            for end in &b_ends {
                if distance(*start, *end) <= tolerance {
                    points.push(*start);
                }
            }
        }
    }
    let mut unique: Vec<Point3> = Vec::with_capacity(points.len());
    for point in points {
        if !unique
            .iter()
            .any(|kept| distance(*kept, point) <= tolerance)
        {
            unique.push(point);
        }
    }
    Ok(unique)
}

/// Whether the contact of `a` and `b` at `point` is tangential.
///
/// A refusal names both entities, the contact point and the measured angle: the
/// pass never silently drops a non-tangent branch.
fn ensure_tangent(
    a: &SemanticGeometry,
    b: &SemanticGeometry,
    point: Point3,
    a_id: EntityId,
    b_id: EntityId,
) -> CadResult<()> {
    let (Some(direction_a), Some(direction_b)) = (tangent_at(a, point), tangent_at(b, point))
    else {
        return Err(refusal(
            "fs.chain_nontangent",
            format!(
                "entities {} and {} meet at ({}, {}, {}) but the contact tangent is undefined; \
                 FSMODE=ON chains resolved tangent contacts only",
                a_id.0, b_id.0, point.x, point.y, point.z
            ),
        ));
    };
    // Both directions are unit vectors, so |cross| is |sin(angle)|.
    let sine = length(cross(direction_a, direction_b));
    if !sine.is_finite() || sine > TANGENT_ANGLE_TOLERANCE {
        let degrees = if sine.is_finite() {
            sine.clamp(0.0, 1.0).asin().to_degrees()
        } else {
            f64::NAN
        };
        return Err(refusal(
            "fs.chain_nontangent",
            format!(
                "entities {} and {} meet at ({}, {}, {}) at {degrees:.3}°, not tangentially; \
                 FSMODE=ON chains tangent contacts only",
                a_id.0, b_id.0, point.x, point.y, point.z
            ),
        ));
    }
    Ok(())
}

/// The unit tangent direction of `geometry` at a point on it.
///
/// Exact per kind: a line/segment direction, the radius-perpendicular of a circle
/// or arc, the parametric derivative of an ellipse, the segment (or bulge-arc)
/// direction of a polyline, and the exact NURBS first derivative of a spline.
/// `None` means the direction cannot be resolved (degenerate geometry), which the
/// caller reports as the non-tangent refusal instead of inventing a direction.
fn tangent_at(geometry: &SemanticGeometry, point: Point3) -> Option<Point3> {
    let direction = match geometry {
        SemanticGeometry::Line { start, end } => unit(sub(*end, *start)),
        SemanticGeometry::Circle { center, normal, .. }
        | SemanticGeometry::Arc { center, normal, .. } => unit(cross(*normal, sub(point, *center))),
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            ..
        } => {
            let semi_major = length(*major_axis);
            if !semi_major.is_finite() || semi_major <= 0.0 {
                return None;
            }
            if !ratio.is_finite() || *ratio <= 0.0 {
                return None;
            }
            let major_unit = unit(*major_axis)?;
            let minor_unit = unit(cross(unit(*normal)?, *major_axis))?;
            let offset = sub(point, *center);
            let cos = dot(offset, major_unit) / semi_major;
            let sin = dot(offset, minor_unit) / (semi_major * *ratio);
            // P(t) = C + a·cos t·û + b·sin t·v̂  ⇒  P'(t) = −a·sin t·û + b·cos t·v̂.
            let along_major = scale(major_unit, semi_major * sin);
            let along_minor = scale(minor_unit, semi_major * *ratio * cos);
            unit(sub(along_minor, along_major))
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            ..
        } => polyline_tangent(points, bulges, *closed, point),
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
            ..
        } => {
            let curve =
                cad_geometry::expand_spline_source(*degree, knots, control_points, weights).ok()?;
            let (parameter, _) = cad_geometry::nearest_point_parameter(&curve, point)?;
            unit(curve.tangent(parameter))
        }
        _ => return None,
    };
    let direction = direction?;
    if length(direction) > 0.5 {
        Some(direction)
    } else {
        None
    }
}

/// The unit tangent of a polyline at `point`, from its nearest edge.
fn polyline_tangent(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    point: Point3,
) -> Option<Point3> {
    let count = points.len();
    let edges = if closed {
        count
    } else {
        count.saturating_sub(1)
    };
    if edges == 0 {
        return None;
    }
    let mut best = f64::INFINITY;
    let mut direction = None;
    for index in 0..edges {
        let start = points[index];
        let end = points[(index + 1) % count];
        let bulge = bulges.get(index).copied().unwrap_or(0.0);
        let (gap, candidate) = if bulge == 0.0 || !bulge.is_finite() {
            let direction = unit(sub(end, start))?;
            let gap = distance_to_segment(point, start, end);
            (gap, direction)
        } else {
            match bulge_arc_geometry(
                start,
                end,
                bulge,
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
            ) {
                Some(SemanticGeometry::Arc { center, normal, .. }) => {
                    let direction = unit(cross(normal, sub(point, center)))?;
                    let gap = distance_to_arc(point, center, normal, start, end);
                    (gap, direction)
                }
                _ => {
                    let direction = unit(sub(end, start))?;
                    let gap = distance_to_segment(point, start, end);
                    (gap, direction)
                }
            }
        };
        if gap < best {
            best = gap;
            direction = Some(candidate);
        }
    }
    direction
}

/// The exact point-to-segment distance.
fn distance_to_segment(point: Point3, start: Point3, end: Point3) -> f64 {
    let chord = sub(end, start);
    let length_squared = dot(chord, chord);
    if length_squared <= f64::MIN_POSITIVE {
        return distance(point, start);
    }
    let t = (dot(sub(point, start), chord) / length_squared).clamp(0.0, 1.0);
    distance(
        point,
        Point3 {
            x: start.x + chord.x * t,
            y: start.y + chord.y * t,
            z: start.z + chord.z * t,
        },
    )
}

/// The point-to-bulge-arc distance: the radius gap inside the sweep, otherwise the
/// nearer endpoint.
fn distance_to_arc(
    point: Point3,
    center: Point3,
    normal: Point3,
    start: Point3,
    end: Point3,
) -> f64 {
    let radius = distance(center, start);
    let Some(axis) = unit(normal) else {
        return distance(point, start).min(distance(point, end));
    };
    let (axis_x, axis_y) = {
        let (x, y, _) = cad_geometry::arbitrary_axis(axis);
        (x, y)
    };
    let angle_of = |p: Point3| -> f64 {
        let offset = sub(p, center);
        dot(offset, axis_y).atan2(dot(offset, axis_x))
    };
    let mut sweep = angle_of(end) - angle_of(start);
    while sweep > std::f64::consts::PI {
        sweep -= std::f64::consts::TAU;
    }
    while sweep <= -std::f64::consts::PI {
        sweep += std::f64::consts::TAU;
    }
    let mut offset = angle_of(point) - angle_of(start);
    while offset > std::f64::consts::PI {
        offset -= std::f64::consts::TAU;
    }
    while offset <= -std::f64::consts::PI {
        offset += std::f64::consts::TAU;
    }
    let inside = if sweep >= 0.0 {
        offset >= 0.0 && offset <= sweep
    } else {
        offset <= 0.0 && offset >= sweep
    };
    if inside {
        (distance(point, center) - radius).abs()
    } else {
        distance(point, start).min(distance(point, end))
    }
}

/// The endpoint set of an open curve (a `LINE` uses its vertices, the rest their
/// tessellation ends).
fn curve_endpoints(
    engine: &DefaultGeometryEngine,
    geometry: &SemanticGeometry,
    policy: &TolerancePolicy,
) -> CadResult<Vec<Point3>> {
    match geometry {
        SemanticGeometry::Line { start, end } => Ok(vec![*start, *end]),
        SemanticGeometry::Arc { .. }
        | SemanticGeometry::Polyline { .. }
        | SemanticGeometry::Spline { .. } => {
            let points = engine.tessellate_curve(geometry, policy)?;
            let mut endpoints = Vec::new();
            if let Some(first) = points.first() {
                endpoints.push(*first);
            }
            if let Some(last) = points.last() {
                if endpoints.first() != Some(last) {
                    endpoints.push(*last);
                }
            }
            Ok(endpoints)
        }
        _ => Ok(Vec::new()),
    }
}

/// Whether two contact-point lists share a contact within `tolerance`.
fn shares_contact(left: &[Point3], right: &[Point3], tolerance: f64) -> bool {
    left.iter()
        .any(|p| right.iter().any(|q| distance(*p, *q) <= tolerance))
}

/// The contact tolerance: the engine's world computation tolerance.
fn contact_tolerance(policy: &TolerancePolicy) -> f64 {
    policy.computation_world.max(1e-9)
}

/// `normalize` without the zero-length passthrough: `None` for a zero vector.
fn unit(vector: Point3) -> Option<Point3> {
    let magnitude = length(vector);
    (magnitude.is_finite() && magnitude > 0.0).then(|| normalize(vector))
}

/// A command refusal carrying its stable reason code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}
