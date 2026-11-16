//! FILLET geometry: the pick-side corner rule, the four-quadrant pick matrix,
//! the zero-radius corner form and the line–circle tangency.
//!
//! Split out of `tests/drawing/chamfer_fillet.rs` to keep that file under the
//! code-size limit.

use super::*;

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

/// The pick-side rule (ADR 0016 decision B): each pick selects the arm of its
/// line nearest the pick, and the tangent-arc centre lies in the wedge on that
/// side. The existing coverage exercises two quadrant combinations; this pins all
/// four sign combinations of (first pick, second pick) against a fixed crossing
/// pair.
#[test]
fn fillet_pick_side_matrix_pins_the_centre_and_trim_points_in_all_four_quadrants() {
    // Fixed crossing pair: L1 the full x-axis, L2 the full y-axis, corner (0,0).
    // R = 2 and the arms are perpendicular, so the tangent distance is
    // R/tan(45°) = 2: the trim points are (sign(pick1.x)*2, 0) on L1 and
    // (0, sign(pick2.y)*2) on L2, and the centre is (sign1*2, sign2*2).
    let cases: [(&str, Point3, Point3, Point3); 4] = [
        ("+x / +y", point(8.0, 0.0), point(0.0, 8.0), point(2.0, 2.0)),
        (
            "-x / +y",
            point(-8.0, 0.0),
            point(0.0, 8.0),
            point(-2.0, 2.0),
        ),
        (
            "-x / -y",
            point(-8.0, 0.0),
            point(0.0, -8.0),
            point(-2.0, -2.0),
        ),
        (
            "+x / -y",
            point(8.0, 0.0),
            point(0.0, -8.0),
            point(2.0, -2.0),
        ),
    ];
    for (label, pick1, pick2, centre) in cases {
        let (mut app, mut session) = two_line_app(
            (point(-10.0, 0.0), point(10.0, 0.0)),
            (point(0.0, -10.0), point(0.0, 10.0)),
        );
        set_number(&mut session, ToolParameter::FilletRadius, 2.0);
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::FilletLines,
            fillet(1, 2, pick1, pick2),
        )
        .unwrap_or_else(|error| panic!("{label}: fillet refused: {error}"));

        let sign1 = pick1.x.signum();
        let sign2 = pick2.y.signum();
        assert!(
            line_is(&app, 1, point(sign1 * 2.0, 0.0), point(sign1 * 10.0, 0.0)),
            "{label}: L1 trim: {:?}",
            line_ends(&app, 1)
        );
        assert!(
            line_is(&app, 2, point(0.0, sign2 * 2.0), point(0.0, sign2 * 10.0)),
            "{label}: L2 trim: {:?}",
            line_ends(&app, 2)
        );
        let arc_id = outcome.objects[2];
        match &app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(arc_id.0))
            .unwrap()
            .geometry
        {
            SemanticGeometry::Arc { center, radius, .. } => {
                assert!(
                    (center.x - centre.x).abs() < 1e-9 && (center.y - centre.y).abs() < 1e-9,
                    "{label}: arc centre {center:?} must lie in the picked wedge"
                );
                assert!((radius - 2.0).abs() < 1e-9, "{label}: radius {radius}");
            }
            other => panic!("{label}: expected the tangent ARC, got {other:?}"),
        }
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
fn fillet_meets_a_circle_tangentially_and_never_trims_it() {
    let (mut app, mut session) =
        line_circle_app((point(0.0, 0.0), point(20.0, 0.0)), point(10.0, 5.0), 3.0);
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    let circle_before = db_entity(&app, 2);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(5.0, 0.0), point(8.0, 3.0)),
    )
    .expect("the line-circle fillet is supported");
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 2);
    assert_eq!(outcome.objects.len(), 2, "trimmed line + inserted arc");

    let circle_after = db_entity(&app, 2);
    assert_eq!(circle_after.geometry, circle_before.geometry);
    assert_eq!(circle_after.object.revision, circle_before.object.revision);

    let arc = db_entity(&app, outcome.objects[1].0);
    let SemanticGeometry::Arc { center, radius, .. } = arc.geometry else {
        panic!("expected the tangent ARC, got {:?}", arc.geometry);
    };
    assert!((radius - 2.0).abs() < 1e-9);
    assert!((center.y.abs() - 2.0).abs() < 1e-9, "{center:?}");
    let gap = ((center.x - 10.0).powi(2) + (center.y - 5.0).powi(2)).sqrt();
    assert!((gap - 5.0).abs() < 1e-9, "{gap}");
}
