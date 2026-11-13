//! The plugin API's **mutating** transport: submissions, validation, routing.
//!
//! Two lanes, kept apart on purpose:
//!
//! * [`super::api_call`] is the **read** lane: one synchronous answer served from
//!   the reflected snapshot;
//! * this module is the **submit** lane: `cad_api_submit` acknowledges that the
//!   request was queued (desktop) or applied (`dom-canvas`), and the caller polls
//!   `cad_api_poll` for the bounded FIFO of completed outcomes.
//!
//! There is no event-correlation channel on the wire, so the outcome is polled.
//! The desktop answers on the next owner tick (the IPC thread cannot touch the
//! `!Send` session); a synchronous carrier answers in the same call and keeps the
//! outcome on the session until the client polls.
//!
//! # Validation order
//!
//! 1. the `requestId` grammar ([`validate_request_id`] — empty, too long or
//!    outside `[A-Za-z0-9._-]` is `cad_api.invalid_request: requestId`);
//! 2. the namespace/operation (an unserved pair is
//!    `unsupported-runtime: cad_api.<ns>.<op>`, checked before host state);
//! 3. the operation itself (no document → `cad_api.no_document`, then the
//!    operation's own argument and revision rules).

use super::api_call::{
    api_call_unknown, validate_request_id, CadApiOutcome, CadApiResults, CadApiSubmission,
    CAD_API_RESULT_CAP,
};

impl super::CadSession {
    /// Answer one plugin-API mutation from the live session.
    ///
    /// Always answers (never panics, never fabricates): a refusal is
    /// [`CadApiOutcome`] with its stable marker and the submission's id. The
    /// outcome is **not** queued here — the caller decides where it lives (the
    /// desktop's shared result queue, or [`Self::push_api_outcome`] on a
    /// synchronous carrier).
    pub fn api_submit(&self, submission: CadApiSubmission) -> CadApiOutcome {
        let request_id = submission.request_id.clone();
        let outcome = if validate_request_id(&submission.request_id).is_err() {
            CadApiOutcome::refusal(super::api_call::API_SUBMIT_INVALID_REQUEST.to_string())
        } else {
            self.dispatch_submission(&submission)
        };
        outcome.with_request_id(request_id)
    }

    /// Keep one completed outcome until the client polls (bounded, oldest dropped).
    ///
    /// The synchronous carrier's sink is rebuilt for every dispatch call, so the
    /// session owns this queue; the desktop never uses it (its owner tick publishes
    /// into the host's shared queue instead).
    pub fn push_api_outcome(&self, outcome: CadApiOutcome) {
        let mut outcomes = self.api_outcomes.borrow_mut();
        push_api_result(&mut outcomes, outcome);
    }

    /// Drain every completed outcome, oldest first.
    pub fn take_api_outcomes(&self) -> Vec<CadApiOutcome> {
        std::mem::take(&mut *self.api_outcomes.borrow_mut())
    }

    /// Route one already-validated submission to its operation.
    fn dispatch_submission(&self, submission: &CadApiSubmission) -> CadApiOutcome {
        match (submission.namespace.as_str(), submission.operation.as_str()) {
            (
                super::api_call::API_CALL_DATABASE_NAMESPACE,
                super::api_call::API_CALL_TRANSACTION_COMMIT,
            ) => self.commit_outcome(submission.parameters.as_ref()),
            (namespace, operation) => {
                CadApiOutcome::refusal(api_call_unknown(namespace, operation))
            }
        }
    }
}

/// Answer a submission on the owner thread, with or without a live session.
///
/// `None` is the honest `cad_waiting` for that request: the desktop's session is
/// created lazily, so a submission that arrives before it exists must still get a
/// correlated answer instead of vanishing from the queue.
pub fn api_submit_or_wait(
    session: Option<&super::CadSession>,
    submission: CadApiSubmission,
) -> CadApiOutcome {
    match session {
        Some(session) => session.api_submit(submission),
        None => CadApiOutcome::waiting(submission.request_id),
    }
}

/// Push one outcome into a bounded result queue, dropping the oldest at the cap.
///
/// Shared by both carriers (the desktop's shared queue and the session's own), so
/// the "bounded, oldest dropped" rule cannot differ between them.
pub fn push_api_result(results: &mut Vec<CadApiOutcome>, outcome: CadApiOutcome) {
    if results.len() >= CAD_API_RESULT_CAP {
        results.remove(0);
    }
    results.push(outcome);
}

/// The poll answer for a queue of finished outcomes (drains it).
pub(super) fn take_results(results: &mut Vec<CadApiOutcome>) -> CadApiResults {
    CadApiResults {
        results: std::mem::take(results),
    }
}
