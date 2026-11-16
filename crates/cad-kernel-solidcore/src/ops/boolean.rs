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
//! The kernel polls a `CancelToken`. A scoped watcher thread polls the seam's
//! `cancelled` callback every [`POLL`] and cancels the token; the call itself runs
//! inside `solidcore_ops::cancel::within`, and the watcher is joined before this
//! function returns. `Error::Cancelled` maps to
//! [`cad_domain::CadError::Cancelled`] and no product is returned. That is
//! cooperative, poll-based cancellation of the running call — not preemption —
//! and it is not claimed as preemption. **Detection granularity is [`POLL`]**
//! (50 ms) while the kernel runs; finishing wakes the watcher immediately
//! (`StopGuard` unparks it), so a completed call does not wait a poll. A panic
//! inside the call stops the watcher through the same guard's `Drop`, so the
//! scope joins instead of hanging.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cad_domain::{BrepData, CadError, CadResult};
use cad_kernel_adapter::{BooleanOp, SolidOperationFamily, SolidOutcome};
use solidcore_ops::cancel::{within, CancelToken};
use solidcore_ops::Body;

use super::{body_outcome, lift, unlift};
use crate::error;

/// How often the watcher thread polls the seam's cancellation callback.
const POLL: Duration = Duration::from_millis(50);

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

    let token = CancelToken::new();
    let stop = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        let watcher_token = token.clone();
        let watcher_stop = &stop;
        let watcher = scope.spawn(move || {
            while !watcher_stop.load(Ordering::Relaxed) {
                if cancelled() {
                    watcher_token.cancel();
                    return;
                }
                // `StopGuard` unparks us when the call ends, so finishing never
                // waits a whole `POLL`; `POLL` is the cancellation-detection
                // granularity while the kernel runs.
                std::thread::park_timeout(POLL);
            }
        });
        let watcher_thread = watcher.thread().clone();
        // The guard is created *before* the kernel call and sets the flag in
        // `Drop`, so a panic inside `within` still stops the watcher: the scope
        // joins it while unwinding instead of hanging on a sleeper.
        let guard = StopGuard {
            stop: &stop,
            thread: &watcher_thread,
        };
        let result = within(&token, || {
            solidcore_ops::boolean::boolean(left_body, right_body, kernel_op(op))
        });
        drop(guard);
        // Joined here (not at the end of the scope) so the post-check below is
        // about the call, not about the watcher.
        let _ = watcher.join();
        result
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

/// Stops the watcher thread, including while unwinding.
///
/// Sets the flag and unparks the thread on `Drop`: a panic inside the kernel call
/// must not leave `thread::scope` waiting for a parked or sleeping watcher (the
/// scope joins before it resumes the unwind).
struct StopGuard<'a> {
    stop: &'a AtomicBool,
    thread: &'a std::thread::Thread,
}

impl Drop for StopGuard<'_> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.unpark();
    }
}

fn kernel_op(op: BooleanOp) -> solidcore_ops::BooleanOp {
    match op {
        BooleanOp::Union => solidcore_ops::BooleanOp::Union,
        BooleanOp::Subtract => solidcore_ops::BooleanOp::Subtract,
        BooleanOp::Intersect => solidcore_ops::BooleanOp::Intersect,
    }
}
