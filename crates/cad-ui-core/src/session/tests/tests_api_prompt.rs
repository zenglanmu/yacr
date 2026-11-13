//! The plugin `Editor` prompt lane: parameter parsing, routing, answers and the
//! `pendingPrompt` status projection.

use super::super::*;
use super::test_support::*;

use cad_app::plugin_prompt::{ApiPromptInput, ApiPromptReply};

fn fresh() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

/// One `EditorInput` submission through the submit lane.
fn submit(
    session: &CadSession,
    request_id: &str,
    operation: &str,
    parameters: serde_json::Value,
) -> Option<CadApiOutcome> {
    session.submit_api_call(CadApiSubmission {
        request_id: request_id.to_string(),
        namespace: "EditorInput".to_string(),
        operation: operation.to_string(),
        parameters: Some(parameters),
    })
}

/// Resolve the pending prompt through the controller (what the input funnel does).
fn resolve(session: &CadSession, input: ApiPromptInput) {
    session
        .core
        .controller
        .borrow_mut()
        .resolve_api_prompt(input);
}

fn cancel(session: &CadSession) {
    session.core.controller.borrow_mut().cancel_api_prompt();
}

fn refusal(outcome: Option<CadApiOutcome>) -> String {
    let outcome = outcome.expect("a refusal is settled immediately");
    assert!(!outcome.ok, "expected a refusal, got {outcome:?}");
    assert_eq!(outcome.value, None);
    outcome.error.expect("a refusal marker")
}

#[test]
fn every_prompt_operation_starts_and_projects_its_kind() {
    let session = fresh();
    for (operation, kind) in [
        (API_CALL_GET_POINT, CadPendingPromptKind::Point),
        (API_CALL_GET_ENTITY, CadPendingPromptKind::Entity),
        (API_CALL_GET_SELECTION, CadPendingPromptKind::Selection),
        (API_CALL_GET_STRING, CadPendingPromptKind::String),
        (API_CALL_GET_DOUBLE, CadPendingPromptKind::Double),
        (API_CALL_GET_INTEGER, CadPendingPromptKind::Integer),
        (API_CALL_GET_KEYWORD, CadPendingPromptKind::Keyword),
    ] {
        let parameters = if operation == API_CALL_GET_KEYWORD {
            serde_json::json!({ "keywords": ["Yes", "No"] })
        } else {
            serde_json::json!({})
        };
        assert!(
            submit(&session, "req-1", operation, parameters).is_none(),
            "{operation} starts a prompt and answers nothing yet"
        );
        let prompt = session.pending_prompt().expect("a prompt is pending");
        assert_eq!(prompt.kind, kind, "{operation}");
        assert!(prompt.error.is_none());
        cancel(&session);
    }
}

#[test]
fn the_message_and_keywords_are_carried_verbatim() {
    let session = fresh();
    submit(
        &session,
        "req-1",
        API_CALL_GET_POINT,
        serde_json::json!({ "message": "Pick corner:", "keywords": ["Yes", "No"] }),
    );
    let prompt = session.pending_prompt().expect("a prompt is pending");
    assert_eq!(prompt.message, "Pick corner:");
    assert_eq!(prompt.keywords, vec!["Yes".to_string(), "No".to_string()]);
}

#[test]
fn an_absent_message_uses_the_kinds_own_sentence() {
    let session = fresh();
    submit(
        &session,
        "req-1",
        API_CALL_GET_SELECTION,
        serde_json::json!({}),
    );
    assert_eq!(
        session.pending_prompt().expect("pending").message,
        "Select objects:"
    );
}

#[test]
fn bad_parameters_are_refused_by_field() {
    let session = fresh();
    for (operation, parameters, expected) in [
        (
            API_CALL_GET_POINT,
            serde_json::json!({ "message": 7 }),
            "api.invalid_argument: message",
        ),
        (
            API_CALL_GET_POINT,
            serde_json::json!({ "keywords": "Yes" }),
            "api.invalid_argument: keywords",
        ),
        (
            API_CALL_GET_KEYWORD,
            serde_json::json!({}),
            "api.invalid_argument: keywords",
        ),
        (
            API_CALL_GET_KEYWORD,
            serde_json::json!({ "keywords": [] }),
            "api.invalid_argument: keywords",
        ),
        (
            API_CALL_GET_ENTITY,
            serde_json::json!({ "rejectMessage": 1 }),
            "api.invalid_argument: rejectMessage",
        ),
        (
            API_CALL_GET_STRING,
            serde_json::json!({ "default": 3 }),
            "api.invalid_argument: default",
        ),
        (
            API_CALL_GET_DOUBLE,
            serde_json::json!({ "default": "one" }),
            "api.invalid_argument: default",
        ),
        (
            API_CALL_GET_INTEGER,
            serde_json::json!({ "defaultValue": 1.5 }),
            "api.invalid_argument: default",
        ),
        (
            API_CALL_GET_POINT,
            serde_json::json!([1]),
            "api.invalid_argument: parameters",
        ),
    ] {
        assert_eq!(
            refusal(submit(&session, "req-1", operation, parameters)),
            expected,
            "{operation}"
        );
        assert!(session.pending_prompt().is_none(), "nothing started");
    }
}

#[test]
fn the_clients_default_value_spelling_is_accepted() {
    let session = fresh();
    submit(
        &session,
        "req-1",
        API_CALL_GET_STRING,
        serde_json::json!({ "defaultValue": "plan A" }),
    );
    assert!(session.pending_prompt().is_some());
    resolve(&session, ApiPromptInput::Confirm);
    assert_eq!(
        session.api_prompt_results()[0].value,
        Some(serde_json::json!({ "status": "Ok", "value": "plan A" }))
    );
}

#[test]
fn a_second_prompt_is_busy_and_keeps_the_first() {
    let session = fresh();
    submit(&session, "req-1", API_CALL_GET_POINT, serde_json::json!({}));
    let outcome = submit(
        &session,
        "req-2",
        API_CALL_GET_STRING,
        serde_json::json!({}),
    )
    .expect("the refusal is settled");
    assert_eq!(outcome.request_id.as_deref(), Some("req-2"));
    assert_eq!(outcome.error.as_deref(), Some("api.prompt.busy"));
    assert_eq!(
        session.pending_prompt().map(|prompt| prompt.kind),
        Some(CadPendingPromptKind::Point),
        "the first prompt is untouched"
    );
}

#[test]
fn unknown_editor_operations_are_refused_by_name() {
    let session = fresh();
    assert_eq!(
        refusal(submit(
            &session,
            "req-1",
            "editor.writeMessage",
            serde_json::json!({})
        )),
        "unsupported-runtime: cad_api.EditorInput.editor.writeMessage"
    );
}

#[test]
fn a_prompt_without_a_document_is_named() {
    let session = fresh();
    session
        .command("CLOSE")
        .expect("closing the demo document is a host command");
    assert_eq!(
        refusal(submit(
            &session,
            "req-1",
            API_CALL_GET_POINT,
            serde_json::json!({})
        )),
        API_CALL_NO_DOCUMENT
    );
    assert_eq!(
        refusal(submit(
            &session,
            "req-2",
            API_CALL_SELECT_ALL,
            serde_json::json!({})
        )),
        API_CALL_NO_DOCUMENT
    );
}

#[test]
fn select_all_answers_the_published_selection_immediately() {
    let session = fresh();
    let outcome = submit(
        &session,
        "req-1",
        API_CALL_SELECT_ALL,
        serde_json::json!({}),
    )
    .expect("selectAll is immediate");
    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(outcome.request_id.as_deref(), Some("req-1"));
    let object_ids = outcome.value.expect("a value")["objectIds"]
        .as_array()
        .expect("an id list")
        .clone();
    assert!(!object_ids.is_empty(), "the demo document has geometry");
    assert!(
        object_ids.iter().all(|id| id.as_str().is_some()),
        "the ids are decimal strings"
    );
    assert!(session.pending_prompt().is_none(), "no prompt was opened");
}

#[test]
fn every_answer_matches_the_clients_prompt_shape() {
    let session = fresh();
    // Point.
    submit(&session, "p", API_CALL_GET_POINT, serde_json::json!({}));
    resolve(
        &session,
        ApiPromptInput::Point(cad_domain::Point3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        }),
    );
    // Entity.
    submit(&session, "e", API_CALL_GET_ENTITY, serde_json::json!({}));
    resolve(&session, ApiPromptInput::Entity(cad_domain::EntityId(7)));
    // Selection.
    submit(&session, "s", API_CALL_GET_SELECTION, serde_json::json!({}));
    resolve(&session, ApiPromptInput::Confirm);
    // String.
    submit(&session, "str", API_CALL_GET_STRING, serde_json::json!({}));
    resolve(&session, ApiPromptInput::Text("hello".to_string()));
    // Integer.
    submit(&session, "i", API_CALL_GET_INTEGER, serde_json::json!({}));
    resolve(&session, ApiPromptInput::Text("12".to_string()));
    // Keyword.
    submit(
        &session,
        "k",
        API_CALL_GET_KEYWORD,
        serde_json::json!({ "keywords": ["Yes", "No"] }),
    );
    resolve(&session, ApiPromptInput::Text("yes".to_string()));
    // None (Enter with no default).
    submit(&session, "n", API_CALL_GET_DOUBLE, serde_json::json!({}));
    resolve(&session, ApiPromptInput::Confirm);
    // Cancel.
    submit(&session, "c", API_CALL_GET_STRING, serde_json::json!({}));
    resolve(&session, ApiPromptInput::Cancel);

    let outcomes = session.api_prompt_results();
    let by_id = |request_id: &str| {
        outcomes
            .iter()
            .find(|outcome| outcome.request_id.as_deref() == Some(request_id))
            .unwrap_or_else(|| panic!("no answer for {request_id}"))
            .clone()
    };
    assert_eq!(
        by_id("p").value,
        Some(serde_json::json!({
            "status": "Ok",
            "point": { "x": 1.0, "y": 2.0, "z": 3.0 },
        }))
    );
    assert_eq!(
        by_id("e").value,
        Some(serde_json::json!({ "status": "Ok", "objectId": "7" }))
    );
    let selection = by_id("s");
    assert_eq!(selection.value.expect("a value")["status"], "Ok");
    assert_eq!(
        by_id("str").value,
        Some(serde_json::json!({ "status": "Ok", "value": "hello" }))
    );
    assert_eq!(
        by_id("i").value,
        Some(serde_json::json!({ "status": "Ok", "value": 12.0 }))
    );
    assert_eq!(
        by_id("k").value,
        Some(serde_json::json!({ "status": "Keyword", "keyword": "Yes" }))
    );
    assert_eq!(
        by_id("n").value,
        Some(serde_json::json!({ "status": "None" }))
    );
    assert_eq!(
        by_id("c").value,
        Some(serde_json::json!({ "status": "Cancel" }))
    );
    assert!(
        outcomes
            .iter()
            .all(|outcome| !outcome.ok || outcome.request_id.is_some()),
        "every answer is correlatable"
    );
}

#[test]
fn the_status_projection_carries_the_pending_prompt_and_its_error() {
    let session = fresh();
    submit(
        &session,
        "req-1",
        API_CALL_GET_DOUBLE,
        serde_json::json!({ "message": "Height:" }),
    );
    let status = serde_json::to_value(session.snapshot(None, None, 0)).expect("serializes");
    assert_eq!(
        status["pendingPrompt"],
        serde_json::json!({
            "kind": "double",
            "message": "Height:",
            "keywords": [],
            "error": null,
        })
    );

    resolve(&session, ApiPromptInput::Text("tall".to_string()));
    let status = serde_json::to_value(session.snapshot(None, None, 0)).expect("serializes");
    assert_eq!(
        status["pendingPrompt"]["error"],
        "invalid input: enter a number"
    );
    assert_eq!(status["pendingPrompt"]["kind"], "double");
}

#[test]
fn the_synchronous_read_lane_does_not_serve_prompts() {
    let session = fresh();
    let snapshot = session.api_snapshot();
    for operation in [API_CALL_GET_POINT, API_CALL_SELECT_ALL] {
        let outcome = dispatch_api_call(
            Some(&snapshot),
            "EditorInput",
            operation,
            Some(&serde_json::json!({})),
        );
        assert_eq!(
            outcome.error.as_deref(),
            Some(format!("unsupported-runtime: cad_api.EditorInput.{operation}").as_str()),
            "{operation} is a live-session prompt, not a snapshot read"
        );
    }
}

#[test]
fn an_unmatched_keyword_text_reprompts_with_the_expected_list() {
    let session = fresh();
    submit(
        &session,
        "req-1",
        API_CALL_GET_KEYWORD,
        serde_json::json!({ "keywords": ["Yes", "No"] }),
    );
    resolve(&session, ApiPromptInput::Text("maybe".to_string()));
    assert_eq!(
        session
            .pending_prompt()
            .expect("still pending")
            .error
            .as_deref(),
        Some("invalid input: expected one of Yes, No")
    );
    assert!(session.api_prompt_results().is_empty());
    resolve(&session, ApiPromptInput::Text("no".to_string()));
    assert_eq!(
        session.api_prompt_results()[0].value,
        Some(serde_json::json!({ "status": "Keyword", "keyword": "No" }))
    );
}

/// A `PromptReply` the controller stores is a typed value; the shared layer is
/// the only place that turns it into JSON (pinned above). Here we only prove the
/// reply variants stay distinguishable.
#[test]
fn the_reply_variants_stay_distinct() {
    assert_ne!(ApiPromptReply::None, ApiPromptReply::Cancel);
}

#[test]
fn probe_text_through_session() {
    let session = fresh();
    submit(
        &session,
        "req-1",
        API_CALL_GET_STRING,
        serde_json::json!({}),
    );
    assert!(session.pending_prompt().is_some());
    match session.set_tool_text("hello") {
        Ok(()) => {}
        Err(error) => panic!("set_tool_text refused: {error}"),
    }
    let outcomes = session.api_prompt_results();
    assert_eq!(outcomes.len(), 1, "no answer: {outcomes:?}");
}
