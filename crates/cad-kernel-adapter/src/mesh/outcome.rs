//! Products and outcomes of a mesh operation.
//!
//! There is deliberately no "success with no mesh" variant and no `aborted`
//! flag: an operation that produces nothing usable is [`MeshOutcome::Failed`],
//! a known input class the kernel cannot evaluate is
//! [`MeshOutcome::Unsupported`], and cancellation is
//! [`cad_domain::CadError::Cancelled`] rather than an outcome (ADR-0061).

use cad_domain::{Diagnostic, TaskStamp};

use super::{
    MeshData, MeshOperationFamily, MeshRepairMode, MeshTopology, MeshTopologyReport,
    MeshWindingRule,
};

/// Why a modeler refused an operation, kept structured so callers act on the
/// category instead of parsing prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshUnsupportedReason {
    /// Stable [`crate::codes`] value (for example `mesh.not_closed`).
    pub code: &'static str,
    /// Which operation family was refused.
    pub operation: MeshOperationFamily,
    /// Human-facing detail (localised by the UI, never parsed).
    pub detail: String,
}

/// What a repair actually did, and what it read before and after.
///
/// `mode` names the requested repair, not a claim that geometry changed. A
/// rebuild may retriangulate; an orientation repair preserves triangle count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshRepair {
    /// The requested repair.
    pub mode: MeshRepairMode,
    /// The winding rule used by a rebuild.
    pub rule: MeshWindingRule,
    /// Topology before the repair (measured by the seam, not the kernel).
    pub input: MeshTopologyReport,
    /// Topology after the repair.
    pub output: MeshTopologyReport,
    /// Coincident vertices welded by the seam before the repair; `0` when the
    /// implementation performs no weld.
    pub welded_vertices: usize,
}

impl MeshRepair {
    /// Whether the output is a closed 2-manifold.
    pub fn is_closed(&self) -> bool {
        self.output.topology.is_closed()
    }
}

/// The outcome of a mesh operation.
///
/// ```text
/// Ok(Success)        a boolean produced a usable mesh
/// Ok(Partial)        a repair produced a mesh; the report says what was done
/// Ok(Unsupported)    declared capability or input class is missing
/// Ok(Failed)         the kernel ran but produced no usable product
/// Err(InvalidInput)  request did not validate
/// Err(Cancelled)     cancelled before or after the kernel call
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum MeshOutcome {
    /// A boolean/repair product. The mesh is never exact B-rep geometry.
    Success {
        mesh: MeshData,
        /// Topology of the produced mesh.
        topology: MeshTopologyReport,
        diagnostics: Vec<Diagnostic>,
    },
    /// A repair product with an explicit report of what was requested/done.
    Partial {
        mesh: MeshData,
        repair: MeshRepair,
        diagnostics: Vec<Diagnostic>,
    },
    /// The seam cannot evaluate this operation in this build. Never a product.
    Unsupported { reason: MeshUnsupportedReason },
    /// The operation was evaluable in principle but produced no usable mesh
    /// (for example an empty boolean result). Never an empty success.
    Failed { diagnostics: Vec<Diagnostic> },
}

impl MeshOutcome {
    /// Whether this outcome carries a mesh.
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            MeshOutcome::Success { .. } | MeshOutcome::Partial { .. }
        )
    }

    /// The produced mesh, if any. `None` for unsupported/failed outcomes, so a
    /// caller cannot mistake "no kernel" for "empty mesh".
    pub fn mesh(&self) -> Option<&MeshData> {
        match self {
            MeshOutcome::Success { mesh, .. } | MeshOutcome::Partial { mesh, .. } => Some(mesh),
            MeshOutcome::Unsupported { .. } | MeshOutcome::Failed { .. } => None,
        }
    }

    /// The produced topology, if any.
    pub fn topology(&self) -> Option<MeshTopology> {
        match self {
            MeshOutcome::Success { topology, .. } => Some(topology.topology),
            MeshOutcome::Partial { repair, .. } => Some(repair.output.topology),
            MeshOutcome::Unsupported { .. } | MeshOutcome::Failed { .. } => None,
        }
    }

    /// The structured refusal, if the outcome is `Unsupported`.
    pub fn unsupported(&self) -> Option<&MeshUnsupportedReason> {
        match self {
            MeshOutcome::Unsupported { reason } => Some(reason),
            _ => None,
        }
    }

    /// Every diagnostic carried by this outcome (the refusal reason is
    /// structured instead of a free-text diagnostic).
    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            MeshOutcome::Success { diagnostics, .. }
            | MeshOutcome::Partial { diagnostics, .. }
            | MeshOutcome::Failed { diagnostics } => diagnostics,
            MeshOutcome::Unsupported { .. } => &[],
        }
    }
}

/// Result of a [`MeshModeler::execute`](super::MeshModeler::execute) call.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshResult {
    pub stamp: TaskStamp,
    pub outcome: MeshOutcome,
}
