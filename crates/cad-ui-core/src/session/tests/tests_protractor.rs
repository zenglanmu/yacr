//! The `PROTRACTOR` selector and its `GUIDE` refusal.
//!
//! The bare selector is an interaction-only measurement selector: it starts
//! `BeginMeasurement(Protractor)` (vertex, then one point on each ray) and never
//! carries a `CommandId`. `PROTRACTOR GUIDE` is admitted only to be refused by
//! name, because this build has no guide model. Contract level: no GUI, no GPU.

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
fn protractor_guide_is_admitted_and_refused_by_name() {
    let session = fresh_session();
    assert_eq!(
        typed_command_argument("PROTRACTOR GUIDE"),
        Some(("PROTRACTOR", "GUIDE"))
    );
    let error = session
        .command("PROTRACTOR GUIDE")
        .expect_err("guide-line creation is not modeled");
    assert!(
        error.contains("protractor.guide_not_implemented"),
        "the refusal is named: {error}"
    );
    // The refusal started no capture and wrote nothing.
    assert!(session.snapshot(None, None, 0).tool.is_none());
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
