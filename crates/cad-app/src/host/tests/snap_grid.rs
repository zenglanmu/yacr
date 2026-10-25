//! `SNAP` (grid snap, F9) and its `SNAPUNIT` spacing at the host boundary.
//!
//! The quantiser itself is a pure function in [`crate::snap`]; these tests prove
//! the places that must *apply* it agree: the capture path
//! ([`crate::SessionState::capture_point`], reached through
//! [`HostController::capture_tool_point`]), the preview projection the rubber band
//! is drawn from, and the documented order grid snap → `ORTHO`. The host
//! deliberately owns no second copy of the rule.

use super::*;
use crate::MeasurementToolKind;
use cad_domain::Point3;

/// Start a free-point draw capture with object snapping off, so the assertion is
/// about grid snap and not about a snap target that happens to be nearby.
fn line_capture(controller: &mut HostController) {
    controller.set_snap_enabled(false);
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
}

#[test]
fn grid_snap_defaults_off_and_stores_the_raw_point() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    line_capture(&mut controller);
    assert!(
        !controller.snap_grid_enabled(),
        "SNAP defaults off, matching AutoCAD's initial state"
    );
    assert_eq!(
        controller.snap_spacing(),
        10.0,
        "the declared default spacing"
    );
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller.capture_tool_point(p(12.4, 18.6)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().points,
        vec![p(0.0, 0.0), p(12.4, 18.6)],
        "with SNAP off the raw (already object-snapped) point is stored unchanged"
    );
}

#[test]
fn grid_snap_on_quantises_the_draw_point_and_the_preview_agrees() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    line_capture(&mut controller);
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();

    controller.set_snap_grid_enabled(true);
    // The preview cursor is the pending point entry: it must show the quantised
    // point, or the rubber band would disagree with the click.
    controller.move_tool_cursor(p(12.4, 18.6)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().cursor,
        Some(p(10.0, 20.0)),
        "the live preview is snapped to the nearest grid multiple"
    );

    controller.capture_tool_point(p(12.4, 18.6)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().points,
        vec![p(0.0, 0.0), p(10.0, 20.0)],
        "the committed point is the snapped point the preview showed"
    );
}

#[test]
fn grid_snap_on_quantises_a_measurement_point_the_same_way() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    // Polyline length is open-ended, so capturing does not auto-complete and the
    // second stored point stays observable.
    controller
        .session
        .begin_measurement(MeasurementToolKind::PolylineLength)
        .unwrap();
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();

    controller.set_snap_grid_enabled(true);
    controller.move_tool_cursor(p(-12.4, -18.6)).unwrap();
    assert_eq!(
        controller.measurement_preview().unwrap().cursor,
        Some(p(-10.0, -20.0)),
        "the measurement preview is snapped like the draw preview"
    );

    controller.capture_tool_point(p(-12.4, -18.6)).unwrap();
    assert_eq!(
        controller.measurement_preview().unwrap().points,
        vec![p(0.0, 0.0), p(-10.0, -20.0)],
        "a measurement point capture goes through the same quantiser"
    );
}

#[test]
fn grid_snap_then_ortho_keeps_the_grid_axis_and_ortho_decides_direction() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    line_capture(&mut controller);
    controller.set_snap_spacing(10.0).unwrap();
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();

    // Both aids on: grid snap runs first (24, 7) -> (20, 10), then ORTHO compares
    // that snapped delta against the previous point, so the final Y is the
    // previous Y and the X stays the grid multiple.
    controller.set_snap_grid_enabled(true);
    controller.set_ortho_enabled(true);
    controller.move_tool_cursor(p(24.0, 7.0)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().cursor,
        Some(p(20.0, 0.0)),
        "the preview shows grid snap followed by the ortho constraint"
    );

    controller.capture_tool_point(p(24.0, 7.0)).unwrap();
    let points = controller.session.draw_preview().unwrap().points;
    assert_eq!(
        points,
        vec![p(0.0, 0.0), p(20.0, 0.0)],
        "the committed point is the one the preview showed"
    );
    // ORTHO won the final axis (horizontal move keeps the previous Y) and the
    // moving coordinate is still on the grid.
    assert_eq!(points[1].y, 0.0);
    assert_eq!(points[1].x % 10.0, 0.0, "the final X stays a grid multiple");
}

#[test]
fn grid_snap_never_quantises_a_pending_entity_pick_preview() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller.set_snap_grid_enabled(true);
    controller.begin_draw_tool(DrawToolKind::Trim).unwrap();
    // The first TRIM step is an entity pick: its click path is
    // `capture_draw_pick`, which bypasses the point constraints, so the preview
    // must stay raw or it would disagree with the commit.
    let document = controller.document_id;
    let reference = crate::selection::model_ref(document, cad_domain::EntityId(1));
    controller
        .session
        .capture_draw_pick(p(0.0, 0.0), reference)
        .unwrap();
    controller.move_tool_cursor(p(12.4, 18.6)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().cursor,
        Some(p(12.4, 18.6)),
        "an entity pick is not a point entry and is never snapped to the grid"
    );
}

#[test]
fn grid_snap_leaves_z_on_the_raw_input() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    line_capture(&mut controller);
    controller.set_snap_grid_enabled(true);
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller
        .capture_tool_point(Point3 {
            x: 12.4,
            y: 18.6,
            z: 7.25,
        })
        .unwrap();
    let point = controller.session.draw_preview().unwrap().points[1];
    assert_eq!((point.x, point.y), (10.0, 20.0));
    assert_eq!(point.z, 7.25, "grid snap is a 2D aid: Z is never moved");
}

#[test]
fn snapunit_refuses_invalid_spacings_and_keeps_the_previous_value() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller
        .set_snap_spacing(25.0)
        .expect("25 is a valid spacing");
    assert_eq!(controller.snap_spacing(), 25.0);

    for refused in [0.0, -0.0, -5.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            controller.set_snap_spacing(refused).is_err(),
            "{refused:?} must be refused explicitly"
        );
        assert_eq!(
            controller.snap_spacing(),
            25.0,
            "{refused:?} must not change the stored spacing"
        );
    }

    // The stored spacing is what the quantiser uses.
    line_capture(&mut controller);
    controller.set_snap_grid_enabled(true);
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller.capture_tool_point(p(30.0, 40.0)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().points,
        vec![p(0.0, 0.0), p(25.0, 50.0)],
        "a spacing of 25 rounds 30/40 to the nearest multiples (25/50)"
    );
}
