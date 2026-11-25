//! One-face offset: the atomic operation behind SketchUp's Push/Pull.
//!
//! The kernel does the geometry (`solidcore_ops::offset::offset_faces`, the
//! faces named by record index through the lifted body's [`Order`]); this module
//! lifts the neutral body, resolves the caller's pick point to one **trimmed**
//! planar-or-curved face, drives the signed move and reads the body back.
//!
//! # Which face, and why a point
//!
//! The seam carries no kernel record ids, so [`SolidFaceSelector::NearestToPoint`]
//! resolves against the lifted body: every face is asked whether the point lies
//! on it *as trimmed* (`Body::point_on_face`), and the nearest such foot wins.
//! The pick tolerance is the chamfer selector's
//! `max(1e-9, 1e-3 · bounding-box diagonal)`; a point farther than that from every
//! face is `solid.face_not_found`, never an arbitrary face. Ties are broken by
//! record index, so the same body and point always pick the same face.
//!
//! # Sign
//!
//! `distance` is signed along the face's **outward** normal: positive adds the
//! swept volume (pull), negative removes it (push). Zero is refused by
//! `SolidRequest::validate` before this module runs.
//!
//! # Refusals
//!
//! A move the kernel will not build (no room, a collapsing offset, folds, no
//! effect) is `Unsupported` with [`OFFSET_FACE_FAILED`]; an integrality failure
//! stays `Failed`; cancellation is [`cad_domain::CadError::Cancelled`] with no
//! product. Every refusal leaves the input body untouched (the upstream edit
//! wrapper restores it), so a caller never sees half a move.

use cad_domain::{BrepData, CadError, CadResult, Point3};
use cad_kernel_adapter::{SolidFaceSelector, SolidOperationFamily, SolidOutcome};
use solidcore_ops::order::Order;
use solidcore_ops::{offset::offset_faces, Body};

use super::{bbox_diagonal, body_outcome, lift, unlift, watched, PICK_FLOOR, PICK_RELATIVE};
use crate::error::{self, LiftRefusal};
use crate::record::{length, point, sub};
use crate::{EMPTY_RESULT, FACE_NOT_FOUND, LIFT_UNSUPPORTED};

const FAMILY: SolidOperationFamily = SolidOperationFamily::OffsetFace;

/// Move the selected face of `brep` along its outward normal by `distance`.
pub(crate) fn offset_face(
    brep: &BrepData,
    selector: SolidFaceSelector,
    distance: f64,
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
    let tolerance = (PICK_RELATIVE * bbox_diagonal(&record)).max(PICK_FLOOR);
    let Some((face, found)) = nearest_face(&body, &order, at, tolerance) else {
        return Ok(not_found(tolerance, at, None));
    };
    if found > tolerance {
        return Ok(not_found(tolerance, at, Some(found)));
    }

    // Last chance to drop the work before the kernel starts.
    if cancelled() {
        return Err(CadError::Cancelled);
    }
    let result = watched(cancelled, || offset_faces(&body, &order, &[face], distance));

    // A result that arrived after cancellation is dropped: it is not a product.
    if cancelled() {
        return Err(CadError::Cancelled);
    }
    match result {
        Ok(()) => finish(&body),
        Err(error) => error::from_offset_face(error),
    }
}

/// The nearest trimmed face and the distance from `at` to its closest point.
///
/// `None` only when the body has no face the kernel can project onto; a face
/// the kernel answers with a status for is skipped rather than guessed.
fn nearest_face(body: &Body, order: &Order, at: Point3, tolerance: f64) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for face in 0..order.faces.len() {
        let foot = match body.point_on_face(order, face, [at.x, at.y, at.z], tolerance) {
            Ok(foot) => foot,
            // The kernel could not answer this face's projection; another face
            // may still hold the pick, so this one is skipped, not assumed.
            Err(_) => continue,
        };
        let Some(foot) = foot else {
            continue;
        };
        let distance = length(sub(at, point(foot)));
        if !distance.is_finite() {
            continue;
        }
        if best.is_none_or(|(_, nearest)| distance < nearest) {
            best = Some((face, distance));
        }
    }
    best
}

/// Read the moved body back into the neutral model, refusing anything a later
/// lift could not consume.
fn finish(body: &Body) -> CadResult<SolidOutcome> {
    let record = match body.to_record() {
        Ok((record, _order)) => record,
        Err(error) => return error::from_ops(error, FAMILY),
    };
    let brep = match unlift(&record) {
        Ok(brep) => brep,
        Err(refusal) => return Ok(error::unsupported(refusal, FAMILY)),
    };
    if brep.face_count() == 0 {
        return Ok(error::failed(
            EMPTY_RESULT,
            "the face offset produced no face",
        ));
    }
    if brep.shells.len() != 1 {
        return Ok(error::unsupported(
            LiftRefusal::new(
                LIFT_UNSUPPORTED,
                format!(
                    "the face offset produced {} shells; this seam hands out exactly one closed \
                     shell, and a second one would only be refused by its next lift",
                    brep.shells.len()
                ),
            ),
            FAMILY,
        ));
    }
    Ok(body_outcome(brep))
}

/// An `Unsupported` outcome with the stable "no face picked" code.
fn not_found(tolerance: f64, at: Point3, found: Option<f64>) -> SolidOutcome {
    let detail = match found {
        Some(found) => format!(
            "no trimmed face lies within {tolerance} of the selector point ({}, {}, {}); the \
             nearest is {found} away",
            at.x, at.y, at.z
        ),
        None => format!(
            "the lifted body has no trimmed face the kernel can project onto near ({}, {}, {})",
            at.x, at.y, at.z
        ),
    };
    error::unsupported(LiftRefusal::new(FACE_NOT_FOUND, detail), FAMILY)
}
