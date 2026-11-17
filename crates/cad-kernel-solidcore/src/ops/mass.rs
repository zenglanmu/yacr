//! Mass properties: volume, boundary area, centroid and the inertia tensor.
//!
//! The body is lifted exactly like a boolean operand (so the same boundary-family
//! refusals apply; a curved face is written with its exact form), rebuilt as a
//! kernel body and integrated by `Body::mass_properties(relative_accuracy)`.
//!
//! **The accuracy is requested, not proved.** The upstream integrator holds its
//! own error *estimate* under the requested relative accuracy; that is not a
//! bound, so the result is reported as
//! [`cad_kernel_adapter::MassAccuracy::Requested`] and must never be promoted to
//! `Precision::Analytic`. (Closed forms exist for a box or sphere, but they are a
//! test oracle, not a kernel guarantee.)
//!
//! # Density
//!
//! `Body::mass_properties` integrates the **unit-density** tensor (the geometric
//! `∫(y²+z²)dV`, … about the centroid). `volume`, `surface_area` and `centroid`
//! are therefore geometric and density-independent, while the seam's
//! [`cad_kernel_adapter::SolidMassProperties::inertia`] is a **mass** inertia:
//! this module multiplies every tensor entry by the requested `density`, which is
//! an exact scale factor for a homogeneous body. `density` is echoed on the
//! result, so `mass = volume · density` and the inertia is in mass · units².
//! `MASSPROP` requests density `1.0`, where the two readings coincide.
//!
//! Units pass straight through: volume in drawing-units³, area in
//! drawing-units², the centroid in model coordinates (see the crate docs).

use cad_domain::{CadResult, Point3};
use cad_kernel_adapter::{
    MassAccuracy, SolidMassProperties, SolidOperationFamily, SolidOutcome, SolidProduct,
};
use solidcore_ops::Body;

use super::lift;
use crate::error;

const FAMILY: SolidOperationFamily = SolidOperationFamily::MassProperties;

/// The relative accuracy every integration asks for (`1e-6` = 0.0001 %).
///
/// The seam's `MassProperties` operation carries no accuracy of its own, so this
/// module picks one and reports the kernel's echo of it
/// ([`MassPropertiesEstimate::requested_accuracy`](solidcore_ops::MassPropertiesEstimate)).
pub(crate) const RELATIVE_ACCURACY: f64 = 1e-6;

/// Integrate the mass properties of one body at the requested density.
pub(crate) fn mass(brep: &cad_domain::BrepData, density: f64) -> CadResult<SolidOutcome> {
    let record = match lift(brep) {
        Ok(record) => record,
        Err(refusal) => return Ok(error::unsupported(refusal, FAMILY)),
    };
    let (body, _order) = match Body::from_record(&record, false) {
        Ok(body) => body,
        Err(error) => return error::from_ops(error, FAMILY),
    };
    let estimate = match body.mass_properties(RELATIVE_ACCURACY) {
        Ok(estimate) => estimate,
        Err(error) => return error::from_ops(error, FAMILY),
    };
    if !estimate.volume.is_finite()
        || !estimate.area.is_finite()
        || !estimate.requested_accuracy.is_finite()
        || estimate.centroid.iter().any(|value| !value.is_finite())
        || estimate
            .tensor
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
    {
        return Ok(error::failed(
            error::KERNEL_FAILED,
            "the mass-properties estimate is not finite",
        ));
    }
    let mass = SolidMassProperties {
        volume: estimate.volume,
        surface_area: estimate.area,
        centroid: Point3 {
            x: estimate.centroid[0],
            y: estimate.centroid[1],
            z: estimate.centroid[2],
        },
        // The kernel's tensor is about the centroid with the negated products
        // off the diagonal — the standard inertia tensor — integrated at unit
        // density. The requested density is an exact scale factor for a
        // homogeneous body; volume/area/centroid stay geometric.
        inertia: Some(estimate.tensor.map(|row| row.map(|value| value * density))),
        density,
        accuracy: MassAccuracy::Requested {
            relative: estimate.requested_accuracy,
        },
    };
    Ok(SolidOutcome::Success(SolidProduct::Mass(mass)))
}
