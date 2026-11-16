//! Similarity transforms, baked into the neutral B-rep.
//!
//! A similarity (uniform scale + rotation + translation) is exact arithmetic on
//! the neutral model: points move, directions rotate, radii scale, and a cone's
//! half angle is unchanged. Nothing re-enters the kernel, so unlike booleans and
//! mass properties a **curved** body can be transformed losslessly here.
//!
//! The composed placement follows
//! [`cad_domain::BrepPlacement::composed_with`]: the body's own placement first,
//! then the request's `transform`. The result carries no placement (the geometry
//! itself is moved), so every consumer — display, export, a later lift — sees the
//! same coordinates.
//!
//! Refused, never approximated (ADR-0036 D4):
//!
//! - a non-similarity (non-uniform scale, shear): [`crate::NONUNIFORM_TRANSFORM`];
//! - a mirroring similarity (negative determinant) and a placement that cannot be
//!   baked (non-positive scale): [`crate::PLACEMENT_NOT_SUPPORTED`]. Reflecting a
//!   solid flips its outward normals, which the record's shell convention cannot
//!   express by mapping directions alone, so it is an explicit refusal.

use cad_domain::{BrepData, BrepPlacement, CadResult, Transform3};
use cad_kernel_adapter::{SolidOperationFamily, SolidOutcome};

use super::body_outcome;
use crate::error;
use crate::record;

const FAMILY: SolidOperationFamily = SolidOperationFamily::Transform;

/// The relative tolerance used to judge "uniform" (see
/// [`Transform3::is_uniform_scale`]).
const SIMILARITY_TOLERANCE: f64 = 1e-9;

/// Map `brep` through `transform`.
pub(crate) fn transform(brep: &BrepData, transform: &Transform3) -> CadResult<SolidOutcome> {
    if !transform.is_uniform_scale(SIMILARITY_TOLERANCE) {
        return Ok(error::unsupported(
            error::LiftRefusal::new(
                error::NONUNIFORM_TRANSFORM,
                "only a similarity (uniform scale, rotation, translation) is baked; \
                 a non-uniform scale or shear would change the body's shape",
            ),
            FAMILY,
        ));
    }
    if transform.determinant() < 0.0 {
        return Ok(error::unsupported(
            error::LiftRefusal::new(
                error::PLACEMENT_NOT_SUPPORTED,
                "a mirroring similarity is not baked in P1",
            ),
            FAMILY,
        ));
    }
    let base = brep.placement.unwrap_or_else(BrepPlacement::identity);
    let composed = base.composed_with(transform);
    match record::bake(brep, composed) {
        Ok(brep) => Ok(body_outcome(brep)),
        Err(refusal) => Ok(error::unsupported(refusal, FAMILY)),
    }
}
