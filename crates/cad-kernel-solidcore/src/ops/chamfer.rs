//! Chamfer: one edge cut back by a distance along each of its two faces.
//!
//! P2.0 is deliberately the smallest honest chamfer: a **straight** section on a
//! **straight** edge. The body it cuts may be planar or curved — the lift writes
//! either — but the selector resolves a **shared** `Line` edge only, so a cylinder
//! has nothing to pick (its one Line edge is the seam the writer reconstructs, and
//! the user never named that). The kernel does the geometry
//! (`solidcore_ops::chamfer_sides_ordered`); this module only lifts, resolves the
//! caller's point to an edge, drives the call and maps its refusals.
//!
//! # Which edge, and why a point
//!
//! The seam carries no kernel record ids (they are not part of the neutral
//! B-rep), so [`SolidEdgeSelector::NearestToPoint`] resolves against the lifted
//! record instead. Only a **shared** `Line` edge is pickable: a curve family this
//! module cannot measure exactly is skipped, never guessed, and so is a **seam**
//! (both uses on one face), a surface that exists only in the record and sits
//! exactly where a user clicks a rim. An edge is accepted only when it lies within
//! `max(1e-9, 1e-3 · bounding-box diagonal)` of the point: enough to absorb a
//! pick at a vertex or a snapped point, far too little to silently cut a
//! different edge. Anything farther is `solid.edge_not_found`. Ties are broken
//! by record index, so the same request on the same body always picks the same
//! edge.
//!
//! # Face of the first distance
//!
//! `chamfer_sides_ordered` needs the face the first distance is measured along.
//! `list_uses` appends each edge's uses in face-index → loop → position order, so
//! `record.edges[i].uses[0]` is the use on the **lowest-numbered face**; that is
//! deterministic for a given body. `second` defaults to `distance`, the symmetric
//! 45° chamfer; when it differs, the removed prism's cross-section has the two
//! set-backs as its legs (see the chain tests).
//!
//! # Refusals
//!
//! A curved body's face is not a refusal by itself (P2.1b part 2): the lift
//! writes it, and what stops a rim chamfer is the pick — `solid.edge_not_found`,
//! because a rim is a curve edge and the reconstructed seam is not selectable. A
//! chamfer the kernel will not build (too
//! large, no room, not blendable, no effect) is `Unsupported` with
//! [`crate::CHAMFER_FAILED`] — this build cannot build the requested chamfer
//! exactly, and an `Unsupported` outcome can never be mistaken for a body. A
//! kernel-integrity failure (or a caught panic) stays `Failed`, and cancellation
//! is [`cad_domain::CadError::Cancelled`] with no product.

use cad_domain::{BrepData, CadError, CadResult, Point3};
use cad_kernel_adapter::{SolidEdgeSelector, SolidOperationFamily, SolidOutcome};
use solidcore_ops::{Body, ChamferSecond};
use solidcore_record::brep::{Brep as Record, Curve, Edge};

use super::{body_outcome, lift, unlift, watched};
use crate::error::{self, LiftRefusal};
use crate::record::{add, dot, length, mul, point, sub};
use crate::{EDGE_NOT_FOUND, EMPTY_RESULT, LIFT_UNSUPPORTED};

const FAMILY: SolidOperationFamily = SolidOperationFamily::Chamfer;

/// The pick tolerance as a fraction of the body's bounding-box diagonal.
const SELECT_RELATIVE: f64 = 1e-3;

/// The pick tolerance floor, so a degenerate/zero-extent record still accepts a
/// point exactly on an edge.
const SELECT_FLOOR: f64 = 1e-9;

/// Cut the selected edge of `brep` with a straight chamfer.
pub(crate) fn chamfer(
    brep: &BrepData,
    selector: SolidEdgeSelector,
    distance: f64,
    second: Option<f64>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> CadResult<SolidOutcome> {
    let record = match lift(brep) {
        Ok(record) => record,
        Err(refusal) => return Ok(error::unsupported(refusal, FAMILY)),
    };
    let (body, order) = match Body::from_record(&record, false) {
        Ok(body) => body,
        Err(error) => return error::from_ops(error, FAMILY),
    };

    let at = selector.point();
    let tolerance = (SELECT_RELATIVE * bbox_diagonal(&record)).max(SELECT_FLOOR);
    let Some((edge, found)) = nearest_edge(&record, at) else {
        return Ok(not_found(
            "the lifted body has no pickable straight edge near that point (a seam, which only \
             closes the record, and a non-Line edge are not pickable)",
        ));
    };
    if !found.is_finite() || found > tolerance {
        return Ok(not_found(format!(
            "no edge lies within {tolerance} of the selector point ({}, {}, {}); \
             the nearest is {found} away",
            at.x, at.y, at.z
        )));
    }
    let Some(primary) = record.edges[edge].uses.first() else {
        // A wire edge cannot survive the closed-shell lift, so this is a record
        // that this seam should not have accepted: refuse rather than guess a
        // face and cut the wrong side.
        return Ok(not_found(format!(
            "edge {edge} has no face use to chamfer along"
        )));
    };
    let face = primary.face as usize;

    // Last chance to drop the work before the kernel starts.
    if cancelled() {
        return Err(CadError::Cancelled);
    }
    let second = ChamferSecond::Distance(second.unwrap_or(distance));
    let result = watched(cancelled, || {
        solidcore_ops::chamfer_sides_ordered(&body, &order, &[edge], &[face], distance, second)
    });

    // A result that arrived after cancellation is dropped: it is not a product.
    if cancelled() {
        return Err(CadError::Cancelled);
    }
    match result {
        Ok(()) => finish(&body),
        Err(error) => error::from_chamfer(error),
    }
}

/// Read the chamfered body back into the neutral model, refusing anything a
/// later lift could not consume.
fn finish(body: &Body) -> CadResult<SolidOutcome> {
    // A `to_record` failure is a kernel-integrity failure, so it takes the
    // generic `from_ops` mapping (`Failed`/`solid.kernel_failed`), not the
    // chamfer-specific geometric refusal.
    let record = match body.to_record() {
        Ok((record, _order)) => record,
        Err(error) => return error::from_ops(error, FAMILY),
    };
    let brep = match unlift(&record) {
        Ok(brep) => brep,
        Err(refusal) => return Ok(error::unsupported(refusal, FAMILY)),
    };
    if brep.face_count() == 0 {
        return Ok(error::failed(EMPTY_RESULT, "the chamfer produced no face"));
    }
    if brep.shells.len() != 1 {
        return Ok(error::unsupported(
            LiftRefusal::new(
                LIFT_UNSUPPORTED,
                format!(
                    "the chamfer produced {} shells; this seam hands out exactly one closed \
                     shell, and a second one would only be refused by its next lift",
                    brep.shells.len()
                ),
            ),
            FAMILY,
        ));
    }
    Ok(body_outcome(brep))
}

/// An `Unsupported` outcome with the stable "no edge picked" code.
fn not_found(detail: impl Into<String>) -> SolidOutcome {
    error::unsupported(LiftRefusal::new(EDGE_NOT_FOUND, detail), FAMILY)
}

/// The nearest pickable edge and its distance; the lowest record index wins a
/// tie, so the same body and point always pick the same edge.
///
/// A **seam** is not pickable. A cylindrical, conical, spherical or toroidal face
/// has no seam in the neutral model — it is reconstructed when the face is written
/// (`record::curved`) — and it sits exactly where a user clicks a rim. Handing it
/// to the kernel would cut nothing and answer "this edge has no blend (Smooth)",
/// naming an edge the user never selected. The test is exact: both uses of the
/// edge are on the **same** face.
fn nearest_edge(record: &Record, at: Point3) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for (index, edge) in record.edges.iter().enumerate() {
        if is_seam(edge) {
            continue;
        }
        let Some(distance) = edge_distance(edge, at) else {
            continue;
        };
        if best.is_none_or(|(_, nearest)| distance < nearest) {
            best = Some((index, distance));
        }
    }
    best
}

/// Whether both uses of the edge are on one face: the marker of a reconstructed
/// seam (a cylinder's generator, a sphere's meridian), not a real shared edge.
fn is_seam(edge: &Edge) -> bool {
    match edge.uses.as_slice() {
        [first, second] => first.face == second.face,
        _ => false,
    }
}

/// Distance from `at` to the edge's curve, when this module can measure it.
///
/// The planar lift admits only `Line` edges (see `record/to_record.rs`), so that
/// is the only family that can win the pick. Anything else is skipped rather
/// than approximated with a made-up distance that could select the wrong edge.
fn edge_distance(edge: &Edge, at: Point3) -> Option<f64> {
    match &edge.curve {
        Curve::Line { start, end } => Some(point_segment(at, point(*start), point(*end))),
        _ => None,
    }
}

/// Point-to-segment distance (`[0, 1]` clamps to the segment's ends).
fn point_segment(at: Point3, start: Point3, end: Point3) -> f64 {
    let along = sub(end, start);
    let squared = dot(along, along);
    let t = if squared > 0.0 {
        (dot(sub(at, start), along) / squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    length(sub(at, add(start, mul(along, t))))
}

/// The lifted body's bounding-box diagonal, the scale the pick tolerance uses.
fn bbox_diagonal(record: &Record) -> f64 {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for vertex in &record.vertices {
        for (axis, value) in vertex.point.iter().enumerate() {
            min[axis] = min[axis].min(*value);
            max[axis] = max[axis].max(*value);
        }
    }
    let mut squared = 0.0;
    for axis in 0..3 {
        let extent = max[axis] - min[axis];
        if !extent.is_finite() {
            return 0.0;
        }
        squared += extent * extent;
    }
    squared.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 1e-12;

    fn at(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    #[test]
    fn point_segment_clamps_to_both_ends_and_measures_the_middle() {
        let start = at(0.0, 0.0, 0.0);
        let end = at(2.0, 0.0, 0.0);
        assert!((point_segment(at(1.0, 1.0, 0.0), start, end) - 1.0).abs() < EPSILON);
        assert!((point_segment(at(-1.0, 0.0, 0.0), start, end) - 1.0).abs() < EPSILON);
        assert!((point_segment(at(3.0, 0.0, 0.0), start, end) - 1.0).abs() < EPSILON);
        // A degenerate segment is a point.
        assert!((point_segment(at(1.0, 0.0, 0.0), start, start) - 1.0).abs() < EPSILON);
    }

    /// A seam — both uses on one face — is not pickable, while a real shared edge
    /// is. The lifted cylinder is the case the selector used to fall into: its only
    /// `Line` edge is the reconstructed seam, and it sits exactly where a user
    /// clicks the rim.
    #[test]
    fn a_seam_is_skipped_and_a_shared_edge_is_not() {
        let cylinder = solidcore_ops::make_cylinder([0.0, 0.0, 0.0], 1.0, 2.0).expect("cylinder");
        let (record, _) = cylinder.to_record().expect("record");
        let seams: Vec<usize> = record
            .edges
            .iter()
            .enumerate()
            .filter(|(_, edge)| is_seam(edge))
            .map(|(index, _)| index)
            .collect();
        assert!(
            !seams.is_empty(),
            "the cylinder record has a reconstructed seam"
        );
        for index in &seams {
            assert_eq!(
                record.edges[*index].uses[0].face,
                record.edges[*index].uses[1].face
            );
        }
        // Nothing at the rim's seam point is pickable any more.
        let rim = Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        };
        assert!(
            nearest_edge(&record, rim).is_none(),
            "the seam must not win the pick"
        );

        // A real edge of a planar body is still pickable, and it is not a seam.
        let boxed = crate::polyhedra::box_brep(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            1.0,
            1.0,
            1.0,
        );
        let record =
            crate::record::to_record(&boxed, crate::record::DEFAULT_TOLERANCE).expect("box lifts");
        assert!(record.edges.iter().all(|edge| !is_seam(edge)));
        let corner = Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.5,
        };
        let (_, distance) = nearest_edge(&record, corner).expect("a box edge is pickable");
        assert!(distance <= 1e-12, "{distance}");
    }
}
