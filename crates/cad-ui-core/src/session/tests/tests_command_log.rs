//! Typed command echo (`status.commandLog`): admission, results, refusals,
//! the document-scoped clear and the wire shape.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

/// Dispatch one interaction through the real synchronous host boundary.
fn interact(session: &CadSession, request: InteractionRequest) {
    let client: &dyn EngineClient = &session.core;
    client
        .dispatch_with_origin(
            cad_engine::protocol::RequestOrigin::Ui,
            EngineRequest::Interaction(request),
        )
        .expect("the interaction executes");
}

fn world(x: f64, y: f64) -> PointInput {
    PointInput::World(Point3 { x, y, z: 0.0 })
}

#[test]
fn a_distance_measurement_echoes_its_command_and_typed_result() {
    let session = fresh_session();
    // The demo drawing's geometry would snap the synthetic picks; the echo test
    // asserts the exact picked coordinates, so object snap is off.
    session
        .core
        .controller
        .borrow_mut()
        .session
        .set_snap_enabled(false);
    session.command("DIST").expect("DIST is admitted");
    interact(&session, InteractionRequest::CapturePoint(world(1.0, 2.0)));
    interact(&session, InteractionRequest::CapturePoint(world(4.0, 6.0)));

    let status = session.snapshot(None, None, 0);
    assert_eq!(status.command_log.len(), 2, "one command, one result");
    assert_eq!(
        status
            .command_log
            .iter()
            .map(|e| e.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2],
        "sequences are monotonic"
    );

    let command = &status.command_log[0];
    assert_eq!(command.kind, CadCommandLogKind::Command);
    assert_eq!(command.command.as_deref(), Some("DIST"));
    assert_eq!(command.code, None);
    assert_eq!(command.measurement, None);

    let result = &status.command_log[1];
    assert_eq!(result.kind, CadCommandLogKind::Result);
    assert_eq!(result.command, None, "a result names no selector");
    assert_eq!(result.code, None);
    let echo = result
        .measurement
        .as_ref()
        .expect("the result carries the echo");
    assert_eq!(echo.algorithm, "distance");
    assert_eq!(echo.value, 5.0);
    assert_eq!(
        echo.points,
        vec![[1.0, 2.0, 0.0], [4.0, 6.0, 0.0]],
        "both picks travel so the UI can format the deltas"
    );
    // The demo document has no unit assignment: drawing units stay `null`
    // instead of being fabricated as millimetres.
    assert_eq!(echo.unit, None);
    assert_eq!(echo.precision.as_deref(), Some("analytic"));
}

#[test]
fn a_refused_command_produces_an_error_entry_with_only_its_code() {
    // The synthetic platform advertises no save capability, so SAVE is a real
    // host refusal: the router admits the selector, then the host refuses it.
    let session = fresh_session();
    assert!(session.command("SAVE").is_err());

    let status = session.snapshot(None, None, 0);
    assert_eq!(status.command_log.len(), 2);
    assert_eq!(status.command_log[0].command.as_deref(), Some("SAVE"));
    let error = &status.command_log[1];
    assert_eq!(error.kind, CadCommandLogKind::Error);
    assert_eq!(error.code.as_deref(), Some("unsupported"));
    assert_eq!(error.command, None);
    assert_eq!(error.measurement, None);
    // The localized refusal reason was never copied: the entry type has no
    // text field at all (compile-time guarantee, not an assertion on content).
}

#[test]
fn the_command_log_clears_on_a_verified_document_replacement() {
    let session = fresh_session();
    session.command("DIST").expect("DIST is admitted");
    assert_eq!(session.snapshot(None, None, 0).command_log.len(), 1);

    // NEW replaces the authoritative document; the host clears the echo at that
    // verified transition, so the NEW selector itself does not survive either.
    session.command("NEW").expect("NEW is admitted");
    assert!(
        session.snapshot(None, None, 0).command_log.is_empty(),
        "no entry from the previous drawing may survive a replacement"
    );

    // The sequence counter is not reset by the clear: the next entry continues
    // after the previous drawing's sequence.
    session.command("DIST").expect("DIST is admitted");
    let entries = session.snapshot(None, None, 0).command_log;
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].sequence, 3,
        "sequences stay monotonic across a clear"
    );
}

#[test]
fn the_command_log_wire_shape_has_no_message_or_text_field() {
    let entry = CadCommandLogEntry {
        sequence: 9,
        kind: CadCommandLogKind::Result,
        command: None,
        code: None,
        measurement: Some(CadMeasurementEcho {
            algorithm: "distance".into(),
            value: 5.0,
            unit: None,
            precision: Some("analytic".into()),
            points: vec![[0.0, 0.0, 0.0], [3.0, 4.0, 0.0]],
        }),
    };
    assert_eq!(
        super::tests_text::wire_field_names(&entry),
        vec!["sequence", "kind", "command", "code", "measurement"],
        "no message/text field exists on the wire"
    );

    let status = fresh_session().snapshot(None, None, 0);
    assert!(
        super::tests_text::wire_field_names(&status)
            .iter()
            .any(|name| name == "commandLog"),
        "the status field is the camelCase `commandLog`"
    );
}
