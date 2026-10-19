//! CHAMFER / FILLET two-line edit contracts (ADR 0016 decisions B/C).

use super::*;

/// The two endpoints of a LINE entity, or a panic.
fn line_ends(app: &Application, entity: u128) -> (Point3, Point3) {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .unwrap_or_else(|| panic!("entity {entity} must exist"))
        .geometry
    {
        SemanticGeometry::Line { start, end } => (*start, *end),
        other => panic!("expected a LINE, got {other:?}"),
    }
}

/// Whether a LINE entity's two endpoints are exactly `a`/`b` (in either order).
fn line_is(app: &Application, entity: u128, a: Point3, b: Point3) -> bool {
    let (start, end) = line_ends(app, entity);
    let same = |p: Point3, q: Point3| (p.x - q.x).abs() < 1e-9 && (p.y - q.y).abs() < 1e-9;
    (same(start, a) && same(end, b)) || (same(start, b) && same(end, a))
}

/// A drawing with two straight lines (ids 1 and 2) on layer 0, a second layer
/// `5`, and an ARC (id 3) for the wrong-type refusal.
fn two_line_app(l1: (Point3, Point3), l2: (Point3, Point3)) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(5), "CORNER")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: l1.0,
            end: l1.1,
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        2,
        SemanticGeometry::Line {
            start: l2.0,
            end: l2.1,
        },
        1,
    ))
    .unwrap();
    b.insert_entity(ent(
        3,
        SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
            start: 0.0,
            sweep: 1.0,
        },
        2,
    ))
    .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

/// An L corner at the origin: L1 the +x axis, L2 the +y axis, both 10 long.
fn l_corner() -> (Application, SessionState) {
    two_line_app(
        (point(0.0, 0.0), point(10.0, 0.0)),
        (point(0.0, 0.0), point(0.0, 10.0)),
    )
}

fn set_number(session: &mut SessionState, parameter: ToolParameter, value: f64) {
    session
        .set_tool_parameter(parameter, ToolValue::Number(value))
        .unwrap_or_else(|error| panic!("{parameter:?} = {value} refused: {error}"));
}

fn chamfer(first: u128, second: u128, pick_first: Point3, pick_second: Point3) -> CommandPayload {
    CommandPayload::Chamfer(Box::new(TwoLineCommand {
        first: ref_of(first),
        second: ref_of(second),
        pick_first,
        pick_second,
    }))
}

fn fillet(first: u128, second: u128, pick_first: Point3, pick_second: Point3) -> CommandPayload {
    CommandPayload::Fillet(Box::new(TwoLineCommand {
        first: ref_of(first),
        second: ref_of(second),
        pick_first,
        pick_second,
    }))
}

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
fn fillet_picks_choose_the_corner_and_the_arc_centre_lies_on_the_pick_side() {
    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 3);
    assert_eq!(revision(&app).0, 1);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // The tangent points at distance R/tan(45°) = R = 2 from the corner.
    assert!(
        line_is(&app, 1, point(2.0, 0.0), point(10.0, 0.0)),
        "{:?}",
        line_ends(&app, 1)
    );
    assert!(
        line_is(&app, 2, point(0.0, 2.0), point(0.0, 10.0)),
        "{:?}",
        line_ends(&app, 2)
    );

    let arc_id = outcome.objects[2];
    let (center, radius, start, sweep) = match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(arc_id.0))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            ..
        } => (*center, *radius, *start, *sweep),
        other => panic!("expected the tangent ARC, got {other:?}"),
    };
    // The centre is at distance R from both infinite lines (the tangency).
    assert!(
        (center.x - 2.0).abs() < 1e-9 && (center.y - 2.0).abs() < 1e-9,
        "{center:?}"
    );
    assert!((radius - 2.0).abs() < 1e-9, "{radius}");
    assert!(
        (center.y.abs() - radius).abs() < 1e-9,
        "distance to L1 (y=0)"
    );
    assert!(
        (center.x.abs() - radius).abs() < 1e-9,
        "distance to L2 (x=0)"
    );
    // The arc's endpoints are the two tangent points.
    let at = |angle: f64| Point3 {
        x: center.x + radius * angle.cos(),
        y: center.y + radius * angle.sin(),
        z: center.z,
    };
    let p0 = at(start);
    let p1 = at(start + sweep);
    let same = |p: Point3, q: Point3| (p.x - q.x).abs() < 1e-9 && (p.y - q.y).abs() < 1e-9;
    assert!(
        (same(p0, point(2.0, 0.0)) && same(p1, point(0.0, 2.0)))
            || (same(p0, point(0.0, 2.0)) && same(p1, point(2.0, 0.0))),
        "arc endpoints {p0:?} .. {p1:?} must be the tangent points"
    );
    // The centre lies on the pick side (both picks were on the +x/+y arms).
    assert!(center.x > 0.0 && center.y > 0.0);

    // The pick side follows the pick: with a pick on the -x arm of the same line
    // the centre flips to the -x side.
    let (mut app2, mut session2) = two_line_app(
        (point(-10.0, 0.0), point(10.0, 0.0)),
        (point(0.0, 0.0), point(0.0, 10.0)),
    );
    set_number(&mut session2, ToolParameter::FilletRadius, 2.0);
    let outcome2 = execute(
        &mut app2,
        &mut session2,
        CommandId::FilletLines,
        fillet(1, 2, point(-8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();
    let inserted = app2.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome2.objects[2].0))
        .unwrap();
    match &inserted.geometry {
        SemanticGeometry::Arc { center, .. } => {
            assert!(center.x < 0.0 && center.y > 0.0, "{center:?}");
        }
        other => panic!("expected the new ARC, got {other:?}"),
    }
}

#[test]
fn fillet_zero_radius_trims_met_lines_and_extends_unmet_lines_to_the_corner() {
    // Met lines (both reach the corner): trimmed to the corner.
    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();
    assert!(line_is(&app, 1, point(0.0, 0.0), point(10.0, 0.0)));
    assert!(line_is(&app, 2, point(0.0, 0.0), point(0.0, 10.0)));

    // Unmet lines (both stop short of the corner): extended to it.
    let (mut app, mut session) = two_line_app(
        (point(2.0, 0.0), point(10.0, 0.0)),
        (point(0.0, 2.0), point(0.0, 10.0)),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();
    assert!(
        line_is(&app, 1, point(0.0, 0.0), point(10.0, 0.0)),
        "{:?}",
        line_ends(&app, 1)
    );
    assert!(
        line_is(&app, 2, point(0.0, 0.0), point(0.0, 10.0)),
        "{:?}",
        line_ends(&app, 2)
    );
}

#[test]
fn fillet_and_chamfer_new_entity_follows_the_declared_layer_rule() {
    // The new entity goes on the **active layer** (the ADR records this as our
    // choice); the two updated lines keep their own layer.
    let (mut app, mut session) = l_corner();
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(5)),
    )
    .unwrap();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();
    let line_id = outcome.objects[2];
    let inserted = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(line_id.0))
        .unwrap();
    assert_eq!(
        inserted.layer,
        LayerId(5),
        "the new LINE follows the active layer"
    );
    assert_eq!(inserted.space, SpaceId::Model);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .unwrap()
            .layer,
        LayerId(0),
        "the trimmed source keeps its own layer"
    );

    let (mut app, mut session) = l_corner();
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(5)),
    )
    .unwrap();
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();
    let arc_id = outcome.objects[2];
    let inserted = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(arc_id.0))
        .unwrap();
    assert_eq!(
        inserted.layer,
        LayerId(5),
        "the new ARC follows the active layer"
    );
    assert_eq!(inserted.space, SpaceId::Model);
}

/// Run a CHAMFER/FILLET payload that must be refused with no state change.
fn assert_refused(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
    expected_unsupported: bool,
) {
    let rev = revision(app);
    let depth = app
        .history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0);
    let error = execute(app, session, id, payload).unwrap_err();
    if expected_unsupported {
        assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    } else {
        assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    }
    assert_eq!(revision(app), rev, "a refused edit changes nothing");
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|history| history.undo_depth())
            .unwrap_or(0),
        depth,
        "a refused edit records no undo step"
    );
}

#[test]
fn chamfer_and_fillet_refuse_parallel_lines() {
    let parallel = || {
        two_line_app(
            (point(0.0, 0.0), point(10.0, 0.0)),
            (point(0.0, 5.0), point(10.0, 5.0)),
        )
    };
    let (mut app, mut session) = parallel();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(8.0, 5.0)),
        false,
    );
    // FILLET refuses parallel lines too (AutoCAD auto-adjusts the radius; we
    // refuse and say so).
    let (mut app, mut session) = parallel();
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(8.0, 5.0)),
        false,
    );
}

#[test]
fn chamfer_and_fillet_refuse_the_same_entity_picked_twice() {
    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 1, point(8.0, 0.0), point(2.0, 0.0)),
        false,
    );
}

#[test]
fn chamfer_and_fillet_refuse_a_cross_document_reference() {
    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    let foreign = SelectionRef {
        document: DocumentId(2),
        entity: EntityId(2),
        instance: InstancePath::default(),
        sub_element: None,
    };
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        CommandPayload::Chamfer(Box::new(TwoLineCommand {
            first: ref_of(1),
            second: foreign,
            pick_first: point(8.0, 0.0),
            pick_second: point(0.0, 8.0),
        })),
        false,
    );
}

#[test]
fn chamfer_and_fillet_refuse_a_pick_that_is_not_a_straight_line() {
    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(3, 2, point(5.0, 0.0), point(0.0, 8.0)),
        true,
    );
}

#[test]
fn chamfer_and_fillet_refuse_non_coplanar_lines() {
    let (mut app, mut session) = two_line_app(
        (point(0.0, 0.0), point(10.0, 0.0)),
        (p3(0.0, 0.0, 0.0), p3(0.0, 10.0, 5.0)),
    );
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        false,
    );
}

#[test]
fn chamfer_and_fillet_refuse_a_pick_on_the_corner() {
    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(0.0, 0.0), point(0.0, 8.0)),
        false,
    );
}

#[test]
fn chamfer_and_fillet_refuse_a_distance_too_long_for_the_line() {
    let (mut app, mut session) = l_corner();
    // The first line's pick side reaches x = 10, so A = 10 collapses the kept
    // segment; it must be refused rather than written.
    set_number(&mut session, ToolParameter::ChamferDistanceA, 10.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        false,
    );

    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::FilletRadius, 20.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        false,
    );
}

#[test]
fn chamfer_and_fillet_refuse_a_missing_active_layer() {
    let (mut app, mut session) = l_corner();
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        false,
    );
    // Even the corner-only FILLET (no new entity) refuses a missing layer.
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        false,
    );
}

#[test]
fn chamfer_and_fillet_are_work_only_and_refused_in_viewer() {
    let (mut app, mut session) = l_corner();
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    let rev = revision(&app);
    for (id, payload) in [
        (
            CommandId::ChamferLines,
            chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        ),
        (
            CommandId::FilletLines,
            fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        ),
    ] {
        let error = execute(&mut app, &mut viewer, id, payload).unwrap_err();
        assert_eq!(error, CadError::PermissionDenied, "{id:?}");
    }
    assert_eq!(revision(&app), rev);
    assert!(CommandId::ChamferLines.requires_work_mode());
    assert!(CommandId::FilletLines.requires_work_mode());
    let _ = &mut session;
}
