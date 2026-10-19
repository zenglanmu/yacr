//! DIMLINEAR/DIMALIGNED host-path contract test (ADR 0017 decision C, P5c).
//!
//! Split out of `tools.rs` to keep that module inside the code-size limit. The
//! command-level numeric contracts live in `crate::tests::drawing::dimension`;
//! this file pins the host wiring: begin the tool, capture the three picks, map
//! the intent onto the command payload, and confirm exactly one entity.

use super::*;

#[test]
fn dimlinear_capture_confirms_one_dimension_and_maps_the_exact_picks() {
    // The host path end to end: begin the DIMLINEAR tool, capture the three
    // picks, map the intent onto the command payload, then confirm exactly one
    // entity. DIMLINEAR is fixed arity 3, so two picks are not confirmable.
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller.new_blank_document().unwrap();
    let count = controller.drawing().unwrap().entity_count();

    controller.begin_draw_tool(DrawToolKind::DimLinear).unwrap();
    let first = p(0.0, 0.0);
    let second = p(10.0, 5.0);
    let location = p(5.0, 12.0);
    controller.capture_tool_point(first).unwrap();
    controller.capture_tool_point(second).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(!preview.can_confirm(), "two picks are not a dimension");
    assert_eq!(preview.remaining_points(), Some(1));
    assert!(!preview.is_open_ended());
    assert!(controller.confirm_draw().is_err());

    controller.capture_tool_point(location).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.can_confirm());
    assert_eq!(preview.points, vec![first, second, location]);

    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        crate::DrawIntent::DimLinear {
            first,
            second,
            location,
        }
    );
    let command = controller.command_for_draw_intent(intent, &[]).unwrap();
    assert_eq!(command.id, crate::CommandId::DimLinear);
    match command.payload {
        crate::CommandPayload::Points(points) => {
            assert_eq!(
                points,
                vec![first, second, location],
                "payload matches capture"
            )
        }
        _ => panic!("expected a Points payload"),
    }

    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(controller.drawing().unwrap().entity_count(), count + 1);
    let entity = controller
        .drawing()
        .unwrap()
        .entity(cad_domain::EntityId(outcome.objects[0].0))
        .cloned()
        .expect("the created dimension");
    assert_eq!(entity.object.type_key, "AcDbDimension");
}
