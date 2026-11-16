//! CHAMFER geometry: the distance method in pick order, the zero-distance
//! corner parity with a zero-radius FILLET, and the angle method.
//!
//! Split out of `tests/drawing/chamfer_fillet.rs` to keep that file under the
//! code-size limit.

use super::*;

#[test]
fn chamfer_distances_apply_in_pick_order() {
    let (mut app, mut session) = l_corner();
    // Dist1 = 2 applies to the first-picked line (the +x axis), Dist2 = 5 to the
    // second (the +y axis). A reversed application would put x=5 on L1.
    set_number(&mut session, ToolParameter::ChamferDistanceA, 2.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 5.0);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();
    // Two updates + one inserted LINE, one transaction and one undo step.
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 3);
    assert_eq!(outcome.objects.len(), 3, "two updated lines + one new line");
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    // L1 trimmed to (2,0), L2 trimmed to (0,5).
    assert!(
        line_is(&app, 1, point(2.0, 0.0), point(10.0, 0.0)),
        "{:?}",
        line_ends(&app, 1)
    );
    assert!(
        line_is(&app, 2, point(0.0, 5.0), point(0.0, 10.0)),
        "{:?}",
        line_ends(&app, 2)
    );
    // The joining LINE runs from the L1 offset point to the L2 offset point.
    let link = outcome.objects[2];
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(link.0))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => {
            let same = |p: Point3, q: Point3| (p.x - q.x).abs() < 1e-9 && (p.y - q.y).abs() < 1e-9;
            assert!(
                (same(*start, point(2.0, 0.0)) && same(*end, point(0.0, 5.0)))
                    || (same(*start, point(0.0, 5.0)) && same(*end, point(2.0, 0.0))),
                "{start:?} .. {end:?}"
            );
        }
        other => panic!("expected the joining LINE, got {other:?}"),
    }

    // One undo restores both lines and removes the inserted one.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(line_is(&app, 1, point(0.0, 0.0), point(10.0, 0.0)));
    assert!(line_is(&app, 2, point(0.0, 0.0), point(0.0, 10.0)));
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(link.0))
        .is_none());
}

#[test]
fn chamfer_zero_zero_matches_fillet_zero_radius() {
    // The ADR-named parity: A=B=0 CHAMFER equals R=0 FILLET — both trim/extend to
    // the corner and write no new entity.
    let (mut chamfer_app, mut chamfer_session) = l_corner();
    set_number(&mut chamfer_session, ToolParameter::ChamferDistanceA, 0.0);
    set_number(&mut chamfer_session, ToolParameter::ChamferDistanceB, 0.0);
    let chamfer_outcome = execute(
        &mut chamfer_app,
        &mut chamfer_session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();

    let (mut fillet_app, mut fillet_session) = l_corner();
    set_number(&mut fillet_session, ToolParameter::FilletRadius, 0.0);
    let fillet_outcome = execute(
        &mut fillet_app,
        &mut fillet_session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();

    // No insert in either: exactly the two updates, one transaction and one undo
    // step each (the "2 updates" shape).
    assert_eq!(chamfer_outcome.changes.as_ref().unwrap().changes.len(), 2);
    assert_eq!(fillet_outcome.changes.as_ref().unwrap().changes.len(), 2);
    assert_eq!(revision(&chamfer_app).0, 1);
    assert_eq!(revision(&fillet_app).0, 1);
    assert_eq!(chamfer_app.history[&DocumentId(1)].undo_depth(), 1);
    assert_eq!(fillet_app.history[&DocumentId(1)].undo_depth(), 1);
    assert_eq!(chamfer_outcome.objects, fillet_outcome.objects);
    // The resulting geometry is identical: both lines meet at the corner (0,0).
    assert_eq!(line_ends(&chamfer_app, 1), line_ends(&fillet_app, 1));
    assert_eq!(line_ends(&chamfer_app, 2), line_ends(&fillet_app, 2));
    assert!(line_is(&chamfer_app, 1, point(0.0, 0.0), point(10.0, 0.0)));
    assert!(line_is(&chamfer_app, 2, point(0.0, 0.0), point(0.0, 10.0)));
}

#[test]
fn chamfer_angle_method_uses_the_documented_derivation() {
    let (mut app, mut session) = l_corner();
    session
        .drafting_extras
        .corner_modes_mut()
        .set_chamfer_method(crate::session_state::ChamferMethod::Angle);
    session
        .drafting_extras
        .corner_modes_mut()
        .set_chamfer_angle_degrees(30.0)
        .unwrap();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 2.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .expect("the angle method chamfers a perpendicular corner");
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 3);
    // d₂ = d·sin α / sin(φ + α) = 2·tan(30°) for φ = 90°.
    let second = 2.0 * (30.0f64).to_radians().tan();
    assert!(line_is(&app, 1, point(2.0, 0.0), point(10.0, 0.0)));
    assert!(
        line_is(&app, 2, point(0.0, second), point(0.0, 10.0)),
        "{:?}",
        line_ends(&app, 2)
    );

    // Distance 0 + angle 0 is the documented intersect-only form.
    let (mut app, mut session) = l_corner();
    session
        .drafting_extras
        .corner_modes_mut()
        .set_chamfer_method(crate::session_state::ChamferMethod::Angle);
    set_number(&mut session, ToolParameter::ChamferDistanceA, 0.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 2);
    assert!(line_is(&app, 1, point(0.0, 0.0), point(10.0, 0.0)));
    assert!(line_is(&app, 2, point(0.0, 0.0), point(0.0, 10.0)));

    let modes = session.drafting_extras.corner_modes_mut();
    for bad in [-1.0, 180.0, f64::NAN] {
        let error = modes.set_chamfer_angle_degrees(bad).unwrap_err();
        assert!(
            error.to_string().contains("chamfer.angle_out_of_range"),
            "{error}"
        );
    }
}
