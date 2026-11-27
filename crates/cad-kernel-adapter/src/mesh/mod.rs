//! Kernel-independent **mesh** modelling seam (ADR-0061, P-MESH-0).
//!
//! [`MeshModeler`] is the sibling of [`crate::SolidModeler`]. The solid seam
//! creates and combines exact B-rep bodies; this seam performs stateless
//! triangle-mesh operations: boolean union/subtract/intersect and repair of a
//! closed triangle soup. No kernel type crosses the boundary — the only
//! implementation allowed to name `manifold_rust` is `cad-kernel-manifold`,
//! exactly as `cad-kernel-solidcore` is the only `solidcore` consumer.
//!
//! # Honest status
//!
//! P-MESH-0 defines the contract and ships one implementation. This crate
//! itself links no mesh kernel, so there is no default [`MeshModeler`]; a host
//! without one reports `unsupported-runtime: mesh-modeler` instead of silently
//! approximating or fabricating geometry.
//!
//! A mesh result is **never** exact B-rep geometry. It must be stored as
//! [`cad_domain::Mesh`] (or a `Body3d` whose `exact` is `Unavailable`), never as
//! `Body3d { kind: Solid, exact: Exact }`, and it must not be presented as an
//! analytic/engineering-exact measurement.
//!
//! # Error mapping
//!
//! | situation | result |
//! |---|---|
//! | capability the modeler does not declare | `Ok(MeshOutcome::Unsupported { reason })` |
//! | kernel refusal of a known input class (open/non-manifold shell) | `Ok(MeshOutcome::Unsupported { reason })` |
//! | kernel failure or an empty result | `Ok(MeshOutcome::Failed { diagnostics })` |
//! | bad input (see [`MeshRequest::validate`]) | `Err(CadError::InvalidInput)` |
//! | cancelled before or after the kernel call | `Err(CadError::Cancelled)` |

mod capabilities;
mod classifier;
mod outcome;
mod request;

pub use capabilities::*;
pub use classifier::*;
pub use outcome::*;
pub use request::*;

#[cfg(test)]
mod tests;

use cad_domain::{CadResult, Registration};

/// The kernel-facing triangle-mesh modelling seam.
///
/// Implementations are stateless services: content in, content out. They never
/// hold a database mesh handle, a document revision or mutable per-document
/// state. `Send + Sync` is a contract, not a convenience: hosts share one
/// modeler through `Arc<dyn MeshModeler>` and call it from a worker thread.
pub trait MeshModeler: Send + Sync {
    /// The kernel registration, for diagnostics and capability reporting.
    fn registration(&self) -> Registration;

    /// What this modeler claims it can do.
    fn capabilities(&self) -> MeshModelerCapabilities;

    /// Execute one mesh operation.
    ///
    /// The seam is synchronous and may be slow. Native hosts run it on a worker
    /// thread and must never block the UI; the `dom-canvas` wasm carrier has no
    /// thread and runs it inline on the session thread under an explicit
    /// `mesh.sync_budget` cap (ADR-0061), which is a deliberate, bounded
    /// exception rather than a silent main-thread fallback.
    ///
    /// # Cancellation
    ///
    /// `cancelled` must be checked **before** the kernel call and **again after**
    /// it returns. An implementation with a kernel cancellation hook may also
    /// check it while running, but must say so; pre/post checks alone must not
    /// be described as mid-kernel cancellation. A request observed as cancelled
    /// returns [`cad_domain::CadError::Cancelled`] and no product.
    fn execute(
        &self,
        request: &MeshRequest<'_>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CadResult<MeshResult>;
}

/// Route mesh work through this call so the seam is not bypassed.
pub fn execute_mesh(
    modeler: &dyn MeshModeler,
    request: &MeshRequest<'_>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> CadResult<MeshResult> {
    modeler.execute(request, cancelled)
}
