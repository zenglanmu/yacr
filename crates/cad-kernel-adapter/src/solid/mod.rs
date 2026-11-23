//! Kernel-independent solid modelling seam (ADR-0036 D1/D7, phase B1).
//!
//! [`SolidModeler`] is the sibling of [`crate::SolidTessellator`]. The
//! tessellator turns an imported payload into facets; the modeler creates and
//! combines **exact** bodies (primitives, booleans, a straight chamfer on one
//! edge, similarity transforms and mass properties) and returns a neutral
//! [`BrepData`](cad_domain::BrepData) that callers commit through `cad-db`. No
//! kernel type crosses this seam: the only implementation allowed to name
//! `solidcore` is `cad-kernel-solidcore` (phase B2), exactly as
//! `cad-connector-acadrust` is the only `acadrust` consumer.
//!
//! # Honest status
//!
//! Phase B1 defines the contract only. This crate links no modelling kernel, so
//! there is no default [`SolidModeler`] and no fabricated body: a host without a
//! modeler passes `None` and reports `unsupported-runtime: solid-modeler`
//! instead of silently succeeding.
//!
//! # Error mapping
//!
//! Implementations must follow this split, which the tessellation seam already
//! uses:
//!
//! | situation | result |
//! |---|---|
//! | capability the modeler does not declare | `Ok(SolidOutcome::Unsupported { reason })` |
//! | kernel failure / empty boolean result | `Ok(SolidOutcome::Failed { diagnostics })` |
//! | bad input (see [`SolidRequest::validate`]) | `Err(CadError::InvalidInput)` |
//! | cancelled before or after the kernel call | `Err(CadError::Cancelled)` |

mod capabilities;
mod outcome;
mod request;

pub use capabilities::*;
pub use outcome::*;
pub use request::*;

#[cfg(test)]
mod tests;

use cad_domain::{CadResult, Registration};

/// The kernel-facing solid modelling seam.
///
/// Implementations are stateless services: content in, content out. They never
/// hold a database body handle, a document revision or mutable per-document
/// state (ADR-0036 D1); [`crate::SolidTessellator`] stays unchanged and remains
/// the import-time facet path.
///
/// `Send + Sync` is a contract, not a convenience: hosts share one modeler
/// through `Arc<dyn SolidModeler>` and call it from a worker thread. A modeler
/// that serialises access to a process-global kernel must do so internally
/// (mutex / single job slot), never by requiring `&mut self`.
pub trait SolidModeler: Send + Sync {
    /// The kernel registration. The solidcore implementation reports
    /// `type_key == "yacr.kernel.solidcore"` and the version of the pinned
    /// revision it builds against (ADR-0036 D1).
    fn registration(&self) -> Registration;

    /// What this modeler claims it can do. [`ModelerCapabilities::supports`] is
    /// the cheap probe callers should use before submitting an operation.
    fn capabilities(&self) -> ModelerCapabilities;

    /// Execute one solid operation.
    ///
    /// The seam is synchronous and may be slow: callers must run it on a worker
    /// thread and must never block the UI (ADR-0036 D1).
    ///
    /// # Cancellation
    ///
    /// `cancelled` must be checked **before** the kernel call and **again after**
    /// it returns (and, where the kernel exposes a hook, while it runs). A
    /// request observed as cancelled returns
    /// [`cad_domain::CadError::Cancelled`] and the produced result is dropped:
    /// it is never committed and never returned as
    /// an `Unsupported` or `Failed` outcome. There is deliberately no `aborted`
    /// flag on [`SolidOutcome`]; a post-completion cancellation is an error, not
    /// a product. Implementations may implement only the pre/post checks (v0);
    /// that is *not* mid-kernel cancellation and must not be described as such.
    ///
    /// # Error mapping
    ///
    /// - a capability the modeler does not declare is
    ///   `Ok(SolidOutcome::Unsupported { reason })` (see [`crate::codes`]);
    /// - a kernel failure, or an operation that produces no usable body, is
    ///   `Ok(SolidOutcome::Failed { diagnostics })` — never an empty success;
    /// - bad input is `Err(CadError::InvalidInput)`; implementations call
    ///   [`SolidRequest::validate`] before any kernel work;
    /// - cancellation is `Err(CadError::Cancelled)`.
    fn execute(
        &self,
        request: &SolidRequest<'_>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CadResult<SolidResult>;
}

/// Entry point mirroring [`crate::tessellate_solid`]: route solid work through
/// this call so the seam is not bypassed.
///
/// It forwards verbatim; the implementation owns validation and the
/// pre/post-cancellation checks described on [`SolidModeler::execute`].
pub fn execute_solid(
    modeler: &dyn SolidModeler,
    request: &SolidRequest<'_>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> CadResult<SolidResult> {
    modeler.execute(request, cancelled)
}
