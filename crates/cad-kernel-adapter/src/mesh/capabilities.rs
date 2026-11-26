//! What a [`MeshModeler`](super::MeshModeler) declares it can do.
//!
//! A `false` flag means the caller must get
//! [`MeshOutcome::Unsupported`](super::MeshOutcome) for that family, never a
//! degraded approximation or an empty success.

use super::{MeshOperation, MeshOperationFamily};

/// Capability declaration of a mesh modeler.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshModelerCapabilities {
    /// Boolean union/subtract/intersect on closed manifolds.
    pub boolean: bool,
    /// Boolean on closed **non-manifold** triangle soup (robust engine).
    pub robust_boolean: bool,
    /// Flip inside-out shells without changing the triangulation.
    pub repair_orientation: bool,
    /// Rebuild a closed solid from the winding numbers.
    pub rebuild_solid: bool,
}

impl MeshModelerCapabilities {
    /// Whether this modeler claims the operation's family.
    pub fn supports(&self, operation: &MeshOperation<'_>) -> bool {
        match operation.family() {
            MeshOperationFamily::Boolean => self.boolean || self.robust_boolean,
            MeshOperationFamily::Repair => self.repair_orientation || self.rebuild_solid,
        }
    }

    /// Every mesh capability.
    pub fn all() -> Self {
        MeshModelerCapabilities {
            boolean: true,
            robust_boolean: true,
            repair_orientation: true,
            rebuild_solid: true,
        }
    }
}
