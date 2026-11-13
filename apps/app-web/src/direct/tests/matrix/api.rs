//! The **mutating** transport on both carriers: submit/poll and the commit round
//! trip, compared through the same harness the read matrix uses.

use super::*;

/// The object-read cases of the equivalence matrix.
///
/// They live in this module (rather than `matrix.rs`) so that file stays under the
/// physical-line gate; `every_wire_command_answers_identically_on_both_paths`
/// chains them into its own run.
pub(super) const OBJECT_READ_CASES: &[(&str, &str, &str, (bool, bool))] = &[
    (
        "api-submit/get-object-id",
        "cad_api_submit",
        r#"{"requestId":"matrix-gid","namespace":"DatabaseServices","operation":"database.getObjectId","parameters":{"database":"1","handle":"2F"}}"#,
        LIVE,
    ),
    (
        "api-submit/get-object",
        "cad_api_submit",
        r#"{"requestId":"matrix-go","namespace":"DatabaseServices","operation":"transaction.getObject","parameters":{"document":"1","transactionId":"matrix-tx","objectId":"1","mode":"read"}}"#,
        LIVE,
    ),
    (
        "api-submit/get-object-bad-handle",
        "cad_api_submit",
        r#"{"requestId":"matrix-gid-bad","namespace":"DatabaseServices","operation":"database.getObjectId","parameters":{"database":"1","handle":"ZZ"}}"#,
        LIVE,
    ),
    // The synchronous read lane refuses both live-session reads.
    (
        "api-call/get-object-is-not-a-read",
        "cad_api_call",
        r#"{"namespace":"DatabaseServices","operation":"transaction.getObject"}"#,
        LIVE,
    ),
    (
        "api-call/get-object-id-is-not-a-read",
        "cad_api_call",
        r#"{"namespace":"DatabaseServices","operation":"database.getObjectId"}"#,
        LIVE,
    ),
];

/// A commit round trip: queued and answered on the desktop's owner tick, applied
/// inline on `dom-canvas` — the two paths poll the identical outcome.
#[test]
fn a_commit_round_trip_is_the_same_on_both_paths() {
    let desktop_session = session();
    let direct_session = session();
    let desktop_snapshot = desktop_session.api_snapshot();
    let direct_snapshot = direct_session.api_snapshot();
    let document = desktop_snapshot
        .document
        .as_ref()
        .expect("the fixture session holds a document");
    let peer = direct_snapshot
        .document
        .as_ref()
        .expect("the peer session holds the same document");
    assert_eq!(
        peer.database_version, document.database_version,
        "both fixtures start from the same revision"
    );
    let revision: u64 = document
        .database_version
        .parse()
        .expect("a decimal revision");
    let parameters = serde_json::json!({
        "document": document.id,
        "transactionId": "matrix-tx",
        "expectedRevision": revision,
        "edits": [{
            "typeName": "Line",
            "layer": "0",
            "space": "model",
            "parameters": { "start": [0.0, 0.0, 0.0], "end": [5.0, 2.0, 0.0] },
        }],
    });
    let submit_args = serde_json::json!({
        "requestId": "matrix-commit",
        "namespace": "DatabaseServices",
        "operation": "transaction.commit",
        "parameters": parameters,
    })
    .to_string();

    // Desktop: admit, apply on the simulated owner tick, poll.
    let mut queues = Queues::new(true, false);
    let desktop_reads = QueueReads::new(None, None, None, Some(&desktop_snapshot));
    let mut popups = NoPopups;
    let ack = {
        let mut sink = queues.sink();
        dispatch(
            "cad_api_submit",
            &submit_args,
            &mut sink,
            &desktop_reads,
            &mut popups,
        )
        .expect("the submission is admitted")
    };
    assert_eq!(ack, serde_json::json!({ "accepted": true }));
    queues
        .drain(&desktop_session)
        .expect("the owner tick applies the commit");
    let desktop_poll = {
        let mut sink = queues.sink();
        dispatch("cad_api_poll", "{}", &mut sink, &desktop_reads, &mut popups)
            .expect("the poll answer")
    };

    // `dom-canvas`: applied inline, answered by the poll on the same session.
    let direct_reads = DirectReads::new(&direct_session);
    let mut direct_sink = DirectSink::with_gates(&direct_session, true, false);
    dispatch(
        "cad_api_submit",
        &submit_args,
        &mut direct_sink,
        &direct_reads,
        &mut popups,
    )
    .expect("the submission is admitted");
    let direct_poll = dispatch(
        "cad_api_poll",
        "{}",
        &mut direct_sink,
        &direct_reads,
        &mut popups,
    )
    .expect("the poll answer");

    assert_eq!(desktop_poll, direct_poll);
    let results = desktop_poll["results"].as_array().expect("a result list");
    assert_eq!(results.len(), 1, "one submission, one answer");
    let outcome = &results[0];
    assert_eq!(outcome["requestId"], serde_json::json!("matrix-commit"));
    assert_eq!(outcome["ok"], serde_json::json!(true));
    assert_eq!(
        outcome["value"]["created"].as_array().map(Vec::len),
        Some(1),
        "one insert created one entity"
    );
    assert_eq!(
        outcome["value"]["revision"],
        serde_json::json!(revision + 1),
        "one commit, one revision"
    );
    // The authoritative session really moved (the commit went through
    // `execute_api`, not a fabricated answer).
    let after: u64 = desktop_session
        .api_snapshot()
        .document
        .expect("a document")
        .database_version
        .parse()
        .expect("a decimal revision");
    assert_eq!(after, revision + 1);
}

/// The desktop's submit queue refuses a still-open id and a full queue; a
/// synchronous carrier never has an open id (it answers inline), so these two
/// verdicts exist only on the queued path.
#[test]
fn the_desktop_submit_queue_refuses_a_duplicate_and_a_full_queue() {
    let session = session();
    let snapshot = session.api_snapshot();
    let reads = QueueReads::new(None, None, None, Some(&snapshot));
    let mut queues = Queues::new(true, false);
    let mut popups = NoPopups;
    let args = |request_id: &str| {
        serde_json::json!({
            "requestId": request_id,
            "namespace": "DatabaseServices",
            "operation": "transaction.commit",
        })
        .to_string()
    };

    let first = args("dup-1");
    {
        let mut sink = queues.sink();
        dispatch("cad_api_submit", &first, &mut sink, &reads, &mut popups)
            .expect("the first submission is admitted");
    }
    {
        let mut sink = queues.sink();
        assert_eq!(
            dispatch("cad_api_submit", &first, &mut sink, &reads, &mut popups).unwrap_err(),
            cad_ui_core::session::API_SUBMIT_DUPLICATE
        );
    }
    for index in 1..cad_ui_core::session::CAD_API_SUBMIT_CAP {
        let mut sink = queues.sink();
        dispatch(
            "cad_api_submit",
            &args(&format!("dup-{}", index + 1)),
            &mut sink,
            &reads,
            &mut popups,
        )
        .expect("admitted below the cap");
    }
    let mut sink = queues.sink();
    assert_eq!(
        dispatch(
            "cad_api_submit",
            &args("one-more"),
            &mut sink,
            &reads,
            &mut popups
        )
        .unwrap_err(),
        cad_ui_core::session::API_SUBMIT_QUEUE_FULL
    );
    assert_eq!(
        queues.api_calls.len(),
        cad_ui_core::session::CAD_API_SUBMIT_CAP
    );
}

/// The wire args of one namespaced submission.
fn submission_args(
    request_id: &str,
    namespace: &str,
    operation: &str,
    parameters: &serde_json::Value,
) -> String {
    serde_json::json!({
        "requestId": request_id,
        "namespace": namespace,
        "operation": operation,
        "parameters": parameters,
    })
    .to_string()
}

/// Submit one request on the desktop queues, apply it like the owner tick, poll.
fn desktop_submit_poll(
    queues: &mut Queues,
    session: &CadSession,
    reads: &QueueReads<'_>,
    request_id: &str,
    namespace: &str,
    operation: &str,
    parameters: &serde_json::Value,
) -> Value {
    let mut popups = NoPopups;
    let args = submission_args(request_id, namespace, operation, parameters);
    let ack = {
        let mut sink = queues.sink();
        dispatch("cad_api_submit", &args, &mut sink, reads, &mut popups)
            .expect("the submission is admitted")
    };
    assert_eq!(ack, serde_json::json!({ "accepted": true }));
    queues
        .drain(session)
        .expect("the owner tick applies the request");
    let poll = {
        let mut sink = queues.sink();
        dispatch("cad_api_poll", "{}", &mut sink, reads, &mut popups).expect("the poll answer")
    };
    let results = poll["results"].as_array().expect("a result list");
    assert_eq!(results.len(), 1, "one submission, one answer");
    results[0].clone()
}

/// Submit one request on `dom-canvas` (applied inline) and poll it.
fn direct_submit_poll(
    session: &CadSession,
    request_id: &str,
    namespace: &str,
    operation: &str,
    parameters: &serde_json::Value,
) -> Value {
    let reads = DirectReads::new(session);
    let mut sink = DirectSink::with_gates(session, true, false);
    let mut popups = NoPopups;
    let args = submission_args(request_id, namespace, operation, parameters);
    dispatch("cad_api_submit", &args, &mut sink, &reads, &mut popups)
        .expect("the submission is admitted");
    let poll =
        dispatch("cad_api_poll", "{}", &mut sink, &reads, &mut popups).expect("the poll answer");
    let results = poll["results"].as_array().expect("a result list");
    assert_eq!(results.len(), 1, "one submission, one answer");
    results[0].clone()
}

/// Create a Line through the commit lane, then read it back: `database.getObjectId`
/// (a real `null` for an unknown handle) and `transaction.getObject` (the client
/// object) answer identically on both carriers.
#[test]
fn a_create_then_read_round_trip_is_the_same_on_both_paths() {
    let desktop_session = session();
    let direct_session = session();
    let desktop_snapshot = desktop_session.api_snapshot();
    let direct_snapshot = direct_session.api_snapshot();
    let document = desktop_snapshot
        .document
        .as_ref()
        .expect("the fixture session holds a document");
    assert_eq!(
        direct_snapshot
            .document
            .as_ref()
            .expect("the peer session holds a document")
            .database_version,
        document.database_version,
        "both fixtures start from the same revision"
    );
    let revision: u64 = document
        .database_version
        .parse()
        .expect("a decimal revision");
    let desktop_reads = QueueReads::new(None, None, None, Some(&desktop_snapshot));
    let mut queues = Queues::new(true, false);

    let commit_parameters = serde_json::json!({
        "document": document.id,
        "transactionId": "read-tx",
        "expectedRevision": revision,
        "edits": [{
            "typeName": "Line",
            "layer": "0",
            "space": "model",
            "parameters": { "start": [0.0, 0.0, 0.0], "end": [5.0, 2.0, 0.0] },
        }],
    });
    let desktop_commit = desktop_submit_poll(
        &mut queues,
        &desktop_session,
        &desktop_reads,
        "read-1",
        "DatabaseServices",
        "transaction.commit",
        &commit_parameters,
    );
    let direct_commit = direct_submit_poll(
        &direct_session,
        "read-1",
        "DatabaseServices",
        "transaction.commit",
        &commit_parameters,
    );
    assert_eq!(desktop_commit, direct_commit);
    assert_eq!(desktop_commit["ok"], serde_json::json!(true));
    let created = desktop_commit["value"]["created"][0]
        .as_str()
        .expect("a decimal id")
        .to_string();

    // A handle that cannot exist: the honest `null`, not an error.
    let handle_parameters = serde_json::json!({
        "database": document.database_id,
        "handle": "FFFFFFFFFF",
    });
    let desktop_handle = desktop_submit_poll(
        &mut queues,
        &desktop_session,
        &desktop_reads,
        "read-2",
        "DatabaseServices",
        "database.getObjectId",
        &handle_parameters,
    );
    let direct_handle = direct_submit_poll(
        &direct_session,
        "read-2",
        "DatabaseServices",
        "database.getObjectId",
        &handle_parameters,
    );
    assert_eq!(desktop_handle, direct_handle);
    assert_eq!(desktop_handle["ok"], serde_json::json!(true));
    assert_eq!(
        desktop_handle["value"],
        serde_json::json!({ "objectId": null })
    );

    // The created entity reads back as the client's `decodeObject` shape.
    let object_parameters = serde_json::json!({
        "document": document.id,
        "transactionId": "read-tx-2",
        "objectId": created,
        "mode": "write",
    });
    let desktop_object = desktop_submit_poll(
        &mut queues,
        &desktop_session,
        &desktop_reads,
        "read-3",
        "DatabaseServices",
        "transaction.getObject",
        &object_parameters,
    );
    let direct_object = direct_submit_poll(
        &direct_session,
        "read-3",
        "DatabaseServices",
        "transaction.getObject",
        &object_parameters,
    );
    assert_eq!(desktop_object, direct_object);
    assert_eq!(desktop_object["ok"], serde_json::json!(true));
    let object = &desktop_object["value"]["object"];
    assert_eq!(object["typeName"], serde_json::json!("Line"));
    assert_eq!(object["objectId"], serde_json::json!(created));
    assert_eq!(
        object["handle"],
        Value::Null,
        "a created entity has no handle"
    );
    assert_eq!(object["layer"], serde_json::json!("0"));
    assert_eq!(object["startPoint"], serde_json::json!([0.0, 0.0, 0.0]));
    assert_eq!(object["endPoint"], serde_json::json!([5.0, 2.0, 0.0]));
    assert_eq!(object["length"], serde_json::json!((29.0_f64).sqrt()));
    assert_eq!(object["color"], serde_json::json!({ "method": "byLayer" }));
    assert_eq!(
        object["linetype"],
        serde_json::json!({ "method": "byLayer" })
    );
}

/// One poll answer on the desktop queues (the simulated owner tick).
fn desktop_poll(queues: &mut Queues, reads: &QueueReads<'_>) -> Value {
    let mut popups = NoPopups;
    let mut sink = queues.sink();
    dispatch("cad_api_poll", "{}", &mut sink, reads, &mut popups).expect("the poll answer")
}

/// `editor.getString` starts a prompt with no answer, is answered by the ordinary
/// typed-text funnel, and the answer is identical on both carriers.
#[test]
fn an_editor_prompt_is_answered_on_both_paths() {
    let desktop_session = session();
    let direct_session = session();
    let desktop_snapshot = desktop_session.api_snapshot();
    let reads = QueueReads::new(None, None, None, Some(&desktop_snapshot));
    let mut queues = Queues::new(true, false);
    let mut popups = NoPopups;
    let args = submission_args(
        "prompt-1",
        "EditorInput",
        "editor.getString",
        &serde_json::json!({ "message": "Name:" }),
    );

    // Desktop: admitted, applied on the owner tick, no answer yet.
    {
        let mut sink = queues.sink();
        dispatch("cad_api_submit", &args, &mut sink, &reads, &mut popups)
            .expect("the prompt submission is admitted");
    }
    queues
        .drain(&desktop_session)
        .expect("the owner tick starts the prompt");
    assert_eq!(
        desktop_poll(&mut queues, &reads)["results"],
        serde_json::json!([]),
        "a started prompt has no answer yet"
    );
    let pending = desktop_session
        .snapshot(None, None, 0)
        .pending_prompt
        .expect("the prompt is pending on the desktop session");
    assert_eq!(pending.message, "Name:");

    // The same submission on `dom-canvas`: applied inline, no answer yet.
    {
        let direct_reads = DirectReads::new(&direct_session);
        let mut direct_sink = DirectSink::with_gates(&direct_session, true, false);
        dispatch(
            "cad_api_submit",
            &args,
            &mut direct_sink,
            &direct_reads,
            &mut popups,
        )
        .expect("the direct prompt submission is admitted");
        let direct_poll = dispatch(
            "cad_api_poll",
            "{}",
            &mut direct_sink,
            &direct_reads,
            &mut popups,
        )
        .expect("the direct poll answer");
        assert_eq!(
            direct_poll["results"],
            serde_json::json!([]),
            "a started prompt has no answer yet"
        );
    }
    assert!(
        direct_session.pending_prompt().is_some(),
        "the direct prompt is pending"
    );

    // The user types at the prompt: the ordinary text funnel answers it.
    desktop_session
        .set_tool_text("hello")
        .expect("the typed text is admitted");
    direct_session
        .set_tool_text("hello")
        .expect("the typed text is admitted");

    queues
        .drain(&desktop_session)
        .expect("the owner tick publishes the prompt answer");
    let desktop_answer = desktop_poll(&mut queues, &reads);
    let direct_answer = {
        let direct_reads = DirectReads::new(&direct_session);
        let mut direct_sink = DirectSink::with_gates(&direct_session, true, false);
        dispatch(
            "cad_api_poll",
            "{}",
            &mut direct_sink,
            &direct_reads,
            &mut popups,
        )
        .expect("the direct poll answer")
    };
    assert_eq!(desktop_answer, direct_answer);
    let results = desktop_answer["results"].as_array().expect("a result list");
    assert_eq!(results.len(), 1, "one prompt, one answer");
    assert_eq!(results[0]["requestId"], serde_json::json!("prompt-1"));
    assert_eq!(
        results[0]["value"],
        serde_json::json!({ "status": "Ok", "value": "hello" })
    );
    assert!(
        desktop_session
            .snapshot(None, None, 0)
            .pending_prompt
            .is_none(),
        "the prompt is settled"
    );
}

/// Esc cancels the prompt on both carriers, and a later poll reports it.
#[test]
fn an_escaped_editor_prompt_answers_cancel_on_both_paths() {
    let desktop_session = session();
    let direct_session = session();
    let desktop_snapshot = desktop_session.api_snapshot();
    let reads = QueueReads::new(None, None, None, Some(&desktop_snapshot));
    let mut queues = Queues::new(true, false);
    let mut popups = NoPopups;
    let args = submission_args(
        "prompt-2",
        "EditorInput",
        "editor.getString",
        &serde_json::json!({}),
    );
    {
        let mut sink = queues.sink();
        dispatch("cad_api_submit", &args, &mut sink, &reads, &mut popups)
            .expect("the prompt submission is admitted");
    }
    {
        let direct_reads = DirectReads::new(&direct_session);
        let mut direct_sink = DirectSink::with_gates(&direct_session, true, false);
        dispatch(
            "cad_api_submit",
            &args,
            &mut direct_sink,
            &direct_reads,
            &mut popups,
        )
        .expect("the direct submission is admitted");
    }
    queues
        .drain(&desktop_session)
        .expect("the owner tick starts the desktop prompt");

    // Esc (the command the shell sends) is consumed by the prompt.
    desktop_session
        .command("CANCEL")
        .expect("the desktop cancel is admitted");
    direct_session
        .command("CANCEL")
        .expect("the direct cancel is admitted");

    queues
        .drain(&desktop_session)
        .expect("the owner tick publishes the cancel");
    let desktop_answer = desktop_poll(&mut queues, &reads);
    let direct_answer = {
        let direct_reads = DirectReads::new(&direct_session);
        let mut direct_sink = DirectSink::with_gates(&direct_session, true, false);
        dispatch(
            "cad_api_poll",
            "{}",
            &mut direct_sink,
            &direct_reads,
            &mut popups,
        )
        .expect("the direct poll answer")
    };
    assert_eq!(desktop_answer, direct_answer);
    assert_eq!(
        desktop_answer["results"][0]["value"],
        serde_json::json!({ "status": "Cancel" })
    );
}

/// `editor.selectAll` is immediate: one submission, one answer in the same poll.
#[test]
fn select_all_answers_immediately_on_both_paths() {
    let desktop_session = session();
    let direct_session = session();
    let desktop_snapshot = desktop_session.api_snapshot();
    let reads = QueueReads::new(None, None, None, Some(&desktop_snapshot));
    let mut queues = Queues::new(true, false);
    let parameters = serde_json::json!({});
    let desktop_answer = desktop_submit_poll(
        &mut queues,
        &desktop_session,
        &reads,
        "all-1",
        "EditorInput",
        "editor.selectAll",
        &parameters,
    );
    let direct_answer = direct_submit_poll(
        &direct_session,
        "all-1",
        "EditorInput",
        "editor.selectAll",
        &parameters,
    );
    assert_eq!(desktop_answer, direct_answer);
    assert_eq!(
        desktop_answer["ok"],
        serde_json::json!(true),
        "selectAll answer: {desktop_answer}"
    );
    let ids = desktop_answer["value"]["objectIds"]
        .as_array()
        .expect("an id list");
    assert!(!ids.is_empty(), "the fixture has selectable geometry");
    assert!(ids.iter().all(|id| id.as_str().is_some()));
}
