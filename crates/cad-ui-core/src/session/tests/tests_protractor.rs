//! The `PROTRACTOR` selector and its `GUIDE` mode.
//!
//! The bare selector is an interaction-only measurement selector: it starts
//! `BeginMeasurement(Protractor)` (vertex, then one point on each ray) and never
//! carries a `CommandId`. `PROTRACTOR GUIDE` starts the same capture with guide
//! mode, so completion writes a guide along the second ray. Contract level: no
//! GUI, no GPU.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_protractor_selector_starts_the_vertex_first_angle_measurement() {
    let session = fresh_session();
    assert!(admissible_names().contains(&"PROTRACTOR"));
    assert_eq!(canonical_command_name("PROTRACTOR"), Some("PROTRACTOR"));
    assert_eq!(
        measurement_selector_kind("PROTRACTOR"),
        Some(cad_app::MeasurementToolKind::Protractor)
    );
    // It is a measurement selector, never a `CommandId` command.
    assert_eq!(normalize_command("PROTRACTOR"), None);
    session
        .command("PROTRACTOR")
        .expect("PROTRACTOR is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("PROTRACTOR starts a measurement");
    assert_eq!(tool.key, "measure.protractor");
    assert_eq!(tool.points, 0);
    assert!(!tool.ready);
}

#[test]
fn protractor_guide_starts_the_guide_capture() {
    let session = fresh_session();
    assert_eq!(
        typed_command_argument("PROTRACTOR GUIDE"),
        Some(("PROTRACTOR", "GUIDE"))
    );
    session
        .command("PROTRACTOR GUIDE")
        .expect("guide mode is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("the protractor capture starts");
    assert_eq!(tool.key, "measure.protractor");
    assert_eq!(tool.points, 0);
}

#[test]
fn any_other_protractor_argument_is_an_unsupported_line() {
    let session = fresh_session();
    let error = session
        .command("PROTRACTOR ARC")
        .expect_err("only GUIDE is admitted");
    assert!(
        error.contains("unsupported CAD command"),
        "the grammar refusal is named: {error}"
    );
    assert!(session.snapshot(None, None, 0).tool.is_none());
}
