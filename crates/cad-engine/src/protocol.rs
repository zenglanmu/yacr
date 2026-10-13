//! Typed request and completion contracts for the engine boundary.
//!
//! This is an in-process semantic contract, not a stable serialized schema or
//! an implementation of request scheduling.

use cad_app::{interaction::InteractionRequest, Command, CommandOutcome};
use cad_domain::{CadError, DocumentId, ViewportId};

/// Engine-scoped monotonic request identity, distinct from domain/platform IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub u64);

pub enum EngineRequest {
    Command(Command),
    Interaction(InteractionRequest),
}

/// Context used to detect requests targeting replaced or stale state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestContext {
    pub document: DocumentId,
    pub document_generation: u64,
    pub viewport: ViewportId,
    pub tool_generation: Option<u64>,
    pub metrics_revision: Option<u64>,
}

pub struct RequestEnvelope {
    pub id: RequestId,
    pub context: RequestContext,
    pub request: EngineRequest,
}

/// Immediate result of submission; it does not report request execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmissionReceipt {
    pub id: RequestId,
    pub status: SubmissionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionStatus {
    Queued,
}

#[derive(Debug)]
pub enum RequestResult {
    Command(Box<CommandOutcome>),
    Interaction(InteractionOutcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionOutcome {
    PointCaptured,
    CursorMoved,
    Confirmed,
    Cancelled,
    SelectionUpdated,
    PointerObserved,
    ScrollObserved,
}

/// Semantic refusal/failure. Acceptance is not a claim that work executed.
#[derive(Debug)]
pub enum RequestError {
    Cancelled,
    QueueFull,
    Busy,
    StaleContext,
    Execution(CadError),
}

#[derive(Debug)]
pub struct RequestCompletion {
    pub id: RequestId,
    pub result: Result<RequestResult, RequestError>,
}

/// Minimal semantic update event; presentation and localization belong to hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostUpdate {
    Status(StatusMessage),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusMessage {
    Ready,
    Busy,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::{DocumentId, ViewportId};

    #[test]
    fn synthetic_envelope_keeps_request_identity_and_context_typed() {
        let envelope = RequestEnvelope {
            id: RequestId(42),
            context: RequestContext {
                document: DocumentId(1),
                document_generation: 4,
                viewport: ViewportId(2),
                tool_generation: Some(9),
                metrics_revision: Some(3),
            },
            request: EngineRequest::Interaction(InteractionRequest::BeginDraw(
                cad_app::DrawToolKind::Line,
            )),
        };
        assert_eq!(envelope.id, RequestId(42));
        assert_eq!(envelope.context.document_generation, 4);
        assert!(matches!(envelope.request, EngineRequest::Interaction(_)));
    }

    #[test]
    fn synthetic_submission_receipt_is_distinct_from_completion() {
        let receipt = SubmissionReceipt {
            id: RequestId(5),
            status: SubmissionStatus::Queued,
        };
        let completion = RequestCompletion {
            id: RequestId(5),
            result: Err(RequestError::Execution(cad_domain::CadError::InvalidInput(
                "synthetic cause".into(),
            ))),
        };
        assert_eq!(receipt.id, completion.id);
        assert_eq!(receipt.status, SubmissionStatus::Queued);
        assert!(matches!(
            completion.result,
            Err(RequestError::Execution(cad_domain::CadError::InvalidInput(cause)))
                if cause == "synthetic cause"
        ));
    }
}
