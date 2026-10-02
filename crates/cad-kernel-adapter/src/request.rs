//! Request-side types: the geometry handle, the tolerance/budget policy and the
//! validated [`TessellationRequest`].

use super::*;

/// A geometry handle the request points at. It is either resolved to a database
/// object or explicitly missing; the seam never silently drops an unresolved
/// handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeometryHandle {
    Resolved(ObjectId),
    Missing { key: String },
}

/// Tolerance / deflection policy for a single tessellation request.
///
/// Both values are world-space quantities. The linear deflection is the maximum
/// chordal deviation between the true surface and the facets; the angular
/// deflection is the maximum angle between adjacent facet normals. They must be
/// finite and strictly positive (see [`TessellationTolerance::validate`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TessellationTolerance {
    pub linear_deflection: f64,
    pub angular_deflection: f64,
}

impl Default for TessellationTolerance {
    fn default() -> Self {
        TessellationTolerance {
            linear_deflection: 0.01,
            angular_deflection: 0.35,
        }
    }
}

impl TessellationTolerance {
    /// Reject non-finite, non-positive or inverted deflection policies.
    pub fn validate(&self) -> Result<(), TessellationInputError> {
        if !self.linear_deflection.is_finite() || self.linear_deflection <= 0.0 {
            return Err(TessellationInputError {
                code: codes::KERNEL_INVALID_TOLERANCE,
                message: format!(
                    "linear deflection must be finite and positive, got {}",
                    self.linear_deflection
                ),
            });
        }
        if !self.angular_deflection.is_finite() || self.angular_deflection <= 0.0 {
            return Err(TessellationInputError {
                code: codes::KERNEL_INVALID_TOLERANCE,
                message: format!(
                    "angular deflection must be finite and positive, got {}",
                    self.angular_deflection
                ),
            });
        }
        Ok(())
    }

    /// Derive a request tolerance from the view policy and the current zoom.
    ///
    /// The linear deflection is the screen-space budget converted to world
    /// units. The angular deflection is not derivable from pixels, so the
    /// default is kept and documented rather than invented per call.
    pub fn from_policy(policy: &TolerancePolicy, world_per_px: f64) -> Self {
        let world_per_px = if world_per_px.is_finite() && world_per_px > 0.0 {
            world_per_px
        } else {
            1.0
        };
        TessellationTolerance {
            linear_deflection: (policy.display_pixels * world_per_px).max(1e-12),
            angular_deflection: TessellationTolerance::default().angular_deflection,
        }
    }
}

/// Per-entity budget enforced on a tessellation request.
///
/// A budget of zero is invalid (it can never be met); see
/// [`TessellationBudget::validate`]. A produced mesh that exceeds the budget is
/// reported with `kernel.budget_exceeded`, never silently truncated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TessellationBudget {
    pub max_faces: u32,
    pub max_vertices: u32,
    pub max_edges: u32,
}

impl Default for TessellationBudget {
    fn default() -> Self {
        TessellationBudget {
            max_faces: 200_000,
            max_vertices: 600_000,
            max_edges: 200_000,
        }
    }
}

impl TessellationBudget {
    pub fn validate(&self) -> Result<(), TessellationInputError> {
        for (name, value) in [
            ("max_faces", self.max_faces),
            ("max_vertices", self.max_vertices),
            ("max_edges", self.max_edges),
        ] {
            if value == 0 {
                return Err(TessellationInputError {
                    code: codes::KERNEL_INVALID_BUDGET,
                    message: format!("tessellation budget {name} must be greater than zero"),
                });
            }
        }
        Ok(())
    }

    /// Check a completed mesh against this budget.
    ///
    /// Returns the structured over-budget diagnostic when the mesh cannot be
    /// accepted, so a caller reports `kernel.budget_exceeded` instead of
    /// silently degrading.
    pub fn check(&self, mesh: &TessellationMesh) -> Result<(), Diagnostic> {
        let faces = mesh.mesh.triangles.len() as u64;
        let vertices = mesh.mesh.vertices.len() as u64;
        let edges = mesh
            .edges
            .iter()
            .map(|e| e.len().saturating_sub(1) as u64)
            .sum();
        let over = |actual: u64, limit: u32, what: &str| {
            (actual > u64::from(limit)).then(|| Diagnostic {
                code: codes::KERNEL_BUDGET_EXCEEDED.to_string(),
                object: None,
                message: format!("tessellation exceeded {what} budget: {actual} > {limit}"),
            })
        };
        over(faces, self.max_faces, "face")
            .or_else(|| over(vertices, self.max_vertices, "vertex"))
            .or_else(|| over(edges, self.max_edges, "edge"))
            .map_or(Ok(()), Err)
    }
}

/// A single tessellation request.
///
/// The request is validated before any kernel work; a bad tolerance or budget
/// is rejected with [`TessellationInputError`], an unresolved handle or an
/// empty payload is reported through the result outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct TessellationRequest {
    /// Which database object this payload belongs to.
    pub geometry: GeometryHandle,
    /// The opaque source payload.
    pub exchange: SolidExchange,
    /// Deflection policy for the facets.
    pub tolerance: TessellationTolerance,
    /// Per-entity budget the result must respect.
    pub budget: TessellationBudget,
    /// Identity of the originating task, for staleness checks.
    pub stamp: TaskStamp,
}

/// A validated-input failure raised before any kernel work starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TessellationInputError {
    /// Stable [`codes`] value (`kernel.invalid_tolerance` / `kernel.invalid_budget`).
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for TessellationInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for TessellationInputError {}

impl TessellationRequest {
    /// Validate tolerance and budget. Handles and payload emptiness are reported
    /// through the outcome, not here, so they can be aggregated with geometry
    /// diagnostics.
    pub fn validate(&self) -> Result<(), TessellationInputError> {
        self.tolerance.validate()?;
        self.budget.validate()?;
        Ok(())
    }
}
