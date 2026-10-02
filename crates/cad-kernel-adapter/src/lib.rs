//! Isolated seam between imported solid/surface payloads and tessellatable
//! geometry (spec §10.2, requirement F15).
//!
//! This crate owns every kernel-facing type. Nothing here may leak an ACIS
//! kernel handle, a SAT/SAB parser or a surface object into the rest of the
//! workspace; the only things that cross the boundary are the plain-data
//! [`TessellationRequest`] and the structured [`TessellationResult`].
//!
//! # Honest status
//!
//! There is **no** ACIS kernel, no SAT/SAB parser and no licensed fixture
//! sample in this repository yet. A default [`SolidTessellator`],
//! [`NoKernelTessellator`], therefore never fabricates a mesh: it reports the
//! request as [`TessellationOutcome::Unsupported`] with a stable diagnostic
//! code. See `docs/kernel-acis.md` for the integration contract and the
//! acceptance criteria that must be met before this can be considered done.

use cad_domain::*;

pub mod brep;
pub use brep::{BrepCurve, BrepData, BrepFace, BrepLoop, BrepPlacement, BrepShell, BrepSurface};

/// Stable, locale-independent diagnostic codes emitted by this crate.
///
/// These codes are part of the contract: the diagnostics layer aggregates them
/// and the UI localises the message, nothing parses the human text.
pub mod codes {
    /// No ACIS kernel is linked into this build, so a SAT/SAB payload cannot be
    /// evaluated. This is the honest default for requirement F15.
    pub const KERNEL_NO_ACIS_KERNEL: &str = "kernel.no_acis_kernel";
    /// The exchange payload declares a type this adapter cannot even classify.
    pub const KERNEL_UNSUPPORTED_EXCHANGE: &str = "kernel.unsupported_exchange";
    /// A neutral B-rep carried a face whose supporting surface (or boundary
    /// curve) this build cannot evaluate. It is never replaced by a fake mesh.
    pub const KERNEL_UNSUPPORTED_SURFACE: &str = "kernel.unsupported_surface";
    /// The exchange payload was present but carried no bytes.
    pub const KERNEL_EMPTY_GEOMETRY: &str = "kernel.empty_geometry";
    /// The request referenced a geometry handle that could not be resolved.
    pub const KERNEL_MISSING_HANDLE: &str = "kernel.missing_handle";
    /// The deflection policy was non-finite, non-positive or otherwise invalid.
    pub const KERNEL_INVALID_TOLERANCE: &str = "kernel.invalid_tolerance";
    /// The per-entity budget was invalid (for example a zero limit).
    pub const KERNEL_INVALID_BUDGET: &str = "kernel.invalid_budget";
    /// A produced mesh exceeds the request's per-entity budget.
    pub const KERNEL_BUDGET_EXCEEDED: &str = "kernel.budget_exceeded";
    /// A face of the source solid could not be tessellated.
    pub const KERNEL_MISSING_FACE: &str = "kernel.missing_face";
    /// A boundary edge was left open (adjacent facets did not share vertices).
    pub const KERNEL_OPEN_EDGE: &str = "kernel.open_edge";
    /// A shell/body was dropped because a face in it could not be tessellated.
    pub const KERNEL_DROPPED_SHELL: &str = "kernel.dropped_shell";
    /// The request was cancelled before a result could be produced.
    pub const KERNEL_CANCELLED: &str = "kernel.cancelled";
}

/// The raw solid/surface payload crossing the kernel seam.
///
/// The payload is kept opaque and byte-oriented on purpose: this adapter is the
/// only place that may ever interpret it, and until an ACIS kernel is linked it
/// stays an unresolved byte buffer.
#[derive(Debug, Clone, PartialEq)]
pub enum SolidExchange {
    /// ACIS SAT text.
    Sat(Vec<u8>),
    /// ACIS SAB binary.
    Sab(Vec<u8>),
    /// Neutral B-rep lifted from a SAT/SAB payload by the importer, the only
    /// acadrust consumer. This is what [`BrepTessellator`] evaluates; the raw
    /// bytes are never handed to a kernel through this variant.
    Brep(BrepData),
    /// Any other source-declared payload (for example an importer-internal
    /// opaque body). It is carried verbatim so a future decoder can be added
    /// without changing the request shape.
    Unsupported { type_key: String, data: Vec<u8> },
}

/// Classification of a [`SolidExchange`], independent of its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExchangeKind {
    Sat,
    Sab,
    Brep,
    Unsupported,
}

impl ExchangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ExchangeKind::Sat => "sat",
            ExchangeKind::Sab => "sab",
            ExchangeKind::Brep => "brep",
            ExchangeKind::Unsupported => "unsupported",
        }
    }
}

impl SolidExchange {
    pub fn kind(&self) -> ExchangeKind {
        match self {
            SolidExchange::Sat(_) => ExchangeKind::Sat,
            SolidExchange::Sab(_) => ExchangeKind::Sab,
            SolidExchange::Brep(_) => ExchangeKind::Brep,
            SolidExchange::Unsupported { .. } => ExchangeKind::Unsupported,
        }
    }

    /// Whether the payload carries any bytes at all.
    ///
    /// An empty payload is *not* a valid solid; callers must report it rather
    /// than treat the absence of geometry as success (`kernel.empty_geometry`).
    pub fn is_empty(&self) -> bool {
        match self {
            SolidExchange::Sat(bytes) | SolidExchange::Sab(bytes) => bytes.is_empty(),
            SolidExchange::Brep(brep) => brep.is_empty(),
            SolidExchange::Unsupported { data, .. } => data.is_empty(),
        }
    }

    /// Byte length of the payload, for budget and diagnostic reporting.
    ///
    /// A neutral B-rep has no byte payload of its own; its size is its face
    /// count, so the byte-oriented callers still get a meaningful number.
    pub fn len(&self) -> usize {
        match self {
            SolidExchange::Sat(bytes) | SolidExchange::Sab(bytes) => bytes.len(),
            SolidExchange::Brep(brep) => brep.face_count(),
            SolidExchange::Unsupported { data, .. } => data.len(),
        }
    }

    /// A sanitised logical key for diagnostics; never a raw payload.
    pub fn type_key(&self) -> &str {
        match self {
            SolidExchange::Sat(_) => "acis.sat",
            SolidExchange::Sab(_) => "acis.sab",
            SolidExchange::Brep(_) => "acis.brep",
            SolidExchange::Unsupported { type_key, .. } => type_key,
        }
    }
}

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

/// The honest default: no ACIS kernel is linked, so every non-empty payload is
/// reported as unsupported with a stable code instead of a fake mesh.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoKernelTessellator;

/// Backwards-compatible name for the default (previously `PendingTessellator`).
pub type PendingTessellator = NoKernelTessellator;

impl NoKernelTessellator {
    fn failed(stamp: &TaskStamp, code: &str, message: String) -> TessellationResult {
        TessellationResult {
            stamp: stamp.clone(),
            outcome: TessellationOutcome::Failed {
                diagnostics: vec![Diagnostic {
                    code: code.to_string(),
                    object: None,
                    message,
                }],
            },
        }
    }

    fn unsupported(
        stamp: &TaskStamp,
        exchange: ExchangeKind,
        code: &str,
        detail: String,
    ) -> TessellationResult {
        TessellationResult {
            stamp: stamp.clone(),
            outcome: TessellationOutcome::Unsupported {
                reason: UnsupportedReason {
                    code: code.to_string(),
                    exchange,
                    detail,
                },
            },
        }
    }
}

impl SolidTessellator for NoKernelTessellator {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "yacr.kernel.unsupported".into(),
            version: 1,
            priority: 0,
            entity_types: vec![
                "3DSOLID".into(),
                "BODY".into(),
                "REGION".into(),
                "SURFACE".into(),
            ],
            // No capability is claimed until an ACIS kernel is actually linked.
            capabilities: vec![],
        }
    }

    fn tessellate(
        &self,
        request: &TessellationRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<TessellationResult> {
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        request
            .validate()
            .map_err(|e| CadError::InvalidInput(e.to_string()))?;

        // Handle resolution is reported explicitly, never swallowed.
        if let GeometryHandle::Missing { key } = &request.geometry {
            return Ok(Self::failed(
                &request.stamp,
                codes::KERNEL_MISSING_HANDLE,
                format!("geometry handle '{key}' could not be resolved"),
            ));
        }

        // An empty payload is not an empty-but-successful solid.
        if request.exchange.is_empty() {
            return Ok(Self::failed(
                &request.stamp,
                codes::KERNEL_EMPTY_GEOMETRY,
                format!("{} payload carried no bytes", request.exchange.type_key()),
            ));
        }

        // A payload we cannot classify is its own category.
        if request.exchange.kind() == ExchangeKind::Unsupported {
            return Ok(Self::unsupported(
                &request.stamp,
                ExchangeKind::Unsupported,
                codes::KERNEL_UNSUPPORTED_EXCHANGE,
                format!(
                    "no decoder registered for exchange type '{}'",
                    request.exchange.type_key()
                ),
            ));
        }

        // Any remaining non-empty payload (raw SAT/SAB bytes or a neutral
        // B-rep): this conservative default evaluates none of them and never
        // fabricates a mesh. Opting into the documented subset means using
        // `BrepTessellator`.
        Ok(Self::unsupported(
            &request.stamp,
            request.exchange.kind(),
            codes::KERNEL_NO_ACIS_KERNEL,
            format!(
                "this build's default kernel evaluates no ACIS payload; {} payload left unsupported",
                request.exchange.kind().as_str()
            ),
        ))
    }
}

/// The real, acadrust-free tessellator for a neutral [`BrepData`] payload.
///
/// It evaluates the documented subset (planar faces with holes, full spheres,
/// full tori and full cylindrical strips) and reports everything else as a
/// missing face. It deliberately does **not** parse SAT/SAB bytes: only the
/// importer may do that. A raw `Sat`/`Sab` payload therefore stays
/// `Unsupported(kernel.no_acis_kernel)` even here.
#[derive(Debug, Default, Clone, Copy)]
pub struct BrepTessellator;

impl BrepTessellator {
    fn failed(stamp: &TaskStamp, code: &str, message: String) -> TessellationResult {
        TessellationResult {
            stamp: stamp.clone(),
            outcome: TessellationOutcome::Failed {
                diagnostics: vec![Diagnostic {
                    code: code.to_string(),
                    object: None,
                    message,
                }],
            },
        }
    }

    fn unsupported(
        stamp: &TaskStamp,
        exchange: ExchangeKind,
        code: &str,
        detail: String,
    ) -> TessellationResult {
        TessellationResult {
            stamp: stamp.clone(),
            outcome: TessellationOutcome::Unsupported {
                reason: UnsupportedReason {
                    code: code.to_string(),
                    exchange,
                    detail,
                },
            },
        }
    }
}

impl SolidTessellator for BrepTessellator {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "yacr.kernel.brep".into(),
            version: 1,
            priority: 1,
            entity_types: vec![
                "3DSOLID".into(),
                "BODY".into(),
                "REGION".into(),
                "SURFACE".into(),
            ],
            capabilities: vec![
                "planar-faces".into(),
                "planar-faces-with-holes".into(),
                "spherical-faces".into(),
                "toroidal-faces".into(),
                "cylindrical-faces".into(),
            ],
        }
    }

    fn tessellate(
        &self,
        request: &TessellationRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<TessellationResult> {
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        request
            .validate()
            .map_err(|e| CadError::InvalidInput(e.to_string()))?;

        if let GeometryHandle::Missing { key } = &request.geometry {
            return Ok(Self::failed(
                &request.stamp,
                codes::KERNEL_MISSING_HANDLE,
                format!("geometry handle '{key}' could not be resolved"),
            ));
        }
        if request.exchange.is_empty() {
            return Ok(Self::failed(
                &request.stamp,
                codes::KERNEL_EMPTY_GEOMETRY,
                format!(
                    "{} payload carried no geometry",
                    request.exchange.type_key()
                ),
            ));
        }

        match &request.exchange {
            SolidExchange::Brep(brep) => Ok(tessellate_brep(request, brep)),
            SolidExchange::Sat(_) | SolidExchange::Sab(_) => Ok(Self::unsupported(
                &request.stamp,
                request.exchange.kind(),
                codes::KERNEL_NO_ACIS_KERNEL,
                "raw SAT/SAB bytes are lifted by the importer, not by this tessellator".into(),
            )),
            SolidExchange::Unsupported { .. } => Ok(Self::unsupported(
                &request.stamp,
                ExchangeKind::Unsupported,
                codes::KERNEL_UNSUPPORTED_EXCHANGE,
                format!(
                    "no decoder registered for exchange type '{}'",
                    request.exchange.type_key()
                ),
            )),
        }
    }
}

/// Tessellate a neutral B-rep, enforcing the request budget and reporting
/// degradation exactly (missing faces, open edges, dropped shells).
fn tessellate_brep(request: &TessellationRequest, brep: &BrepData) -> TessellationResult {
    let tol = request.tolerance;
    let placement = brep.placement.unwrap_or_else(BrepPlacement::identity);

    let mut vertices: Vec<Point3> = Vec::new();
    let mut normals: Vec<Point3> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut face_sources: Vec<Option<SubElementId>> = Vec::new();
    let mut edges: Vec<Vec<Point3>> = Vec::new();
    let mut missing_faces: Vec<FaceRef> = Vec::new();
    let mut dropped_shells: Vec<ShellRef> = Vec::new();
    let mut all_unsupported = true;
    let mut all_planar = true;
    let mut error_bound = 0.0f64;

    for shell in &brep.shells {
        let mut shell_triangles = 0usize;
        for face in &shell.faces {
            match brep::tessellate_face(face, tol.linear_deflection, tol.angular_deflection) {
                Ok(fm) => {
                    if fm.curved {
                        all_planar = false;
                        error_bound = error_bound.max(fm.error_bound.unwrap_or(0.0));
                    }
                    let base = vertices.len() as u32;
                    let produced = fm.triangles.len();
                    for t in &fm.triangles {
                        triangles.push([base + t[0], base + t[1], base + t[2]]);
                        face_sources.push(None);
                    }
                    vertices.extend(fm.vertices.iter().map(|p| placement.apply(*p)));
                    normals.extend(fm.normals.iter().map(|n| placement.apply_direction(*n)));
                    for edge in fm.edges {
                        edges.push(edge.iter().map(|p| placement.apply(*p)).collect());
                    }
                    shell_triangles += produced;
                }
                Err(failure) => {
                    if !failure.is_unsupported() {
                        all_unsupported = false;
                    }
                    missing_faces.push(FaceRef {
                        id: face.id,
                        reason: failure.reason(),
                    });
                }
            }
        }
        if shell_triangles == 0 {
            dropped_shells.push(ShellRef {
                id: shell.id,
                reason: "no face in the shell could be tessellated".into(),
            });
        }
    }

    // No usable facet set is never a success.
    if triangles.is_empty() {
        if missing_faces.is_empty() {
            return BrepTessellator::failed(
                &request.stamp,
                codes::KERNEL_EMPTY_GEOMETRY,
                "neutral B-rep carried no tessellatable face".into(),
            );
        }
        if all_unsupported {
            let detail = missing_faces
                .iter()
                .map(|f| f.reason.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            return BrepTessellator::unsupported(
                &request.stamp,
                ExchangeKind::Brep,
                codes::KERNEL_UNSUPPORTED_SURFACE,
                detail,
            );
        }
        let diagnostics = TessellationDegradation {
            missing_faces,
            ..TessellationDegradation::default()
        }
        .diagnostics();
        return TessellationResult {
            stamp: request.stamp.clone(),
            outcome: TessellationOutcome::Failed { diagnostics },
        };
    }

    let open_edges = detect_open_edges(&vertices, &triangles);
    let precision = if all_planar {
        Precision::Analytic
    } else {
        Precision::Approximate {
            error_bound: Some(error_bound),
        }
    };
    let mesh = TessellationMesh {
        mesh: Mesh {
            vertices,
            triangles,
            normals,
            face_sources,
        },
        edges,
        precision,
    };

    if let Err(diagnostic) = request.budget.check(&mesh) {
        return TessellationResult {
            stamp: request.stamp.clone(),
            outcome: TessellationOutcome::Failed {
                diagnostics: vec![diagnostic],
            },
        };
    }

    let degradation = TessellationDegradation {
        missing_faces,
        open_edges,
        dropped_shells,
    };
    if degradation.is_empty() {
        TessellationResult {
            stamp: request.stamp.clone(),
            outcome: TessellationOutcome::Success {
                geometry: mesh,
                diagnostics: Vec::new(),
            },
        }
    } else {
        let diagnostics = degradation.diagnostics();
        TessellationResult {
            stamp: request.stamp.clone(),
            outcome: TessellationOutcome::Partial {
                geometry: mesh,
                degradation,
                diagnostics,
            },
        }
    }
}

/// Detect positional edges not shared by exactly two facets (open or
/// non-manifold) after welding coincident vertices within a relative epsilon.
fn detect_open_edges(vertices: &[Point3], triangles: &[[u32; 3]]) -> Vec<EdgeRef> {
    use std::collections::HashMap;
    if vertices.is_empty() {
        return Vec::new();
    }
    let (mut min, mut max) = (vertices[0], vertices[0]);
    for p in vertices {
        min = Point3 {
            x: min.x.min(p.x),
            y: min.y.min(p.y),
            z: min.z.min(p.z),
        };
        max = Point3 {
            x: max.x.max(p.x),
            y: max.y.max(p.y),
            z: max.z.max(p.z),
        };
    }
    let diag = {
        let d = brep::sub(max, min);
        brep::length(d)
    };
    let eps = (1e-9 * diag.max(1.0)).max(1e-12);
    let key = |p: Point3| -> (i64, i64, i64) {
        (
            (p.x / eps).round() as i64,
            (p.y / eps).round() as i64,
            (p.z / eps).round() as i64,
        )
    };
    let mut weld: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut welded = Vec::with_capacity(vertices.len());
    for p in vertices {
        let k = key(*p);
        let id = *weld.entry(k).or_insert_with(|| welded.len() as u32);
        welded.push(id);
    }
    let mut counts: HashMap<(u32, u32), u32> = HashMap::new();
    for t in triangles {
        let w = [
            welded[t[0] as usize],
            welded[t[1] as usize],
            welded[t[2] as usize],
        ];
        for e in [(w[0], w[1]), (w[1], w[2]), (w[2], w[0])] {
            let k = if e.0 <= e.1 { (e.0, e.1) } else { (e.1, e.0) };
            *counts.entry(k).or_insert(0) += 1;
        }
    }
    let mut open: Vec<EdgeRef> = counts
        .into_iter()
        .filter(|(_, c)| *c != 2)
        .map(|(_, c)| EdgeRef {
            id: 0,
            reason: if c == 1 {
                "boundary edge used by a single facet".to_string()
            } else {
                format!("non-manifold edge shared by {c} facets")
            },
        })
        .collect();
    open.sort_by(|a, b| a.reason.cmp(&b.reason));
    for (i, e) in open.iter_mut().enumerate() {
        e.id = i as u32;
    }
    open
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

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp() -> TaskStamp {
        TaskStamp::new(DocumentId(1), 1)
    }

    fn request(exchange: SolidExchange) -> TessellationRequest {
        TessellationRequest {
            geometry: GeometryHandle::Resolved(ObjectId(7)),
            exchange,
            tolerance: TessellationTolerance::default(),
            budget: TessellationBudget::default(),
            stamp: stamp(),
        }
    }

    fn no_cancel() -> bool {
        false
    }

    #[test]
    fn non_empty_sat_is_unsupported_not_empty_success() {
        // A non-empty ACIS payload must report `Unsupported`, never `Ok(empty)`.
        let req = request(SolidExchange::Sat(b"ACIS ...".to_vec()));
        let result = NoKernelTessellator.tessellate(&req, &no_cancel).unwrap();
        match &result.outcome {
            TessellationOutcome::Unsupported { reason } => {
                assert_eq!(reason.code, codes::KERNEL_NO_ACIS_KERNEL);
                assert_eq!(reason.exchange, ExchangeKind::Sat);
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
        assert!(result.outcome.mesh().is_none());
        assert!(!result.is_usable());
    }

    #[test]
    fn empty_payload_is_failed_not_success() {
        let req = request(SolidExchange::Sab(Vec::new()));
        let result = NoKernelTessellator.tessellate(&req, &no_cancel).unwrap();
        let TessellationOutcome::Failed { diagnostics } = result.outcome else {
            panic!("expected Failed for an empty payload");
        };
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, codes::KERNEL_EMPTY_GEOMETRY);
    }

    #[test]
    fn missing_handle_is_reported() {
        let mut req = request(SolidExchange::Sat(b"data".to_vec()));
        req.geometry = GeometryHandle::Missing {
            key: "acis:42".into(),
        };
        let result = NoKernelTessellator.tessellate(&req, &no_cancel).unwrap();
        let TessellationOutcome::Failed { diagnostics } = result.outcome else {
            panic!("expected Failed for an unresolved handle");
        };
        assert_eq!(diagnostics[0].code, codes::KERNEL_MISSING_HANDLE);
    }

    #[test]
    fn unknown_exchange_is_unsupported_exchange() {
        let req = request(SolidExchange::Unsupported {
            type_key: "vendor.cfd".into(),
            data: vec![1, 2, 3],
        });
        let result = NoKernelTessellator.tessellate(&req, &no_cancel).unwrap();
        let TessellationOutcome::Unsupported { reason } = result.outcome else {
            panic!("expected Unsupported");
        };
        assert_eq!(reason.code, codes::KERNEL_UNSUPPORTED_EXCHANGE);
    }

    #[test]
    fn bad_tolerance_is_rejected() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut req = request(SolidExchange::Sat(b"data".to_vec()));
            req.tolerance.linear_deflection = bad;
            let err = NoKernelTessellator
                .tessellate(&req, &no_cancel)
                .expect_err("bad tolerance must be rejected");
            match err {
                CadError::InvalidInput(message) => {
                    assert!(message.contains(codes::KERNEL_INVALID_TOLERANCE));
                }
                other => panic!("expected InvalidInput, got {other:?}"),
            }
        }
    }

    #[test]
    fn zero_budget_is_rejected() {
        let mut req = request(SolidExchange::Sat(b"data".to_vec()));
        req.budget.max_faces = 0;
        let err = NoKernelTessellator
            .tessellate(&req, &no_cancel)
            .expect_err("zero budget must be rejected");
        match err {
            CadError::InvalidInput(message) => {
                assert!(message.contains(codes::KERNEL_INVALID_BUDGET));
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn cancellation_wins_before_validation() {
        let mut req = request(SolidExchange::Sat(b"data".to_vec()));
        req.tolerance.linear_deflection = f64::NAN;
        let err = NoKernelTessellator
            .tessellate(&req, &|| true)
            .expect_err("cancelled request must error");
        assert_eq!(err, CadError::Cancelled);
    }

    #[test]
    fn budget_exceeded_is_reported_with_stable_code() {
        let mesh = TessellationMesh {
            mesh: Mesh {
                vertices: vec![
                    Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 0.0,
                        y: 1.0,
                        z: 0.0,
                    },
                ],
                triangles: vec![[0, 1, 2]],
                normals: vec![],
                face_sources: vec![None],
            },
            edges: vec![vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            ]],
            precision: Precision::Approximate {
                error_bound: Some(0.01),
            },
        };
        // `validate()` rejects zero, but `check` is the post-hoc enforcement a
        // real kernel uses; exercise it directly with a tiny non-zero cap.
        let budget = TessellationBudget {
            max_faces: 1,
            max_vertices: 1,
            max_edges: 1,
        };
        let diagnostic = budget
            .check(&mesh)
            .expect_err("1 face / 3 vertices must exceed a 1/1/1 budget");
        assert_eq!(diagnostic.code, codes::KERNEL_BUDGET_EXCEEDED);

        let generous = TessellationBudget::default();
        assert!(generous.check(&mesh).is_ok());
    }

    #[test]
    fn degradation_reports_stable_codes() {
        let degradation = TessellationDegradation {
            missing_faces: vec![FaceRef {
                id: 3,
                reason: "self-intersecting loop".into(),
            }],
            open_edges: vec![EdgeRef {
                id: 9,
                reason: "vertices not shared".into(),
            }],
            dropped_shells: vec![ShellRef {
                id: 1,
                reason: "contained a missing face".into(),
            }],
        };
        assert!(!degradation.is_empty());
        let codes: Vec<String> = degradation
            .diagnostics()
            .into_iter()
            .map(|d| d.code)
            .collect();
        assert!(codes.contains(&codes::KERNEL_MISSING_FACE.to_string()));
        assert!(codes.contains(&codes::KERNEL_OPEN_EDGE.to_string()));
        assert!(codes.contains(&codes::KERNEL_DROPPED_SHELL.to_string()));
        assert_eq!(codes.len(), 3);
    }

    #[test]
    fn partial_outcome_exposes_mesh_and_degradation() {
        let outcome = TessellationOutcome::Partial {
            geometry: TessellationMesh {
                mesh: Mesh::default(),
                edges: vec![],
                precision: Precision::Approximate { error_bound: None },
            },
            degradation: TessellationDegradation {
                missing_faces: vec![FaceRef {
                    id: 0,
                    reason: "unsupported surface".into(),
                }],
                ..TessellationDegradation::default()
            },
            diagnostics: vec![],
        };
        assert!(outcome.mesh().is_some());
        assert!(outcome.degradation().unwrap().missing_faces.len() == 1);
        assert!(!outcome.is_success());
    }

    #[test]
    fn entry_point_routes_through_the_seam() {
        let req = request(SolidExchange::Sat(b"ACIS".to_vec()));
        let result = tessellate_solid(&NoKernelTessellator, &req, &no_cancel).unwrap();
        assert_eq!(result.stamp, req.stamp);
        assert!(matches!(
            result.outcome,
            TessellationOutcome::Unsupported { .. }
        ));
    }

    #[test]
    fn default_registration_claims_no_capability() {
        let registration = NoKernelTessellator.registration();
        assert_eq!(registration.type_key, "yacr.kernel.unsupported");
        assert!(registration.capabilities.is_empty());
    }

    // ---- Neutral B-rep tessellation (real mesh subset) ----

    fn pt(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn line_loop(pts: &[[f64; 3]]) -> BrepLoop {
        BrepLoop {
            edges: (0..pts.len())
                .map(|i| {
                    let a = pts[i];
                    let b = pts[(i + 1) % pts.len()];
                    BrepCurve::Line {
                        start: pt(a[0], a[1], a[2]),
                        end: pt(b[0], b[1], b[2]),
                    }
                })
                .collect(),
        }
    }

    fn plane_face(id: u32, origin: [f64; 3], normal: [f64; 3], loop_pts: &[[f64; 3]]) -> BrepFace {
        BrepFace {
            id,
            surface: BrepSurface::Plane {
                origin: pt(origin[0], origin[1], origin[2]),
                normal: pt(normal[0], normal[1], normal[2]),
                u_dir: pt(1.0, 0.0, 0.0),
            },
            reversed: false,
            loops: vec![line_loop(loop_pts)],
        }
    }

    fn cube_brep(size: f64) -> BrepData {
        let s = size;
        let faces = vec![
            plane_face(
                0,
                [0.0, 0.0, 0.0],
                [0.0, 0.0, -1.0],
                &[[0.0, 0.0, 0.0], [0.0, s, 0.0], [s, s, 0.0], [s, 0.0, 0.0]],
            ),
            plane_face(
                1,
                [0.0, 0.0, s],
                [0.0, 0.0, 1.0],
                &[[0.0, 0.0, s], [s, 0.0, s], [s, s, s], [0.0, s, s]],
            ),
            plane_face(
                2,
                [0.0, 0.0, 0.0],
                [-1.0, 0.0, 0.0],
                &[[0.0, 0.0, 0.0], [0.0, 0.0, s], [0.0, s, s], [0.0, s, 0.0]],
            ),
            plane_face(
                3,
                [s, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                &[[s, 0.0, 0.0], [s, s, 0.0], [s, s, s], [s, 0.0, s]],
            ),
            plane_face(
                4,
                [0.0, 0.0, 0.0],
                [0.0, -1.0, 0.0],
                &[[0.0, 0.0, 0.0], [s, 0.0, 0.0], [s, 0.0, s], [0.0, 0.0, s]],
            ),
            plane_face(
                5,
                [0.0, s, 0.0],
                [0.0, 1.0, 0.0],
                &[[0.0, s, 0.0], [0.0, s, s], [s, s, s], [s, s, 0.0]],
            ),
        ];
        BrepData {
            shells: vec![BrepShell { id: 0, faces }],
            placement: None,
        }
    }

    fn brep_request(brep: BrepData) -> TessellationRequest {
        request(SolidExchange::Brep(brep))
    }

    #[test]
    fn brep_cube_is_a_closed_success() {
        let req = brep_request(cube_brep(1.0));
        let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
        let mesh = result.outcome.mesh().expect("cube must produce a mesh");
        assert_eq!(mesh.triangle_count(), 12, "cube face count / triangles");
        assert_eq!(mesh.precision, Precision::Analytic);
        let area = brep::mesh_area(&mesh.mesh);
        assert!((area - 6.0).abs() < 1e-9, "cube area {area}");
        assert!(matches!(
            result.outcome,
            TessellationOutcome::Success { .. }
        ));
    }

    #[test]
    fn brep_cube_placement_is_applied() {
        let mut brep = cube_brep(1.0);
        brep.placement = Some(BrepPlacement {
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: pt(10.0, 0.0, 0.0),
            scale: 1.0,
        });
        let req = brep_request(brep);
        let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
        let mesh = result.outcome.mesh().unwrap();
        assert!(mesh.mesh.vertices.iter().all(|p| p.x >= 10.0 - 1e-9));
    }

    #[test]
    fn brep_sphere_is_closed_and_monotone_in_tolerance() {
        let sphere = |tolerance: f64| {
            let brep = BrepData {
                shells: vec![BrepShell {
                    id: 0,
                    faces: vec![BrepFace {
                        id: 0,
                        surface: BrepSurface::Sphere {
                            center: pt(0.0, 0.0, 0.0),
                            radius: 1.0,
                            u_dir: pt(1.0, 0.0, 0.0),
                            pole: pt(0.0, 0.0, 1.0),
                        },
                        reversed: false,
                        loops: Vec::new(),
                    }],
                }],
                placement: None,
            };
            let mut req = brep_request(brep);
            req.tolerance.linear_deflection = tolerance;
            let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
            assert!(
                matches!(result.outcome, TessellationOutcome::Success { .. }),
                "sphere must be a closed success: {:?}",
                result.outcome
            );
            let mesh = result.outcome.mesh().unwrap();
            assert!(mesh.triangle_count() > 0);
            let err = match mesh.precision {
                Precision::Approximate {
                    error_bound: Some(e),
                } => e,
                ref other => panic!("sphere must report a chordal bound, got {other:?}"),
            };
            (mesh.triangle_count(), err)
        };
        let (coarse_tris, coarse_err) = sphere(0.5);
        let (fine_tris, fine_err) = sphere(0.05);
        assert!(
            fine_tris >= coarse_tris,
            "finer tolerance must not coarsen: {fine_tris} < {coarse_tris}"
        );
        assert!(
            fine_err <= coarse_err + 1e-12,
            "finer tolerance must not increase error: {fine_err} > {coarse_err}"
        );
    }

    #[test]
    fn brep_budget_exceeded_is_reported_not_truncated() {
        let mut brep = BrepData::default();
        brep.shells.push(BrepShell {
            id: 0,
            faces: vec![BrepFace {
                id: 0,
                surface: BrepSurface::Sphere {
                    center: pt(0.0, 0.0, 0.0),
                    radius: 1.0,
                    u_dir: pt(1.0, 0.0, 0.0),
                    pole: pt(0.0, 0.0, 1.0),
                },
                reversed: false,
                loops: Vec::new(),
            }],
        });
        let mut req = brep_request(brep);
        req.budget.max_faces = 2;
        let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
        let TessellationOutcome::Failed { diagnostics } = result.outcome else {
            panic!("over-budget sphere must fail, not truncate");
        };
        assert_eq!(diagnostics[0].code, codes::KERNEL_BUDGET_EXCEEDED);
    }

    #[test]
    fn brep_partial_reports_missing_face_and_open_edge() {
        // A lone planar square plus one unsupported face: usable facets, with
        // the unsupported face and the square's open boundary both explicit.
        let brep = BrepData {
            shells: vec![BrepShell {
                id: 7,
                faces: vec![
                    plane_face(
                        0,
                        [0.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0],
                        &[
                            [0.0, 0.0, 0.0],
                            [1.0, 0.0, 0.0],
                            [1.0, 1.0, 0.0],
                            [0.0, 1.0, 0.0],
                        ],
                    ),
                    BrepFace {
                        id: 5,
                        surface: BrepSurface::Unsupported {
                            type_key: "nurbs-surface".into(),
                        },
                        reversed: false,
                        loops: Vec::new(),
                    },
                ],
            }],
            placement: None,
        };
        let req = brep_request(brep);
        let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
        let TessellationOutcome::Partial {
            geometry,
            degradation,
            diagnostics,
        } = result.outcome
        else {
            panic!("expected Partial, got a different outcome");
        };
        assert_eq!(geometry.triangle_count(), 2);
        assert_eq!(degradation.missing_faces.len(), 1);
        assert_eq!(degradation.missing_faces[0].id, 5);
        assert!(!degradation.open_edges.is_empty());
        assert!(diagnostics
            .iter()
            .any(|d| d.code == codes::KERNEL_MISSING_FACE));
        assert!(diagnostics
            .iter()
            .any(|d| d.code == codes::KERNEL_OPEN_EDGE));
    }

    #[test]
    fn brep_all_unsupported_is_unsupported_not_empty_success() {
        let brep = BrepData {
            shells: vec![BrepShell {
                id: 0,
                faces: vec![BrepFace {
                    id: 0,
                    surface: BrepSurface::Unsupported {
                        type_key: "spline-surface".into(),
                    },
                    reversed: false,
                    loops: Vec::new(),
                }],
            }],
            placement: None,
        };
        let req = brep_request(brep);
        let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
        let TessellationOutcome::Unsupported { reason } = result.outcome else {
            panic!("all-unsupported B-rep must be Unsupported");
        };
        assert_eq!(reason.code, codes::KERNEL_UNSUPPORTED_SURFACE);
        assert!(!reason.detail.is_empty());
    }

    #[test]
    fn empty_brep_is_failed_not_success() {
        let req = brep_request(BrepData::default());
        let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
        let TessellationOutcome::Failed { diagnostics } = result.outcome else {
            panic!("empty B-rep must fail");
        };
        assert_eq!(diagnostics[0].code, codes::KERNEL_EMPTY_GEOMETRY);
    }

    #[test]
    fn brep_tessellator_never_parses_raw_bytes() {
        let req = request(SolidExchange::Sat(b"ACIS ...".to_vec()));
        let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
        let TessellationOutcome::Unsupported { reason } = result.outcome else {
            panic!("raw SAT must stay unsupported");
        };
        assert_eq!(reason.code, codes::KERNEL_NO_ACIS_KERNEL);
        assert_eq!(reason.exchange, ExchangeKind::Sat);
    }

    #[test]
    fn brep_tessellator_honours_cancellation() {
        let req = brep_request(cube_brep(1.0));
        assert_eq!(
            BrepTessellator.tessellate(&req, &|| true).unwrap_err(),
            CadError::Cancelled
        );
    }
}
