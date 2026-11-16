//! Booleans: `UNION` / `SUBTRACT` / `INTERSECT` through `solidcore_ops::boolean`.
//!
//! Both operands are lifted to records and rebuilt as kernel bodies first; a
//! body that cannot be lifted (any curved face or non-line boundary in P1) is
//! `Unsupported` with [`crate::LIFT_REQUIRES_PLANAR`] and the kernel is never
//! called. The upstream call **consumes** its operands, which is fine here: the
//! modeler only ever owns copies rebuilt from the caller's snapshot.
//!
//! # Empty results
//!
//! `solidcore_ops::boolean` answers `Ok(None)` for the empty set and
//! `Err(Error::Empty)` from its other families. Both become `Failed` with
//! [`crate::EMPTY_RESULT`]: an empty intersection is a correct geometric answer,
//! but it is not a body and must never surface as a success.
//!
//! # Cancellation
//!
//! The kernel polls a `CancelToken`. The shared [`watched`](super::watched)
//! helper runs a scoped watcher thread that polls the seam's `cancelled`
//! callback every [`POLL`](super::POLL) and cancels the token, joins it before
//! returning, and stays panic-safe. `Error::Cancelled` maps to
//! [`cad_domain::CadError::Cancelled`] and no product is returned. That is
//! cooperative, poll-based cancellation of the running call — not preemption —
//! and it is not claimed as preemption.

use cad_domain::{BrepData, CadError, CadResult};
use cad_kernel_adapter::{BooleanOp, SolidOperationFamily, SolidOutcome};
use solidcore_ops::Body;

use super::{body_outcome, lift, unlift, watched};
use crate::error;

const FAMILY: SolidOperationFamily = SolidOperationFamily::Boolean;

/// Apply `op` to `left` and `right`.
pub(crate) fn boolean(
    op: BooleanOp,
    left: &BrepData,
    right: &BrepData,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> CadResult<SolidOutcome> {
    let left_record = match lift(left) {
        Ok(record) => record,
        Err(refusal) => return Ok(error::unsupported(refusal, FAMILY)),
    };
    let right_record = match lift(right) {
        Ok(record) => record,
        Err(refusal) => return Ok(error::unsupported(refusal, FAMILY)),
    };
    let left_body = match Body::from_record(&left_record, false) {
        Ok((body, _order)) => body,
        Err(error) => return error::from_ops(error, FAMILY),
    };
    let right_body = match Body::from_record(&right_record, false) {
        Ok((body, _order)) => body,
        Err(error) => return error::from_ops(error, FAMILY),
    };
    // Last chance to drop the work before the kernel starts.
    if cancelled() {
        return Err(CadError::Cancelled);
    }

    let result = watched(cancelled, || {
        solidcore_ops::boolean::boolean(left_body, right_body, kernel_op(op))
    });

    // A result that arrived after cancellation is dropped: it is not a product.
    if cancelled() {
        return Err(CadError::Cancelled);
    }
    match result {
        Ok(Some(body)) => match body.to_record() {
            Ok((record, _order)) => match unlift(&record) {
                Ok(brep) => Ok(body_outcome(brep)),
                Err(refusal) => Ok(error::unsupported(refusal, FAMILY)),
            },
            Err(error) => error::from_ops(error, FAMILY),
        },
        Ok(None) => Ok(error::failed(
            error::EMPTY_RESULT,
            format!(
                "the {} result is empty: the operands do not overlap as requested",
                op.as_str()
            ),
        )),
        Err(error) => error::from_ops(error, FAMILY),
    }
}

fn kernel_op(op: BooleanOp) -> solidcore_ops::BooleanOp {
    match op {
        BooleanOp::Union => solidcore_ops::BooleanOp::Union,
        BooleanOp::Subtract => solidcore_ops::BooleanOp::Subtract,
        BooleanOp::Intersect => solidcore_ops::BooleanOp::Intersect,
    }
}
