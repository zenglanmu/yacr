//! Stable, locale-independent diagnostic codes emitted by this crate.
//!
//! These codes are part of the contract: the diagnostics layer aggregates them
//! and the UI localises the message, nothing parses the human text.

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
