//! The **mutating** transport on both carriers: submit/poll and the commit round
//! trip, compared through the same harness the read matrix uses.

use super::*;

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
