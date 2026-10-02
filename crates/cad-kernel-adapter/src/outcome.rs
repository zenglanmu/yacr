//! Result-side types: the tessellated mesh, its structured degradation and the
//! outcome enum.

use super::*;

/// Why a returned facet set is not a complete, closed solid.
///
/// Populated only by a [`TessellationOutcome::Partial`] result. Each entry keeps
/// a stable reason so the UI can aggregate missing faces / open edges / dropped
/// shells instead of guessing from geometry (audit F15).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TessellationDegradation {
    pub missing_faces: Vec<FaceRef>,
    pub open_edges: Vec<EdgeRef>,
    pub dropped_shells: Vec<ShellRef>,
}

impl TessellationDegradation {
    /// Whether nothing was lost and the result is actually complete.
    pub fn is_empty(&self) -> bool {
        self.missing_faces.is_empty()
            && self.open_edges.is_empty()
            && self.dropped_shells.is_empty()
    }

    /// Structured diagnostics for every reported degradation.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut out = Vec::with_capacity(
            self.missing_faces.len() + self.open_edges.len() + self.dropped_shells.len(),
        );
        for face in &self.missing_faces {
            out.push(Diagnostic {
                code: codes::KERNEL_MISSING_FACE.to_string(),
                object: None,
                message: format!("face {} not tessellated: {}", face.id, face.reason),
            });
        }
        for edge in &self.open_edges {
            out.push(Diagnostic {
                code: codes::KERNEL_OPEN_EDGE.to_string(),
                object: None,
                message: format!("edge {} left open: {}", edge.id, edge.reason),
            });
        }
        for shell in &self.dropped_shells {
            out.push(Diagnostic {
                code: codes::KERNEL_DROPPED_SHELL.to_string(),
                object: None,
                message: format!("shell {} dropped: {}", shell.id, shell.reason),
            });
        }
        out
    }
}

/// A source face that failed to tessellate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceRef {
    pub id: u32,
    pub reason: String,
}

/// A boundary edge that was not closed by the produced facets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeRef {
    pub id: u32,
    pub reason: String,
}

/// A shell/body that was dropped because part of it could not be tessellated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellRef {
    pub id: u32,
    pub reason: String,
}

/// A tessellated solid: the facet mesh, its boundary edges and the precision
/// achieved.
#[derive(Debug, Clone, PartialEq)]
pub struct TessellationMesh {
    pub mesh: Mesh,
    /// Boundary/feature polylines in world space.
    pub edges: Vec<Vec<Point3>>,
    /// How faithful this mesh is to the exact surface.
    pub precision: Precision,
}

impl TessellationMesh {
    pub fn triangle_count(&self) -> usize {
        self.mesh.triangles.len()
    }

    /// A mesh with no faces is not usable output.
    pub fn is_empty(&self) -> bool {
        self.mesh.triangles.is_empty()
    }
}

/// Why the seam could not produce facets, kept structured so callers can act
/// on the category rather than parse prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedReason {
    /// Stable [`codes`] value.
    pub code: String,
    /// Which payload kind was refused.
    pub exchange: ExchangeKind,
    /// Human-facing detail (localised by the UI, never parsed).
    pub detail: String,
}

/// The outcome of a tessellation attempt.
///
/// There is deliberately no "success with no mesh" variant: an attempt that
/// produces nothing usable is [`TessellationOutcome::Unsupported`] or
/// [`TessellationOutcome::Failed`].
#[derive(Debug, Clone, PartialEq)]
pub enum TessellationOutcome {
    /// A complete facet set.
    Success {
        geometry: TessellationMesh,
        diagnostics: Vec<Diagnostic>,
    },
    /// Usable facets, but with explicit, reported degradation.
    Partial {
        geometry: TessellationMesh,
        degradation: TessellationDegradation,
        diagnostics: Vec<Diagnostic>,
    },
    /// The seam cannot evaluate this payload in this build. Never a mesh.
    Unsupported { reason: UnsupportedReason },
    /// The payload was evaluable in principle but produced no usable result.
    Failed { diagnostics: Vec<Diagnostic> },
}

impl TessellationOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, TessellationOutcome::Success { .. })
    }

    /// The produced facets, if any. `None` for unsupported/failed outcomes, so
    /// callers cannot mistake "no kernel" for "empty solid".
    pub fn mesh(&self) -> Option<&TessellationMesh> {
        match self {
            TessellationOutcome::Success { geometry, .. }
            | TessellationOutcome::Partial { geometry, .. } => Some(geometry),
            TessellationOutcome::Unsupported { .. } | TessellationOutcome::Failed { .. } => None,
        }
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            TessellationOutcome::Success { diagnostics, .. }
            | TessellationOutcome::Partial { diagnostics, .. }
            | TessellationOutcome::Failed { diagnostics } => diagnostics,
            TessellationOutcome::Unsupported { .. } => &[],
        }
    }

    pub fn degradation(&self) -> Option<&TessellationDegradation> {
        match self {
            TessellationOutcome::Partial { degradation, .. } => Some(degradation),
            _ => None,
        }
    }
}

/// Result of a [`SolidTessellator::tessellate`] call.
#[derive(Debug, Clone, PartialEq)]
pub struct TessellationResult {
    pub stamp: TaskStamp,
    pub outcome: TessellationOutcome,
}

impl TessellationResult {
    /// Whether the result carries usable (possibly degraded) facets.
    pub fn is_usable(&self) -> bool {
        self.outcome.mesh().map(|m| !m.is_empty()).unwrap_or(false)
    }
}
