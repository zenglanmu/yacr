//! Primitive creation: `PrimitiveSpec` → exact body.
//!
//! Two paths, chosen by the family:
//!
//! - `BOX`, `WEDGE`, `PYRAMID` are authored locally as exact planar B-rep data
//!   ([`crate::polyhedra`]) and lifted once so the kernel proves it accepts the
//!   body before it is handed out. The product is the **authored** body (its
//!   numbers are exact), not the kernel's round trip;
//! - `CYLINDER`, `SPHERE`, `CONE`, `TORUS` are built by the upstream
//!   constructors and read back through `Body::to_record`, then mapped to
//!   analytic neutral surfaces. Those bodies display and export exactly, and they
//!   **re-lift** (P2.1b part 2): the writer carries each face's exact NURBS form,
//!   its `(u, v)` box and exact pcurves back into the kernel, so a boolean, mass
//!   properties or a chamfer over one is answered. A boundary family this build
//!   cannot write is still refused ([`crate::LIFT_REQUIRES_PLANAR`]), and a
//!   trimmed spherical/toroidal face is refused by the reader.
//!
//! Input validation happens before this module runs
//! ([`cad_kernel_adapter::SolidRequest::validate`]); a dimension the kernel
//! additionally refuses (a torus whose tube reaches its axis, a cone with equal
//! radii) comes back as `Failed` with [`crate::KERNEL_FAILED`], never as an
//! invented body.

use cad_domain::{CadResult, Point3};
use cad_kernel_adapter::{PrimitiveSpec, SolidOperationFamily, SolidOutcome};
use solidcore_ops::Body;

use super::{body_outcome, lift, unlift};
use crate::error;
use crate::polyhedra;

const FAMILY: SolidOperationFamily = SolidOperationFamily::Primitive;

/// Create one primitive body.
pub(crate) fn create(spec: &PrimitiveSpec) -> CadResult<SolidOutcome> {
    match *spec {
        PrimitiveSpec::Box {
            origin,
            length,
            width,
            height,
        } => authored(polyhedra::box_brep(origin, length, width, height)),
        PrimitiveSpec::Wedge {
            origin,
            length,
            width,
            height,
        } => authored(polyhedra::wedge_brep(origin, length, width, height)),
        PrimitiveSpec::Pyramid {
            base_center,
            length,
            height,
        } => authored(polyhedra::pyramid_brep(base_center, length, height)),
        PrimitiveSpec::PyramidNgon {
            base_center,
            radius,
            sides,
            rotation,
            height,
        } => authored(polyhedra::pyramid_ngon_brep(
            base_center,
            radius,
            sides,
            rotation,
            height,
        )),
        PrimitiveSpec::Cylinder {
            origin,
            radius,
            height,
        } => curved(solidcore_ops::make_cylinder(triple(origin), radius, height)),
        PrimitiveSpec::Sphere { origin, radius } => {
            curved(solidcore_ops::make_sphere(triple(origin), radius))
        }
        PrimitiveSpec::Cone {
            origin,
            radius_bottom,
            radius_top,
            height,
        } => curved(solidcore_ops::make_cone(
            triple(origin),
            radius_bottom,
            radius_top,
            height,
        )),
        PrimitiveSpec::Torus {
            origin,
            major_radius,
            minor_radius,
        } => curved(solidcore_ops::make_torus(
            triple(origin),
            major_radius,
            minor_radius,
        )),
    }
}

/// A locally authored planar body, proved acceptable to the kernel.
fn authored(brep: cad_domain::BrepData) -> CadResult<SolidOutcome> {
    let record = match lift(&brep) {
        Ok(record) => record,
        Err(refusal) => return Ok(error::unsupported(refusal, FAMILY)),
    };
    match Body::from_record(&record, false) {
        Ok(_) => Ok(body_outcome(brep)),
        Err(error) => error::from_ops(error, FAMILY),
    }
}

/// A body built by an upstream constructor, read back as neutral analytic data.
fn curved(body: solidcore_ops::Result<Body>) -> CadResult<SolidOutcome> {
    let body = match body {
        Ok(body) => body,
        Err(error) => return error::from_ops(error, FAMILY),
    };
    let record = match body.to_record() {
        Ok((record, _order)) => record,
        Err(error) => return error::from_ops(error, FAMILY),
    };
    match unlift(&record) {
        Ok(brep) => Ok(body_outcome(brep)),
        Err(refusal) => Ok(error::unsupported(refusal, FAMILY)),
    }
}

fn triple(point: Point3) -> [f64; 3] {
    [point.x, point.y, point.z]
}
