//! PLINE create-command contract tests (the phase-P3 open-ended draw template).
//!
//! Split out of `create_trim.rs` to keep both files under the 1000-line code
//! limit. The tests pin the honest subset: one open straight-segment polyline
//! per capture, one transaction and one undo step, and explicit refusals
//! (fewer than two points, all-identical points, non-finite points, a
//! mismatched programmatic payload, a missing active layer) that change nothing.

use super::*;

#[test]
fn create_polyline_inserts_one_open_polyline_and_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let captured = vec![point(0.0, 0.0), point(1.0, 0.0), point(1.0, 1.0)];
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolyline,
        CommandPayload::Points(captured.clone()),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one entity for one capture");
    let oid = outcome.objects[0];
    let cs = outcome.changes.expect("change set");
    assert_eq!(cs.before, before);
    assert_eq!(cs.after.0, before.0 + 1);
    assert_eq!(cs.changes.len(), 1, "one transaction");
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(oid.0))
        .expect("new entity");
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert_eq!(points, &captured, "payload points inserted unchanged");
            assert_eq!(bulges.len(), captured.len(), "one bulge per captured point");
            assert!(bulges.iter().all(|bulge| *bulge == 0.0), "{bulges:?}");
            assert!(!*closed, "PLINE authors an open polyline");
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbPolyline");
    assert!(app.can_undo(&DocumentId(1)));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // Undo removes the polyline exactly; redo restores it.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(oid.0))
        .is_none());
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(oid.0))
        .is_some());
}

#[test]
fn create_polyline_two_points_carry_one_bulge_each_and_stay_open() {
    let (mut app, mut session) = drawing_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolyline,
        CommandPayload::Points(vec![point(0.0, 0.0), point(3.0, 4.0)]),
    )
    .unwrap();
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .unwrap();
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert_eq!(points, &vec![point(0.0, 0.0), point(3.0, 4.0)]);
            assert_eq!(bulges, &vec![0.0, 0.0]);
            assert!(!*closed);
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
}

#[test]
fn create_polyline_refuses_degenerate_captures_without_change() {
    // Single point, all-identical points, and an empty capture are all explicit
    // refusals that leave the drawing, revision and history untouched.
    let cases = [
        vec![point(1.0, 1.0)],
        vec![point(2.0, 2.0), point(2.0, 2.0), point(2.0, 2.0)],
        Vec::new(),
    ];
    for points in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreatePolyline,
            CommandPayload::Points(points.clone()),
        )
        .unwrap_err();
        assert!(matches!(err, CadError::InvalidInput(_)), "{points:?}");
        assert_eq!(revision(&app), rev, "{points:?}");
        assert!(!app.can_undo(&DocumentId(1)), "{points:?}");
    }
}

#[test]
fn create_polyline_refuses_a_non_finite_point_without_change() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolyline,
        CommandPayload::Points(vec![point(0.0, 0.0), point(f64::NAN, 1.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_polyline_on_a_missing_layer_is_refused_by_the_shared_funnel() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolyline,
        CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_polyline_rejects_a_wrong_geometry_payload() {
    // The programmatic `Geometry` arm accepts only a straight open Polyline; a
    // mismatched kind is refused instead of being coerced.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolyline,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        })),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
}

// ---------------------------------------------------------------------------
// The authored (`Geometry(Polyline)`) arm: PLINE's `Arc` segment mode (stored as
// bulges) and its `Close` option, both already modeled by the domain.
// ---------------------------------------------------------------------------

/// Execute one `Geometry(Polyline)` payload and read back the stored parts.
fn execute_polyline_geometry(
    app: &mut Application,
    session: &mut SessionState,
    points: Vec<Point3>,
    bulges: Vec<f64>,
    closed: bool,
) -> CadResult<(Vec<Point3>, Vec<f64>, bool)> {
    let outcome = execute(
        app,
        session,
        CommandId::CreatePolyline,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        })),
    )?;
    Ok(stored_polyline(app, outcome.objects[0]))
}

/// The stored `(points, bulges, closed)` of one created polyline.
fn stored_polyline(app: &Application, object: ObjectId) -> (Vec<Point3>, Vec<f64>, bool) {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the polyline entity");
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbPolyline");
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => (points.clone(), bulges.clone(), *closed),
        other => panic!("expected a polyline, got {other:?}"),
    }
}

#[test]
fn create_polyline_arc_segment_mode_stores_the_bulges() {
    // PLINE `Arc`: `bulges[i]` is the tan(θ/4) of the segment leaving vertex `i`,
    // so a 0.5 bulge is the documented 180°-ish quarter/half arc and is stored
    // exactly as authored. One entity, one transaction, one undo step.
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let points = vec![point(0.0, 0.0), point(2.0, 0.0), point(2.0, 2.0)];
    let bulges = vec![0.5, 0.0, 0.0];
    let (stored_points, stored_bulges, closed) = execute_polyline_geometry(
        &mut app,
        &mut session,
        points.clone(),
        bulges.clone(),
        false,
    )
    .unwrap();
    assert_eq!(stored_points, points);
    assert_eq!(stored_bulges, bulges, "the authored bulges are stored");
    assert!(!closed);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    assert!(revision(&app).0 > before.0, "one revision bump");

    // The stored bulge resolves to the same circular arc the author described: the
    // chord is 2 world units and a 0.5 bulge is tan(θ/4), i.e. a ~106° arc of radius
    // 1.25 (never a fabricated straight segment).
    let arc = cad_geometry::bulge_arc_geometry(
        stored_points[0],
        stored_points[1],
        stored_bulges[0],
        p3(0.0, 0.0, 1.0),
    )
    .expect("the stored bulge resolves to an arc");
    match arc {
        SemanticGeometry::Arc { radius, sweep, .. } => {
            assert!(
                (radius - 1.25).abs() < 1e-9,
                "a 0.5 bulge over a 2-unit chord has radius 1.25, got {radius}"
            );
            assert!(
                (sweep.abs() - 4.0 * 0.5_f64.atan()).abs() < 1e-9,
                "the sweep is 4·atan(bulge), got {sweep}"
            );
        }
        other => panic!("expected the bulged segment to resolve to an arc, got {other:?}"),
    }
}

#[test]
fn create_polyline_close_option_stores_a_closed_ring() {
    // PLINE `Close`: a closed ring is one entity whose last segment runs from the
    // final vertex back to the first, and whose bulges are one per vertex.
    let (mut app, mut session) = drawing_app();
    let points = vec![
        point(0.0, 0.0),
        point(3.0, 0.0),
        point(3.0, 2.0),
        point(0.0, 2.0),
    ];
    let (stored_points, stored_bulges, closed) = execute_polyline_geometry(
        &mut app,
        &mut session,
        points.clone(),
        vec![0.0; points.len()],
        true,
    )
    .unwrap();
    assert_eq!(stored_points, points);
    assert_eq!(stored_bulges, vec![0.0; points.len()]);
    assert!(closed, "the ring is stored closed");

    // A closed ring with an arc segment: the last bulge owns the implicit closing
    // segment (the fillet-style rounded corner shape).
    let (stored_points, stored_bulges, closed) = execute_polyline_geometry(
        &mut app,
        &mut session,
        points.clone(),
        vec![0.0, 0.0, 0.0, 0.414_213_562_373_095_1],
        true,
    )
    .unwrap();
    assert_eq!(stored_points, points);
    assert!(closed);
    assert!(
        stored_bulges[3].abs() > 0.0,
        "the closing segment keeps its bulge: {stored_bulges:?}"
    );
}

#[test]
fn create_polyline_normalises_documented_bulge_and_ring_encodings() {
    // Two lossless encodings are accepted: an empty bulge list means "all straight",
    // and a closed ring whose last vertex repeats the first stores the implicit ring
    // (the redundant vertex carries no geometry).
    let (mut app, mut session) = drawing_app();
    let points = vec![point(0.0, 0.0), point(2.0, 0.0), point(2.0, 1.0)];
    let (stored_points, stored_bulges, closed) =
        execute_polyline_geometry(&mut app, &mut session, points.clone(), Vec::new(), false)
            .unwrap();
    assert_eq!(stored_points, points);
    assert_eq!(stored_bulges, vec![0.0; points.len()], "all straight");
    assert!(!closed, "the empty bulge list does not close the polyline");

    // One bulge per SEGMENT on an open polyline: the trailing vertex gets 0.0.
    let (_, stored_bulges, _) = execute_polyline_geometry(
        &mut app,
        &mut session,
        points.clone(),
        vec![0.25, 0.0],
        false,
    )
    .unwrap();
    assert_eq!(stored_bulges, vec![0.25, 0.0, 0.0]);

    // A repeated closing vertex collapses to the implicit ring.
    let mut repeated = points.clone();
    repeated.push(points[0]);
    let (stored_points, stored_bulges, closed) = execute_polyline_geometry(
        &mut app,
        &mut session,
        repeated,
        vec![0.0, 0.0, 0.0, 0.0],
        true,
    )
    .unwrap();
    assert_eq!(stored_points, points, "the repeated vertex is dropped");
    assert_eq!(stored_bulges, vec![0.0; points.len()]);
    assert!(closed);
}

/// One refusal case of the authored-polyline arm:
/// `(label, expected <selector>.<reason> prefix, points, bulges, closed, is_unsupported)`.
type AuthoredPolylineCase = (
    &'static str,
    &'static str,
    Vec<Point3>,
    Vec<f64>,
    bool,
    bool,
);

#[test]
fn create_polyline_refuses_unrepresentable_authored_polylines_without_change() {
    // Every authored refusal is named `pline.<reason>` and writes nothing.
    let cases: &[AuthoredPolylineCase] = &[
        (
            "one vertex",
            "pline.too_few_points",
            vec![point(0.0, 0.0)],
            vec![0.0],
            false,
            false,
        ),
        (
            "non-finite vertex",
            "pline.invalid_point",
            vec![point(0.0, 0.0), point(f64::NAN, 1.0)],
            vec![0.0, 0.0],
            false,
            false,
        ),
        (
            "non-finite bulge",
            "pline.non_finite_bulge",
            vec![point(0.0, 0.0), point(2.0, 0.0)],
            vec![f64::INFINITY, 0.0],
            false,
            false,
        ),
        (
            "closed ring with two vertices",
            "pline.closed_too_few_vertices",
            vec![point(0.0, 0.0), point(2.0, 0.0)],
            vec![0.0, 0.0],
            true,
            false,
        ),
        (
            "bulge list of the wrong length",
            "pline.bulge_count",
            vec![point(0.0, 0.0), point(2.0, 0.0), point(2.0, 1.0)],
            vec![0.0],
            false,
            false,
        ),
        (
            "all vertices coincide",
            "pline.degenerate",
            vec![point(4.0, 4.0), point(4.0, 4.0)],
            vec![0.0, 0.0],
            false,
            false,
        ),
        (
            "bulge on a zero-length chord",
            "pline.degenerate_bulge",
            vec![point(0.0, 0.0), point(0.0, 0.0), point(2.0, 0.0)],
            vec![0.5, 0.0, 0.0],
            false,
            false,
        ),
        (
            "bulged polyline off the +Z plane",
            "pline.bulged_non_planar",
            vec![point(0.0, 0.0), point(2.0, 0.0), p3(2.0, 1.0, 3.0)],
            vec![0.5, 0.0, 0.0],
            false,
            true,
        ),
    ];
    for (label, code, points, bulges, closed, unsupported) in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreatePolyline,
            CommandPayload::Geometry(Box::new(SemanticGeometry::Polyline {
                points: points.clone(),
                bulges: bulges.clone(),
                closed: *closed,
            })),
        )
        .unwrap_err();
        if *unsupported {
            match &err {
                CadError::Unsupported(message) => assert!(
                    message.starts_with(code),
                    "{label}: expected {code}, got {message}"
                ),
                other => panic!("{label}: expected an Unsupported refusal, got {other:?}"),
            }
        } else {
            match &err {
                CadError::InvalidInput(message) => assert!(
                    message.starts_with(code),
                    "{label}: expected {code}, got {message}"
                ),
                other => panic!("{label}: expected an InvalidInput refusal, got {other:?}"),
            }
        }
        assert_eq!(revision(&app), rev, "{label}: refusal must not write");
        assert!(!app.can_undo(&DocumentId(1)), "{label}");
    }
}

#[test]
fn create_polyline_straight_geometry_payload_still_authors_the_click_shape() {
    // The pre-existing programmatic shape (an open straight polyline) is unchanged by
    // the authored arm: one zero bulge per vertex.
    let (mut app, mut session) = drawing_app();
    let points = vec![point(0.0, 0.0), point(1.0, 0.0), point(1.0, 1.0)];
    let (stored_points, stored_bulges, closed) = execute_polyline_geometry(
        &mut app,
        &mut session,
        points.clone(),
        vec![0.0; points.len()],
        false,
    )
    .unwrap();
    assert_eq!(stored_points, points);
    assert_eq!(stored_bulges, vec![0.0; points.len()]);
    assert!(!closed);
}
