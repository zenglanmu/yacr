//! The plugin API's **mutating** lane: submission validation, routing and the
//! one commit operation, exercised through the real session funnel.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

/// A submission with an explicit id and operation.
fn submission(
    request_id: &str,
    namespace: &str,
    operation: &str,
    parameters: Option<serde_json::Value>,
) -> CadApiSubmission {
    CadApiSubmission {
        request_id: request_id.to_string(),
        namespace: namespace.to_string(),
        operation: operation.to_string(),
        parameters,
    }
}

/// The demo session's document id, database id and revision, as the plugin sees
/// them (the request's own vocabulary).
fn document_facts(session: &CadSession) -> (String, String, u64) {
    let snapshot = session.api_snapshot();
    let document = snapshot
        .document
        .expect("the demo session holds a document");
    let revision = document
        .database_version
        .parse::<u64>()
        .expect("a decimal revision");
    (document.id, document.database_id, revision)
}

/// One `transaction.commit` parameter object against the live session state.
fn commit_parameters(
    session: &CadSession,
    expected_revision: u64,
    edits: serde_json::Value,
) -> serde_json::Value {
    let (document, _database, _revision) = document_facts(session);
    serde_json::json!({
        "document": document,
        "transactionId": "tx-test",
        "expectedRevision": expected_revision,
        "edits": edits,
    })
}

/// A Line insert on layer `0` in model space.
fn line_insert() -> serde_json::Value {
    serde_json::json!([{
        "typeName": "Line",
        "layer": "0",
        "space": "model",
        "parameters": { "start": [0.0, 0.0, 0.0], "end": [10.0, 5.0, 0.0] },
    }])
}

fn commit(session: &CadSession, request_id: &str, parameters: serde_json::Value) -> CadApiOutcome {
    session.api_submit(submission(
        request_id,
        "DatabaseServices",
        "transaction.commit",
        Some(parameters),
    ))
}

fn error_of(outcome: CadApiOutcome) -> String {
    assert!(!outcome.ok, "expected a refusal, got {outcome:?}");
    assert_eq!(outcome.value, None, "a refusal carries no value");
    outcome.error.expect("a refusal marker")
}

#[test]
fn a_bad_request_id_is_refused_by_name_and_kept_for_correlation() {
    let session = fresh_session();
    let too_long = "y".repeat(65);
    for bad in ["", "has space", "tab\there", too_long.as_str()] {
        let outcome = session.api_submit(submission(
            bad,
            "DatabaseServices",
            "transaction.commit",
            None,
        ));
        assert_eq!(
            error_of(outcome.clone()),
            API_SUBMIT_INVALID_REQUEST,
            "{bad:?}"
        );
        assert_eq!(
            outcome.request_id.as_deref(),
            Some(bad),
            "the refusal stays correlatable"
        );
    }
}

#[test]
fn a_valid_request_id_is_accepted() {
    let session = fresh_session();
    let (_document, _database, revision) = document_facts(&session);
    // Reaching the operation (and succeeding) proves the id was accepted.
    let outcome = commit(
        &session,
        "req-1.a_b-c",
        commit_parameters(&session, revision, line_insert()),
    );
    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(outcome.request_id.as_deref(), Some("req-1.a_b-c"));
}

#[test]
fn the_read_lane_is_not_a_mutation() {
    let session = fresh_session();
    // The synchronous reads are served by `cad_api_call`; submitting one is an
    // unserved operation (never routed through the commit path).
    let outcome = session.api_submit(submission(
        "req-2",
        "ApplicationServices",
        "documentManager.mdiActiveDocument",
        None,
    ));
    assert_eq!(
        error_of(outcome),
        "unsupported-runtime: cad_api.ApplicationServices.documentManager.mdiActiveDocument"
    );
}

#[test]
fn an_unknown_namespace_is_refused_before_host_state() {
    let session = fresh_session();
    let outcome = session.api_submit(submission("req-3", "Runtime", "load", None));
    assert_eq!(
        error_of(outcome),
        "unsupported-runtime: cad_api.Runtime.load"
    );
}

#[test]
fn a_commit_inserts_one_line_and_reports_the_new_revision() {
    let session = fresh_session();
    let (_document, _database, start_revision) = document_facts(&session);
    let outcome = commit(
        &session,
        "req-4",
        commit_parameters(&session, start_revision, line_insert()),
    );
    assert!(outcome.ok, "{outcome:?}");
    let value = outcome.value.expect("a value");
    let created = value["created"].as_array().expect("a created list");
    assert_eq!(created.len(), 1, "one insert created one entity");
    assert!(created[0]
        .as_str()
        .expect("a decimal id string")
        .parse::<u128>()
        .is_ok());
    // The reported revision is the session's own after the commit, not a guess.
    let (_document, _database, end_revision) = document_facts(&session);
    assert_eq!(value["revision"], serde_json::json!(end_revision));
    assert_eq!(end_revision, start_revision + 1, "one commit, one revision");
}

#[test]
fn a_translate_and_a_delete_address_the_created_entity() {
    let session = fresh_session();
    let (_document, _database, revision) = document_facts(&session);
    let inserted = commit(
        &session,
        "req-5",
        commit_parameters(&session, revision, line_insert()),
    );
    let created = inserted.value.expect("a value")["created"][0]
        .as_str()
        .expect("a decimal id")
        .to_string();
    let (_document, _database, after_insert) = document_facts(&session);

    let translated = commit(
        &session,
        "req-6",
        commit_parameters(
            &session,
            after_insert,
            serde_json::json!([{ "objectId": created, "displacement": [1.0, 0.0, 0.0] }]),
        ),
    );
    assert!(translated.ok, "{translated:?}");
    assert_eq!(
        translated.value.expect("a value")["created"],
        serde_json::json!([]),
        "a translate creates nothing"
    );

    let (_document, _database, after_translate) = document_facts(&session);
    let deleted = commit(
        &session,
        "req-7",
        commit_parameters(
            &session,
            after_translate,
            serde_json::json!([{ "objectId": created }]),
        ),
    );
    assert!(deleted.ok, "{deleted:?}");
}

#[test]
fn a_stale_expected_revision_is_the_named_conflict() {
    let session = fresh_session();
    let (_document, _database, revision) = document_facts(&session);
    let stale = commit(
        &session,
        "req-8",
        commit_parameters(&session, revision + 1, line_insert()),
    );
    assert_eq!(error_of(stale), API_CALL_REVISION_CONFLICT);

    // A repeated commit with the revision the first one consumed is stale too.
    let first = commit(
        &session,
        "req-9",
        commit_parameters(&session, revision, line_insert()),
    );
    assert!(first.ok, "{first:?}");
    let replay = commit(
        &session,
        "req-10",
        commit_parameters(&session, revision, line_insert()),
    );
    assert_eq!(error_of(replay), API_CALL_REVISION_CONFLICT);
}

#[test]
fn malformed_commit_requests_name_the_broken_field() {
    let session = fresh_session();
    let (_document, _database, revision) = document_facts(&session);
    let valid = commit_parameters(&session, revision, line_insert());

    let mut cases: Vec<(serde_json::Value, &str)> = vec![
        (serde_json::Value::Null, "api.invalid_argument: parameters"),
        (serde_json::json!([1]), "api.invalid_argument: parameters"),
    ];
    let mut without_document = valid.clone();
    without_document["document"] = serde_json::json!("999");
    cases.push((without_document, "api.invalid_argument: document"));
    let mut without_transaction = valid.clone();
    without_transaction["transactionId"] = serde_json::json!("");
    cases.push((without_transaction, "api.invalid_argument: transactionId"));
    let mut float_revision = valid.clone();
    float_revision["expectedRevision"] = serde_json::json!(1.5);
    cases.push((float_revision, "api.invalid_argument: expectedRevision"));
    let mut no_edits = valid.clone();
    no_edits["edits"] = serde_json::json!({});
    cases.push((no_edits, "api.invalid_argument: edits"));

    let edit_case = |edit: serde_json::Value| {
        let mut parameters = commit_parameters(&session, revision, serde_json::json!([]));
        parameters["edits"] = serde_json::json!([edit]);
        parameters
    };
    cases.push((
        edit_case(serde_json::json!({
            "typeName": "Line", "layer": "NOPE", "space": "model",
            "parameters": { "start": [0.0, 0.0, 0.0], "end": [1.0, 1.0, 0.0] },
        })),
        "api.invalid_argument: layer",
    ));
    cases.push((
        edit_case(serde_json::json!({
            "typeName": "Line", "layer": "0", "space": "paper",
            "parameters": { "start": [0.0, 0.0, 0.0], "end": [1.0, 1.0, 0.0] },
        })),
        "api.invalid_argument: space",
    ));
    cases.push((
        edit_case(serde_json::json!({
            "typeName": "Spline", "layer": "0", "space": "model",
            "parameters": {},
        })),
        "api.invalid_argument: typeName",
    ));
    cases.push((
        edit_case(serde_json::json!({
            "typeName": "Line", "layer": "0", "space": "model",
            "parameters": { "start": [0.0, 0.0, 0.0] },
        })),
        "api.invalid_argument: parameters.end",
    ));
    cases.push((
        edit_case(serde_json::json!({
            "typeName": "Circle", "layer": "0", "space": "model",
            "parameters": { "center": [0.0, 0.0, 0.0], "radius": "big" },
        })),
        "api.invalid_argument: parameters.radius",
    ));
    cases.push((
        edit_case(
            serde_json::json!({ "objectId": "not-a-number", "displacement": [1.0, 0.0, 0.0] }),
        ),
        "api.invalid_argument: objectId",
    ));
    cases.push((
        edit_case(serde_json::json!({ "objectId": "1", "displacement": [1.0, 0.0] })),
        "api.invalid_argument: displacement",
    ));
    cases.push((
        edit_case(serde_json::json!({ "objectId": "abc" })),
        "api.invalid_argument: objectId",
    ));
    cases.push((
        edit_case(serde_json::json!({ "unrelated": true })),
        "api.invalid_argument: edits",
    ));

    for (index, (parameters, expected)) in cases.into_iter().enumerate() {
        let outcome = commit(&session, &format!("bad-{index}"), parameters);
        assert_eq!(error_of(outcome), expected, "case {index}");
    }
    // Every case was refused before the commit path: the drawing did not move.
    let (_document, _database, after) = document_facts(&session);
    assert_eq!(after, revision, "a refused request never commits");
}

#[test]
fn the_session_result_queue_is_bounded_and_fifo() {
    let session = fresh_session();
    for index in 0..CAD_API_RESULT_CAP + 2 {
        session.push_api_outcome(
            CadApiOutcome::value(serde_json::json!({ "i": index }))
                .with_request_id(format!("r{index}")),
        );
    }
    let outcomes = session.take_api_outcomes();
    assert_eq!(outcomes.len(), CAD_API_RESULT_CAP, "bounded");
    assert_eq!(
        outcomes[0].request_id.as_deref(),
        Some("r2"),
        "the two oldest were dropped"
    );
    assert_eq!(
        outcomes[CAD_API_RESULT_CAP - 1].request_id.as_deref(),
        Some(format!("r{}", CAD_API_RESULT_CAP + 1).as_str())
    );
    assert!(session.take_api_outcomes().is_empty(), "drained");
}

#[test]
fn a_submission_without_a_session_is_cad_waiting() {
    let outcome = api_submit_or_wait(
        None,
        submission("req-11", "DatabaseServices", "transaction.commit", None),
    );
    assert!(!outcome.ok);
    assert_eq!(outcome.request_id.as_deref(), Some("req-11"));
    assert_eq!(outcome.error.as_deref(), Some(CAD_WAITING));
}
