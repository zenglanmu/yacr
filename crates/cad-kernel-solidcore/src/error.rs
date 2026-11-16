//! Stable reason codes, refusal values and the one place upstream errors become
//! seam outcomes.
//!
//! Mapping rules (ADR-0036 D1, [`cad_kernel_adapter::SolidModeler::execute`]):
//!
//! | upstream | seam |
//! |---|---|
//! | `Error::Unsupported` / `Unported` | `Ok(Unsupported)` with [`cad_kernel_adapter::codes::SOLID_UNSUPPORTED`] |
//! | `Error::UnsureRegion` | `Ok(Failed)` with [`UNSURE_REGION`] (a wrong *diagnosis* as `Unsupported`: the kernel ran but could not decide) |
//! | `Error::Empty` | `Ok(Failed)` with [`EMPTY_RESULT`] |
//! | `Error::Internal` (the session caught a kernel panic) | `Ok(Failed)` with [`KERNEL_PANIC`] and a **generic** message |
//! | `Error::Cancelled` | `Err(CadError::Cancelled)` |
//! | anything else (a kernel status, a refusal, …) | `Ok(Failed)` with [`KERNEL_FAILED`] |
//!
//! The `Display` wording of an upstream error is passed through as diagnostic
//! **text** only: callers act on the stable code, never on the message. The one
//! exception is `Error::Internal`, whose `Display` embeds the panic payload: a
//! panic's message is an implementation detail (and may quote internals), so it
//! is replaced by a fixed sentence and the stable [`KERNEL_PANIC`] code.
//!
//! Chamfer has its own mapping ([`from_chamfer`]) because its outcomes split by
//! cause: a **geometric** refusal (`NoRoom`, `TooLarge`, `NoEffect`, `Refused`,
//! `Status`, blend refusals) becomes `Unsupported` with [`CHAMFER_FAILED`], while
//! a **kernel-integrity** failure (`Record`, `NotSolid`, `NotExact`, `Degenerate`,
//! `Argument`, and the non-exhaustive `NotSolidBecause` through the catch-all)
//! stays `Failed` with [`KERNEL_FAILED`]. A kernel failure must never be reported
//! as a missing capability.
//!
//! # Refusal codes (conversion, before the kernel runs)
//!
//! | code | used for | sites |
//! |---|---|---|
//! | [`LIFT_REQUIRES_PLANAR`] | **only** a non-`Plane` face or a non-`Line` boundary curve in `BrepData → record` | `to_record.rs` (`record_face_surface`, the loop builder) |
//! | [`LIFT_UNSUPPORTED`] | every structural `BrepData → record` refusal (more than one shell, empty loop, coincident endpoints, tolerance, record validation, closed-shell/chain/agreement checks) **and** every `record → BrepData` refusal (unsupported surface/curve family, void region, wire/vertex shell, a vertex loop other than a full cone's degenerate apex, trimmed face, torus tube/radii, arc edge) | `to_record.rs`, `from_record.rs` |
//! | [`PLACEMENT_NOT_SUPPORTED`] | a placement or plane frame that cannot be baked (mirror, non-positive/non-finite scale, singular or sheared matrix, degenerate frame) | `record/mod.rs` (`bake`), `record/surface.rs` (`record_plane`), `to_record.rs` (degenerate frame) |
//! | [`NONUNIFORM_TRANSFORM`] | a `Transform` request that is not a similarity | `ops/transform.rs` |

use cad_domain::{CadError, CadResult, Diagnostic};
use cad_kernel_adapter::{codes, SolidOperationFamily, SolidOutcome, SolidUnsupportedReason};
use solidcore_ops::Error;

/// The body has a **non-planar face** or a **non-`Line` boundary curve**, so it
/// cannot be lifted from the neutral B-rep into a kernel record.
///
/// This code is reserved for exactly those two sites: every boolean or
/// mass-properties request on a curved body returns it. Everything else that
/// stops a conversion uses [`LIFT_UNSUPPORTED`] or [`PLACEMENT_NOT_SUPPORTED`],
/// so a caller can tell "curved lift is P2" apart from "this body is structurally
/// unusable".
pub const LIFT_REQUIRES_PLANAR: &str = "solid.lift_requires_planar";

/// A conversion refused for a structural reason.
///
/// `record → BrepData`: an unsupported surface or curve family, a void region, a
/// `wire`/`vertex` shell, a vertex loop (except the degenerate apex a full cone
/// needs), a trimmed face, a torus whose tube starts off its equator or is not a
/// ring, an arc edge, a face shell with embedded wire edges. `BrepData → record`:
/// more than one shell, an empty loop, coincident endpoints, a bad tolerance, a
/// record that fails validation or its geometry/consistency checks.
///
/// Used by **both** directions: the detail says which, the code says "not a
/// kernel limitation". A multi-shell body is *accepted* by `record → BrepData`
/// (a disjoint `UNION` is legitimate) and refused by `BrepData → record`, so a
/// body can exist whose next lift is refused — mass properties therefore see
/// `Unsupported` for something a boolean successfully produced.
pub const LIFT_UNSUPPORTED: &str = "solid.lift_unsupported";

/// A placement or plane frame cannot be baked (a mirror, a non-positive or
/// non-finite scale, a singular or sheared matrix, a degenerate frame), so the
/// request is refused instead of moving the geometry in a way the neutral model
/// cannot represent.
pub const PLACEMENT_NOT_SUPPORTED: &str = "solid.placement_not_supported";

/// The requested transform is not a similarity (non-uniform scale, shear), so it
/// cannot be folded into [`cad_domain::BrepPlacement`] and is not baked
/// approximately (ADR-0036 D4).
pub const NONUNIFORM_TRANSFORM: &str = "solid.nonuniform_transform";

/// The kernel refused the chamfer itself: a size too large for its faces, no
/// room, an edge it cannot blend, a size with no effect, or an argument it
/// rejected. Reported as `Unsupported` (never as a plausible body): this build
/// cannot build the requested chamfer exactly.
pub const CHAMFER_FAILED: &str = "solid.chamfer_failed";

/// No edge of the lifted body lies within the selector's resolution tolerance of
/// the point (`max(1e-9, 1e-3 · bounding-box diagonal)`), so nothing was picked.
///
/// Reported as `Unsupported` rather than chamfering the nearest edge anyway: a
/// fuzzy pick would cut geometry the caller did not name.
pub const EDGE_NOT_FOUND: &str = "solid.edge_not_found";

/// The kernel ran but produced no usable product: a non-closed solid, a refused
/// input, a non-finite mass property. Never a success, never an empty body.
pub const KERNEL_FAILED: &str = "solid.kernel_failed";

/// A boolean produced the empty set. Reported as `Failed` with this code: an
/// empty result is a legitimate answer to a geometric question, but it is not a
/// body, and returning one would be a fake success.
pub const EMPTY_RESULT: &str = "solid.empty_result";

/// The kernel is unsure whether a region is material, and the answer depends on
/// it (mass properties). Reported as `Failed`: the call ran and could not
/// decide, which is not a missing capability.
pub const UNSURE_REGION: &str = "solid.unsure_region";

/// The session caught a panic inside the kernel during this call. Reported as
/// `Failed` with a generic message: the panic's own text is not forwarded.
pub const KERNEL_PANIC: &str = "solid.kernel_panic";

/// Why a conversion refused, kept as data so callers act on the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiftRefusal {
    /// One of the stable codes above.
    pub code: &'static str,
    /// Human-facing detail (diagnostics only, never parsed).
    pub detail: String,
}

impl LiftRefusal {
    /// The body has a non-planar face or a non-line boundary (the only
    /// [`LIFT_REQUIRES_PLANAR`] sites).
    pub(crate) fn non_planar(detail: impl Into<String>) -> Self {
        LiftRefusal {
            code: LIFT_REQUIRES_PLANAR,
            detail: detail.into(),
        }
    }

    /// A structural conversion refusal (see [`LIFT_UNSUPPORTED`]).
    pub(crate) fn unsupported(detail: impl Into<String>) -> Self {
        LiftRefusal {
            code: LIFT_UNSUPPORTED,
            detail: detail.into(),
        }
    }

    /// A placement/frame refusal (see [`PLACEMENT_NOT_SUPPORTED`]).
    pub(crate) fn placement(detail: impl Into<String>) -> Self {
        LiftRefusal {
            code: PLACEMENT_NOT_SUPPORTED,
            detail: detail.into(),
        }
    }

    /// A refusal with an explicit stable code.
    pub(crate) fn new(code: &'static str, detail: impl Into<String>) -> Self {
        LiftRefusal {
            code,
            detail: detail.into(),
        }
    }
}

/// Turn a refusal into the seam's structured `Unsupported` outcome.
pub(crate) fn unsupported(refusal: LiftRefusal, operation: SolidOperationFamily) -> SolidOutcome {
    SolidOutcome::Unsupported {
        reason: SolidUnsupportedReason {
            code: refusal.code,
            operation,
            detail: refusal.detail,
        },
    }
}

/// A `Failed` outcome carrying one diagnostic with a stable code.
pub(crate) fn failed(code: &'static str, message: impl Into<String>) -> SolidOutcome {
    SolidOutcome::Failed {
        diagnostics: vec![Diagnostic {
            object: None,
            code: code.to_string(),
            message: message.into(),
        }],
    }
}

/// Map an upstream operation error onto the seam contract.
///
/// Returns `Err(CadError::Cancelled)` for a cancelled call (there is no
/// `aborted` outcome: a cancelled request has no product) and
/// `Ok(SolidOutcome::…)` otherwise, so the caller can keep one code path.
pub(crate) fn from_ops(error: Error, operation: SolidOperationFamily) -> CadResult<SolidOutcome> {
    match error {
        Error::Cancelled => Err(CadError::Cancelled),
        // The kernel ran and cannot build this exactly: a capability refusal.
        // `Unported` is the upstream marker for a routine that the port has not
        // carried yet, which is the same statement from the caller's side.
        Error::Unsupported(what) | Error::Unported(what) => Ok(unsupported(
            LiftRefusal {
                code: codes::SOLID_UNSUPPORTED,
                detail: format!("solidcore does not build this yet: {what}"),
            },
            operation,
        )),
        // The call ran and could not decide: a wrong *diagnosis* as Unsupported.
        Error::UnsureRegion { region } => Ok(failed(
            UNSURE_REGION,
            format!("the kernel is unsure whether region {region} is material"),
        )),
        // A boolean that produced nothing: an answer, not a body.
        Error::Empty => Ok(failed(EMPTY_RESULT, "the boolean result is empty")),
        // The session's panic guard: the payload can quote kernel internals, so
        // only the stable fact is reported.
        Error::Internal(_) => Ok(failed(
            KERNEL_PANIC,
            "the kernel panicked during this call; the session replaced it, please retry",
        )),
        other => Ok(failed(KERNEL_FAILED, other.to_string())),
    }
}

/// Map an upstream chamfer error onto the seam contract.
///
/// A chamfer is an *edit*, so its refusals fall into two very different classes:
///
/// | upstream | seam | why |
/// |---|---|---|
/// | `Error::Cancelled` | `Err(CadError::Cancelled)` | no product |
/// | `Error::Internal(_)` | `Ok(Failed)` with [`KERNEL_PANIC`], generic text | caught kernel panic |
/// | `Error::Empty` | `Ok(Failed)` with [`EMPTY_RESULT`] | a chamfer that produced nothing |
/// | `Error::UnsureRegion` | `Ok(Failed)` with [`UNSURE_REGION`] | ran, could not decide |
/// | `Error::Unsupported` / `Unported` | `Ok(Unsupported)` with [`codes::SOLID_UNSUPPORTED`] | not ported/built |
/// | `NoRoom`, `TooLarge`, `NoEffect`, `Refused`, `Status`, blend refusals | `Ok(Unsupported)` with [`CHAMFER_FAILED`] | the kernel refused this geometric edit |
/// | `Record`, `NotSolid`, `NotSolidBecause`, `NotExact`, `Degenerate`, `Argument` | `Ok(Failed)` with [`KERNEL_FAILED`] | the kernel could not vouch for the body/record it ran on |
///
/// The distinction matters: `Unsupported` says "this build cannot build it",
/// while `Failed` says "the kernel ran and produced no trustworthy product" —
/// never an empty success. The non-exhaustive `Error::NotSolidBecause` variant
/// cannot be named outside `solidcore-ops`, so it lands in the `Failed`
/// catch-all along with `Exception`, `Invariant`, `Reentered` and refusals of
/// operations a chamfer never runs.
pub(crate) fn from_chamfer(error: Error) -> CadResult<SolidOutcome> {
    match error {
        Error::Cancelled => Err(CadError::Cancelled),
        Error::Internal(_) => Ok(failed(
            KERNEL_PANIC,
            "the kernel panicked during this call; the session replaced it, please retry",
        )),
        Error::Empty => Ok(failed(EMPTY_RESULT, "the chamfer result is empty")),
        Error::UnsureRegion { region } => Ok(failed(
            UNSURE_REGION,
            format!("the kernel is unsure whether region {region} is material"),
        )),
        Error::Unsupported(what) | Error::Unported(what) => Ok(unsupported(
            LiftRefusal {
                code: codes::SOLID_UNSUPPORTED,
                detail: format!("solidcore does not build this yet: {what}"),
            },
            SolidOperationFamily::Chamfer,
        )),
        // Geometric refusals of this chamfer: no room, too large, no effect, an
        // argument the kernel rejects, a status it reports as a refusal, or a
        // blend refusal (the chamfer is a blend at the kernel's level).
        error @ (Error::NoRoom
        | Error::TooLarge { .. }
        | Error::NoEffect
        | Error::Refused { .. }
        | Error::Status(_)
        | Error::ChamferAngleTooLarge { .. }
        | Error::EdgeNotBlendable { .. }
        | Error::ContinuesInto { .. }
        | Error::RoundTooSmall { .. }
        | Error::SetbackTooShort { .. }
        | Error::SetbackTooLong { .. }
        | Error::CornerRefused { .. }
        | Error::CornerUnsupported
        | Error::RoundsCross { .. }
        | Error::BlendFailed { .. }
        | Error::BlendNotCertified { .. }) => chamfer_refusal(error),
        // Kernel-integrity failures: the kernel ran on a record/body it cannot
        // vouch for, or produced a result it cannot certify. These are `Failed`,
        // exactly as the boolean path reports them.
        error @ (Error::Record(_)
        | Error::NotSolid
        | Error::NotExact
        | Error::Degenerate
        | Error::Argument) => Ok(failed(
            KERNEL_FAILED,
            format!("solidcore failed the chamfer: {error}"),
        )),
        // Anything not named above is a kernel failure, never a capability
        // claim. That includes `Error::NotSolidBecause`, which is a
        // `#[non_exhaustive]` tuple variant and cannot be named from this crate,
        // plus `Exception`/`Invariant`/`Reentered` and refusals of operations a
        // chamfer never runs.
        other => Ok(failed(
            KERNEL_FAILED,
            format!("solidcore failed the chamfer: {other}"),
        )),
    }
}

/// The chamfer-specific `Unsupported`: this build cannot build the request
/// exactly, and an `Unsupported` outcome can never be mistaken for a body.
fn chamfer_refusal(error: Error) -> CadResult<SolidOutcome> {
    Ok(unsupported(
        LiftRefusal {
            code: CHAMFER_FAILED,
            detail: format!("solidcore refused the chamfer: {error}"),
        },
        SolidOperationFamily::Chamfer,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILY: SolidOperationFamily = SolidOperationFamily::Boolean;

    fn outcome(error: Error) -> SolidOutcome {
        from_ops(error, FAMILY).expect("only cancellation is an Err")
    }

    fn failed_code(outcome: &SolidOutcome) -> &str {
        match outcome {
            SolidOutcome::Failed { diagnostics } => diagnostics
                .first()
                .map_or("", |diagnostic| diagnostic.code.as_str()),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn cancellation_is_an_error_with_no_product() {
        assert_eq!(
            from_ops(Error::Cancelled, FAMILY).unwrap_err(),
            CadError::Cancelled
        );
    }

    #[test]
    fn each_upstream_error_keeps_its_own_code() {
        // A capability the port does not have: Unsupported.
        for error in [Error::Unsupported("fillet"), Error::Unported("ssi")] {
            let outcome = outcome(error);
            assert_eq!(
                outcome.unsupported().map(|reason| reason.code),
                Some(codes::SOLID_UNSUPPORTED)
            );
        }

        // The kernel ran and could not decide: Failed, not Unsupported.
        let unsure = outcome(Error::UnsureRegion { region: 7 });
        assert!(unsure.unsupported().is_none());
        assert_eq!(failed_code(&unsure), UNSURE_REGION);
        match &unsure {
            SolidOutcome::Failed { diagnostics } => {
                assert!(diagnostics[0].message.contains('7'), "{:?}", diagnostics[0]);
            }
            other => panic!("expected Failed, got {other:?}"),
        }

        // An empty boolean is an answer, not a body.
        assert_eq!(failed_code(&outcome(Error::Empty)), EMPTY_RESULT);

        // A caught kernel panic: its payload must never reach the diagnostic.
        let panic = outcome(Error::Internal(
            "panicked at src/kernel.rs:142: assertion `left == right` failed".into(),
        ));
        assert_eq!(failed_code(&panic), KERNEL_PANIC);
        match &panic {
            SolidOutcome::Failed { diagnostics } => {
                assert!(
                    !diagnostics[0].message.contains("assertion")
                        && !diagnostics[0].message.contains("kernel.rs"),
                    "panic text leaked: {}",
                    diagnostics[0].message
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }

        // Everything else is the generic kernel failure, with the upstream
        // wording kept as text only.
        for error in [
            Error::Status(1001),
            Error::Argument,
            Error::NotSolid,
            Error::NoRoom,
            Error::Degenerate,
        ] {
            assert_eq!(failed_code(&outcome(error)), KERNEL_FAILED);
        }
    }

    #[test]
    fn chamfer_geometric_refusals_are_unsupported_not_plausible_bodies() {
        // A chamfer the kernel will not build for this geometry is a
        // capability-style refusal: Unsupported, never a made-up body.
        for error in [
            Error::Status(1001),
            Error::NoRoom,
            Error::NoEffect,
            Error::Refused {
                operation: "chamfer_sides",
                reason: "the faces make no wedge",
            },
            Error::TooLarge {
                size: 0.2,
                kind: solidcore_ops::FitKind::Distance,
                at: None,
                why: None,
            },
        ] {
            let outcome = from_chamfer(error).expect("only cancellation is an Err");
            assert_eq!(
                outcome.unsupported().map(|reason| reason.code),
                Some(CHAMFER_FAILED),
                "{outcome:?}"
            );
            assert_eq!(
                outcome.unsupported().map(|reason| reason.operation),
                Some(SolidOperationFamily::Chamfer)
            );
            assert!(outcome.body().is_none() && outcome.mass().is_none());
        }
    }

    #[test]
    fn chamfer_integrity_failures_are_failed_not_capability_refusals() {
        // The kernel ran on a record/body it cannot vouch for (or produced a
        // result it cannot certify): that is a failure, not "this build cannot
        // do chamfers". Same class the boolean path reports as KERNEL_FAILED.
        for error in [
            Error::Record("a record the conversion cannot carry".into()),
            Error::NotSolid,
            Error::NotExact,
            Error::Degenerate,
            Error::Argument,
        ] {
            let outcome = from_chamfer(error).expect("only cancellation is an Err");
            assert!(
                outcome.unsupported().is_none(),
                "an integrity failure must not be Unsupported: {outcome:?}"
            );
            assert_eq!(failed_code(&outcome), KERNEL_FAILED, "{outcome:?}");
            assert!(outcome.body().is_none() && outcome.mass().is_none());
        }
    }

    #[test]
    fn chamfer_keeps_cancellation_and_panic_mappings() {
        assert_eq!(
            from_chamfer(Error::Cancelled).unwrap_err(),
            CadError::Cancelled
        );
        let panic = from_chamfer(Error::Internal("panicked at x.rs".into())).expect("not an Err");
        assert_eq!(failed_code(&panic), KERNEL_PANIC);
        // A capability the kernel does not have stays SOLID_UNSUPPORTED, so a
        // caller can tell "not ported" from "wrong size for this geometry".
        for error in [Error::Unsupported("chamfer"), Error::Unported("blend")] {
            let outcome = from_chamfer(error).expect("not an Err");
            assert_eq!(
                outcome.unsupported().map(|reason| reason.code),
                Some(codes::SOLID_UNSUPPORTED)
            );
        }
    }
}
