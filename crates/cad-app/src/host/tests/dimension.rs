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

/// A `Point3` with `z = 0` plus a `+Z` normal, for the host fixtures.
fn z_up() -> cad_domain::Point3 {
    cad_domain::Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    }
}

fn host_entity(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: crate::app_drawing::class_name(&geometry).to_string(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

/// Replace the controller's drawing with one circle (r 100 at the origin) and a
/// `Standard` DIMSTYLE carrying `center_mark`.
fn circle_fixture(center_mark: f64) -> HostController {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(host_entity(
            301,
            SemanticGeometry::Circle {
                center: p(0.0, 0.0),
                normal: z_up(),
                radius: 100.0,
            },
            0,
        ))
        .unwrap();
    builder
        .insert_dim_style(cad_db::DimStyle {
            center_mark,
            ..cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0))
        })
        .unwrap();
    controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    controller.set_snap_enabled(false);
    controller
}

#[test]
fn dimradius_capture_picks_the_circle_then_takes_the_location() {
    // The host path end to end: the first click is an **entity pick** resolved
    // through the same pick path TRIM/EXTEND use (a miss is refused and consumes
    // no step); the second click is the free location point.
    let mut controller = circle_fixture(0.0);
    let count = controller.drawing().unwrap().entity_count();

    controller.begin_draw_tool(DrawToolKind::DimRadius).unwrap();
    // An empty-space click at the **pick step** is a miss: refused, and no step
    // is consumed.
    assert!(controller.capture_tool_point(p(1000.0, 1000.0)).is_err());
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(preview.picks.len(), 0);
    assert_eq!(preview.points.len(), 0);

    controller.capture_tool_point(p(100.0, 0.0)).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(preview.picks.len(), 1, "the circle pick resolved");
    assert_eq!(preview.points.len(), 1);
    assert!(!preview.can_confirm(), "the location is still needed");
    assert_eq!(preview.remaining_points(), Some(1));

    controller.capture_tool_point(p(0.0, 120.0)).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.can_confirm());
    assert_eq!(preview.points.len(), 2);
    assert_eq!(preview.picks.len(), 1);

    let intent = controller.session.draw_intent().unwrap();
    assert!(
        matches!(intent, crate::DrawIntent::DimRadius { .. }),
        "{intent:?}"
    );
    let command = controller.command_for_draw_intent(intent, &[]).unwrap();
    assert_eq!(command.id, crate::CommandId::DimRadius);
    match command.payload {
        crate::CommandPayload::DimRadial(input) => {
            assert_eq!(input.target.entity, cad_domain::EntityId(301));
            assert_eq!(input.location, p(0.0, 120.0));
        }
        _ => panic!("expected a DimRadial payload"),
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

/// Replace the controller's drawing with two crossing lines and a `Standard`
/// DIMSTYLE, for the two-line angular capture.
fn crossing_lines_fixture() -> HostController {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(host_entity(
            401,
            SemanticGeometry::Line {
                start: p(-50.0, 0.0),
                end: p(50.0, 0.0),
            },
            0,
        ))
        .unwrap();
    builder
        .insert_entity(host_entity(
            402,
            SemanticGeometry::Line {
                start: p(0.0, -50.0),
                end: p(0.0, 50.0),
            },
            1,
        ))
        .unwrap();
    builder
        .insert_dim_style(cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0)))
        .unwrap();
    controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    controller.set_snap_enabled(false);
    controller
}

#[test]
fn dimangular_capture_picks_both_lines_then_the_arc_location() {
    let mut controller = crossing_lines_fixture();
    let count = controller.drawing().unwrap().entity_count();

    controller
        .begin_draw_tool(DrawToolKind::DimAngular)
        .unwrap();
    // The two leading steps are entity picks; the third is a free point.
    controller.capture_tool_point(p(25.0, 0.0)).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(preview.picks.len(), 1);
    assert_eq!(preview.remaining_points(), Some(2));

    controller.capture_tool_point(p(0.0, 25.0)).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(preview.picks.len(), 2);
    assert!(!preview.can_confirm(), "the arc location is still needed");
    assert_eq!(preview.remaining_points(), Some(1));

    let location = p(25.0, 25.0);
    controller.capture_tool_point(location).unwrap();
    let intent = controller.session.draw_intent().unwrap();
    let command = controller.command_for_draw_intent(intent, &[]).unwrap();
    assert_eq!(command.id, crate::CommandId::DimAngular);
    match command.payload {
        crate::CommandPayload::DimAngular(input) => match input.as_ref() {
            crate::AngularDimensionInput::TwoLine {
                first,
                second,
                location: arc_location,
            } => {
                assert_eq!(first.entity, cad_domain::EntityId(401));
                assert_eq!(second.entity, cad_domain::EntityId(402));
                assert_eq!(*arc_location, location);
            }
            other => panic!("expected a TwoLine input, got {other:?}"),
        },
        _ => panic!("expected a DimAngular payload"),
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

#[test]
fn dimordinate_capture_takes_the_feature_and_leader_points() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller.new_blank_document().unwrap();
    let count = controller.drawing().unwrap().entity_count();

    controller
        .begin_draw_tool(DrawToolKind::DimOrdinate)
        .unwrap();
    let feature = p(10.0, 10.0);
    let leader = p(10.0, 30.0);
    controller.capture_tool_point(feature).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(preview.remaining_points(), Some(1));
    controller.capture_tool_point(leader).unwrap();

    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(intent, crate::DrawIntent::DimOrdinate { feature, leader });
    let command = controller.command_for_draw_intent(intent, &[]).unwrap();
    assert_eq!(command.id, crate::CommandId::DimOrdinate);
    match command.payload {
        crate::CommandPayload::DimOrdinate(input) => {
            assert_eq!(input.feature, feature);
            assert_eq!(input.leader, leader);
            assert_eq!(
                input.axis, None,
                "the interactive form auto-selects the axis"
            );
        }
        _ => panic!("expected a DimOrdinate payload"),
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

#[test]
fn dimcenter_capture_picks_the_circle_and_commits_plain_lines() {
    let mut controller = circle_fixture(2.5);
    let count = controller.drawing().unwrap().entity_count();

    controller.begin_draw_tool(DrawToolKind::DimCenter).unwrap();
    controller.capture_tool_point(p(100.0, 0.0)).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.can_confirm(), "one pick defines the centre mark");
    assert_eq!(preview.picks.len(), 1);

    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        crate::DrawIntent::DimCenter {
            target: cad_domain::SelectionRef {
                document: controller.document_id,
                entity: cad_domain::EntityId(301),
                instance: cad_domain::InstancePath::default(),
                sub_element: None,
            },
        }
    );
    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 2, "a crossed pair of plain lines");
    assert_eq!(controller.drawing().unwrap().entity_count(), count + 2);
    for object in &outcome.objects {
        let entity = controller
            .drawing()
            .unwrap()
            .entity(cad_domain::EntityId(object.0))
            .cloned()
            .expect("a centre-mark entity");
        assert_eq!(
            entity.object.type_key, "AcDbLine",
            "plain geometry, not a DIMENSION"
        );
    }
}
