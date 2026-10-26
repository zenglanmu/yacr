//! `AREA OBJECT` host wiring: the selection-based object measurement reaches the
//! typed command echo under its **own** algorithm key.
//!
//! `cad-app` pins the values and the refusals (`tests::area_object`); this file
//! pins the wiring no unit test on either side can see: the host dispatches the
//! command, the session keeps the measurement, and `commandLog` reports the record
//! with `algorithm: "area_object"` — not `"area"` (a picked polygon) and not
//! `"polyline"` (a length).
//!
//! These are synthetic contracts over `HostCore` (no window, no GPU): they are not
//! render/visual acceptance evidence.

use super::*;

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// A blank document with one CIRCLE of radius 2 on layer `0`.
fn fixture() -> (HostCore, SelectionRef) {
    let core = core();
    core.controller
        .borrow_mut()
        .new_blank_document()
        .expect("a blank document is installed");
    core.note_document_replaced();
    let outcome = core
        .controller
        .borrow_mut()
        .create_circle(point(0.0, 0.0, 0.0), point(2.0, 0.0, 0.0))
        .expect("the CIRCLE is created");
    let id = outcome.objects[0];
    let document = core.controller.borrow().document_id;
    let reference = cad_app::selection::model_ref(document, cad_domain::EntityId(id.0));
    core.controller
        .borrow_mut()
        .set_selection(vec![reference.clone()])
        .expect("the circle is selected");
    (core, reference)
}

#[test]
fn area_object_reaches_the_command_echo_under_its_own_algorithm_key() {
    let (core, _reference) = fixture();
    let (document, before_revision, before_undo_depth) = {
        let controller = core.controller.borrow();
        (
            controller.document_id,
            controller.drawing().expect("the blank document").revision(),
            controller
                .application
                .history
                .get(&controller.document_id)
                .map(|history| history.undo_depth())
                .unwrap_or(0),
        )
    };
    assert_eq!(
        before_undo_depth, 1,
        "the fixture's CIRCLE creation is one undo step"
    );

    core.record_command("AREA OBJECT");
    core.command(CommandId::AreaObject, CommandPayload::None)
        .expect("AREA OBJECT runs over the real selection");

    let entries = core.command_log_entries();
    assert!(
        entries
            .iter()
            .any(|entry| entry.kind == CommandLogKind::Command
                && entry.command.as_deref() == Some("AREA OBJECT")),
        "the admitted selector is echoed: {entries:?}"
    );
    let result = entries
        .iter()
        .find(|entry| entry.measurement.is_some())
        .expect("the measurement produced a Result echo");
    assert_eq!(result.kind, CommandLogKind::Result);
    assert!(
        result.command.is_none(),
        "a Result entry carries no selector (the Command entry does)"
    );
    let echo = result.measurement.as_ref().unwrap();
    assert_eq!(
        echo.algorithm, "area_object",
        "the mouse-free object area has its own key: neither 'area' nor 'polyline'"
    );
    assert!(
        (echo.value - 4.0 * std::f64::consts::PI).abs() < 1e-9,
        "{}",
        echo.value
    );
    assert!(
        echo.points.is_empty(),
        "a circle has no input vertices; the echo never fabricates them: {:?}",
        echo.points
    );

    // Measuring writes nothing: the document revision is unchanged and the undo
    // stack is exactly where the fixture's CIRCLE creation left it.
    let controller = core.controller.borrow();
    assert_eq!(
        controller.drawing().expect("the blank document").revision(),
        before_revision
    );
    assert_eq!(
        controller
            .application
            .history
            .get(&document)
            .map(|history| history.undo_depth())
            .unwrap_or(0),
        before_undo_depth,
        "a measurement records no undo entry"
    );
}
