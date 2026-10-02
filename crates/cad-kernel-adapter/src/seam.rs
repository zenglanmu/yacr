//! The kernel-facing tessellation seam: the [`SolidTessellator`] trait and the
//! single entry point the rest of the system should route through.

use super::*;

/// The kernel-facing tessellation seam.
pub trait SolidTessellator {
    fn registration(&self) -> Registration;

    /// Tessellate a single solid/surface request.
    ///
    /// Implementations must:
    /// - return [`CadError::Cancelled`] when `cancelled` reports true;
    /// - reject a bad tolerance/budget with [`CadError::InvalidInput`];
    /// - return [`TessellationOutcome::Unsupported`] (never a fabricated mesh)
    ///   when they cannot evaluate the payload.
    fn tessellate(
        &self,
        request: &TessellationRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<TessellationResult>;
}

/// Entry point: tessellate `request` with `tessellator`, tagging the result with
/// the request's stamp. This is the single call site the rest of the system
/// should use so the seam is not bypassed.
pub fn tessellate_solid(
    tessellator: &dyn SolidTessellator,
    request: &TessellationRequest,
    cancelled: &dyn Fn() -> bool,
) -> CadResult<TessellationResult> {
    tessellator.tessellate(request, cancelled)
}
