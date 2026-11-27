//! The Protractor on the real host interaction path.
//!
//! `cad-app` pins the vertex-first capture order and the angle itself; this file
//! pins the wiring: the measurement request reaches the session, three world
//! captures auto-complete it, and the result is recorded with the vertex in the
//! engine's second slot while the document is untouched. No GUI or GPU is
//! involved.
use super::*;

fn revision(core: &HostCore) -> cad_domain::Revision {
    core.controller.borrow().drawing().unwrap().revision()
}

fn capture(core: &HostCore, world: Point3) {
    EngineClient::dispatch(
        core,
        EngineRequest::Interaction(InteractionRequest::CapturePoint(PointInput::World(world))),
    )
    .expect("the capture is admitted");
}

#[test]
fn the_protractor_measures_a_vertex_first_right_angle_without_writing() {
    let core = core();
    core.controller.borrow_mut().set_snap_enabled(false);
    core.command(
        CommandId::Measure,
        CommandPayload::MeasureTool(cad_app::MeasurementToolKind::Protractor),
    )
    .expect("the protractor tool starts");
    assert_eq!(
        core.controller
            .borrow()
            .session
            .measurement_preview()
            .map(|p| p.kind),
        Some(cad_app::MeasurementToolKind::Protractor)
    );
    let before = revision(&core);

    // SketchUp's order: the vertex, then one point on each ray.
    capture(&core, p(0.0, 0.0));
    capture(&core, p(10.0, 0.0));
    capture(&core, p(0.0, 10.0));

    {
        let c = core.controller.borrow();
        let record = c.session.last_measurement().expect("a measurement record");
        assert_eq!(record.algorithm, cad_db::MeasurementAlgorithm::Angle3Points);
        assert!(
            (record.value - 90.0).abs() < 1e-9,
            "a right angle at the vertex: {}",
            record.value
        );
        // The engine measures at its second point, so the adapted record holds
        // the vertex there; the capture order is only the display order.
        assert_eq!(record.inputs[1], p(0.0, 0.0));
        assert!(
            c.session.measurement_preview().is_none(),
            "one-shot tool ends"
        );
        assert_eq!(c.drawing().unwrap().revision(), before, "read-only");
    }
    assert!(
        core.controller.borrow().session.has_last_measurement(),
        "the record is retained for the measurement panel"
    );
}
