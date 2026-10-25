//! `ORTHO` (F8) contracts at the host boundary.
//!
//! The constraint itself is a pure function in [`crate::ortho`]; these tests only
//! prove the two places that must *apply* it agree: the capture path
//! ([`crate::SessionState::capture_point`], which the host reaches through
//! [`HostController::capture_tool_point`]) and the preview projection the rubber
//! band is drawn from. The host deliberately owns no second copy of the rule.

use super::*;
use crate::MeasurementToolKind;
use cad_domain::Point3;

/// Start a free-point draw capture with object snapping off, so the assertion is
/// about ORTHO and not about a snap target that happens to be nearby.
fn line_capture(controller: &mut HostController) {
    controller.set_snap_enabled(false);
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
}

#[test]
fn ortho_off_stores_the_raw_second_point() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    line_capture(&mut controller);
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller.capture_tool_point(p(10.0, 3.0)).unwrap();

    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(
        preview.points,
        vec![p(0.0, 0.0), p(10.0, 3.0)],
        "with ORTHO off the raw (already snapped) point is stored unchanged"
    );
}

#[test]
fn ortho_on_constrains_the_second_draw_point_and_the_preview_agrees() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    line_capture(&mut controller);
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();

    controller.set_ortho_enabled(true);
    // The preview cursor is the pending point entry: it must show the constrained
    // point, not the raw cursor, or the rubber band would disagree with the click.
    controller.move_tool_cursor(p(10.0, 3.0)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().cursor,
        Some(p(10.0, 0.0)),
        "the live preview is constrained against the previous point"
    );

    controller.capture_tool_point(p(10.0, 3.0)).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(
        preview.points,
        vec![p(0.0, 0.0), p(10.0, 0.0)],
        "the committed point is the constrained point the preview showed"
    );
}

#[test]
fn ortho_on_constrains_a_vertical_entry_too() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    line_capture(&mut controller);
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller.set_ortho_enabled(true);
    controller.capture_tool_point(p(3.0, 10.0)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().points,
        vec![p(0.0, 0.0), p(0.0, 10.0)]
    );
}

#[test]
fn ortho_on_constrains_a_measurement_point_the_same_way() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    // Polyline length is open-ended, so capturing does not auto-complete and the
    // second stored point stays observable.
    controller
        .session
        .begin_measurement(MeasurementToolKind::PolylineLength)
        .unwrap();
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();

    controller.set_ortho_enabled(true);
    controller.move_tool_cursor(p(10.0, 3.0)).unwrap();
    assert_eq!(
        controller.measurement_preview().unwrap().cursor,
        Some(p(10.0, 0.0)),
        "the measurement preview is constrained like the draw preview"
    );

    controller.capture_tool_point(p(10.0, 3.0)).unwrap();
    assert_eq!(
        controller.measurement_preview().unwrap().points,
        vec![p(0.0, 0.0), p(10.0, 0.0)],
        "a measurement point capture goes through the same constraint"
    );
}

#[test]
fn ortho_never_constrains_a_pending_entity_pick_preview() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller.set_ortho_enabled(true);
    controller.begin_draw_tool(DrawToolKind::Trim).unwrap();
    // The first TRIM step is an entity pick: its click path is
    // `capture_draw_pick`, which bypasses the point constraint, so the preview
    // must stay raw or it would disagree with the commit.
    let document = controller.document_id;
    let reference = crate::selection::model_ref(document, cad_domain::EntityId(1));
    controller
        .session
        .capture_draw_pick(p(0.0, 0.0), reference)
        .unwrap();
    controller.move_tool_cursor(p(10.0, 3.0)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().cursor,
        Some(p(10.0, 3.0)),
        "an entity pick is not a point entry and is never constrained"
    );
}

#[test]
fn ortho_leaves_z_on_the_raw_input() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    line_capture(&mut controller);
    let first = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    let second = Point3 {
        x: 10.0,
        y: 3.0,
        z: 7.0,
    };
    controller.capture_tool_point(first).unwrap();
    controller.set_ortho_enabled(true);
    controller.capture_tool_point(second).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(preview.points[1].x, 10.0);
    assert_eq!(preview.points[1].y, 0.0);
    assert_eq!(
        preview.points[1].z, 7.0,
        "ORTHO is a 2D aid: it never moves a point in Z"
    );
}
