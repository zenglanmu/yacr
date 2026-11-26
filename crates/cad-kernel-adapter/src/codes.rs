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
/// The mesh was produced but does **not** meet the requested fidelity: a face the
/// mesher could only fill leniently, a face or edge whose refinement stopped at a
/// bound above the chord tolerance, or an edge that could not be sampled at all.
/// Never used for a missing entity (that is `kernel.missing_face`); always paired
/// with a `Partial` outcome.
pub const KERNEL_TOLERANCE_NOT_MET: &str = "kernel.tolerance_not_met";
/// The request was cancelled before a result could be produced.
pub const KERNEL_CANCELLED: &str = "kernel.cancelled";

// ---------------------------------------------------------------------------
// Solid modelling seam (`SolidModeler`, ADR-0036 D1/D7).
//
// These are the input/capability categories of the modelling seam, separate
// from the tessellation codes above: a body can be modelled and still fail to
// tessellate, and vice versa.
// ---------------------------------------------------------------------------

/// A primitive length/radius or a chamfer set-back was non-finite,
/// non-positive, or above the seam's limit.
pub const SOLID_INVALID_DIMENSION: &str = "solid.invalid_dimension";
/// A primitive origin/base centre coordinate was not finite.
pub const SOLID_INVALID_POINT: &str = "solid.invalid_point";
/// A mass-properties density was non-finite or not strictly positive.
pub const SOLID_INVALID_DENSITY: &str = "solid.invalid_density";
/// A transform matrix was non-finite, not affine, or singular.
pub const SOLID_INVALID_TRANSFORM: &str = "solid.invalid_transform";
/// A boolean/transform/mass operand carried no B-rep geometry.
pub const SOLID_EMPTY_OPERAND: &str = "solid.empty_operand";
/// The modeler was asked for a capability it does not declare.
pub const SOLID_UNSUPPORTED: &str = "solid.unsupported";
/// The operand carries a curved (non-planar) face and the modeler does not
/// declare the `nurbs` capability, so its B-rep cannot be lifted back into the
/// kernel. Distinct from [`SOLID_UNSUPPORTED`] so a caller can tell "this
/// modeler cannot carry curved geometry" apart from "this family is unknown".
pub const SOLID_NURBS_NOT_SUPPORTED: &str = "solid.nurbs_not_supported";
/// No face of the lifted body lies within the selector's resolution tolerance
/// of the point, so nothing was picked.
pub const SOLID_FACE_NOT_FOUND: &str = "solid.face_not_found";
/// The kernel refused the requested face move itself (no room, a collapsing
/// offset, folds), so this build cannot move that face by that distance
/// exactly. Reported as `Unsupported`, never as an approximate body.
pub const SOLID_OFFSET_FACE_FAILED: &str = "solid.offset_face_failed";
/// The kernel refused the requested profile extrusion (open/invalid profile, a
/// zero-area loop, a cap it cannot build), so this build cannot make that solid
/// exactly. Reported as `Unsupported`, never as an approximate body.
pub const SOLID_EXTRUDE_PROFILE_FAILED: &str = "solid.extrude_profile_failed";
/// The kernel ran but produced no usable body (for example an empty boolean
/// result). Never an empty success.
pub const SOLID_FAILED: &str = "solid.failed";

// ---------------------------------------------------------------------------
// Mesh modelling seam (`MeshModeler`, ADR-0061 / P-MESH-0).
//
// These are input/repair categories of the mesh seam, separate from the solid
// codes above: a mesh boolean is not an exact B-rep operation, and an open
// shell is a named refusal rather than a failure.
// ---------------------------------------------------------------------------

/// A mesh operand has no vertices or no triangles.
pub const MESH_EMPTY: &str = "mesh.empty";
/// A mesh operand has a non-finite position, an out-of-range index or a
/// triangle that repeats an index.
pub const MESH_INVALID: &str = "mesh.invalid_mesh";
/// The kernel/repair requires a geometrically closed and orientable mesh;
/// this one is open (a boundary edge exists). Hole filling is P-MESH-3 and is
/// deliberately not attempted here.
pub const MESH_NOT_CLOSED: &str = "mesh.not_closed";
/// A pairing-dependent kernel operation was asked of a closed triangle soup
/// that is not a 2-manifold. The robust boolean may still accept it; operations
/// that need halfedge pairing do not.
pub const MESH_NOT_MANIFOLD: &str = "mesh.not_manifold";
/// The modeler was asked for a capability it does not declare.
pub const MESH_UNSUPPORTED: &str = "mesh.unsupported";
/// The kernel ran but produced no usable mesh (for example an empty boolean
/// result). Never an empty success.
pub const MESH_FAILED: &str = "mesh.failed";
/// The mesh result exceeded the kernel's index/result limits.
pub const MESH_RESULT_TOO_LARGE: &str = "mesh.result_too_large";
