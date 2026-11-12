//! EXTEND with a **SPLINE target**: one declared tangent-continuation rule.
//!
//! An open NURBS curve is not extrapolated as a curve. The picked end grows along
//! its own **end tangent** as a straight tangent tail (`super::extend_spline`):
//! the source control points/weights/knots are kept, `degree + 1` collinear control
//! points plus `degree + 1` copies of the new end knot are appended, so the source
//! curve is **unchanged** on its own knot domain and the joint is C0/G1. The tests
//! below pin that shape, both ends (the start is the exact mirror), the sampled
//! boundary diagnostic and every named refusal (nothing written, no undo entry).

use super::trim_extend_curves::{app_of, extend, geometry_of, line, near, snapshot, undo_depth};
use super::*;

/// The clamped rational cubic sampled by every test: control points
/// `(0,0) (1,1) (2,1) (3,0)`, weights `1,2,2,1`, knots `0³ 1³` clamped. Its start
/// tangent is along `(1,1)` and its end tangent along `(1,-1)`, so an extension of
/// `2√2` lands exactly on `x = 5` (end) or `x = -2` (start).
fn spline_geometry() -> SemanticGeometry {
    SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 1.0),
            point(2.0, 1.0),
            point(3.0, 0.0),
        ],
        weights: vec![1.0, 2.0, 2.0, 1.0],
    }
}

fn spline(id: u128, order: i64) -> DbEntity {
    ent(id, spline_geometry(), order)
}

/// The stored spline expanded into a real curve (the same helper the command uses).
fn curve_of(app: &Application, id: u128) -> cad_geometry::NurbsCurve {
    match geometry_of(app, id) {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => cad_geometry::expand_spline_source(degree, &knots, &control_points, &weights)
            .expect("the stored spline must stay valid"),
        other => panic!("expected a SPLINE, got {other:?}"),
    }
}

/// `(degree, knots, control_points, weights)` of a stored SPLINE.
fn parts(geometry: SemanticGeometry) -> (u32, Vec<f64>, Vec<Point3>, Vec<f64>) {
    match geometry {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => (degree, knots, control_points, weights),
        other => panic!("expected a SPLINE, got {other:?}"),
    }
}

fn revision_of(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn extend_an_open_spline_along_its_end_tangent_to_the_nearest_crossing() {
    // Pick the right end; the vertical boundary x = 5 crosses the end-tangent ray
    // (from (3,0) along (1,-1)/√2) exactly at (5,-2), 2√2 away.
    let mut app = app_of(vec![
        spline(1, 0),
        line(2, point(5.0, -10.0), point(5.0, 10.0), 1),
    ]);
    let source = curve_of(&app, 1);
    let (t_min, t_max) = source.domain();
    let pick = source.evaluate(t_min + 0.9 * (t_max - t_min));
    let before = revision_of(&app);
    let outcome = extend(&mut app, 1, &[2], pick).unwrap();
    assert_eq!(outcome.diagnostics[0].code, "drawing.extend");
    assert_eq!(revision_of(&app).0, before.0 + 1, "one transaction");
    assert_eq!(
        outcome.objects,
        vec![ObjectId(1)],
        "the target is replaced in place"
    );

    let (degree, knots, control_points, weights) = parts(geometry_of(&app, 1));
    assert_eq!(degree, 3, "the degree is preserved");
    // The source control polygon/weights come first, unchanged.
    assert_eq!(control_points.len(), 4 + 4, "{control_points:?}");
    assert_eq!(
        control_points[..4],
        vec![
            point(0.0, 0.0),
            point(1.0, 1.0),
            point(2.0, 1.0),
            point(3.0, 0.0)
        ],
        "the original control points are retained"
    );
    assert_eq!(weights[..4], vec![1.0, 2.0, 2.0, 1.0], "weights preserved");
    assert_eq!(
        weights[4..],
        vec![1.0; 4],
        "the straight tail is non-rational"
    );
    // The appended polygon is the straight tangent continuation: (3,0) -> (5,-2).
    assert!(
        near(control_points[4].x, 3.0) && near(control_points[4].y, 0.0),
        "the tail starts at the old end: {:?}",
        control_points[4]
    );
    for (index, expected) in [
        (5, point(3.0 + 2.0 / 3.0, -2.0 / 3.0)),
        (6, point(3.0 + 4.0 / 3.0, -4.0 / 3.0)),
        (7, point(5.0, -2.0)),
    ] {
        assert!(
            near(control_points[index].x, expected.x) && near(control_points[index].y, expected.y),
            "appended point {index} is {:?}, expected {expected:?}",
            control_points[index]
        );
    }
    // The source knots survive bit-for-bit and only the new end knot is appended.
    assert_eq!(knots[..8], vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
    assert_eq!(knots.len(), 12);
    assert!(
        knots[8..].iter().all(|knot| near(*knot, knots[8])),
        "the appended end knot has multiplicity degree+1: {knots:?}"
    );

    // The curve itself: unchanged on the source domain, exact new end, straight tail.
    let extended = curve_of(&app, 1);
    let (new_min, new_max) = extended.domain();
    assert!(near(new_min, 0.0));
    assert!(near(new_max, knots[8]), "the domain grew by one span");
    // The source occupies the first span [0, 1] (the tail is appended after it).
    let mid = source.evaluate(t_min + 0.5 * (t_max - t_min));
    let kept = extended.evaluate(0.5);
    assert!(
        near(kept.x, mid.x) && near(kept.y, mid.y),
        "the source curve is unchanged: kept {kept:?}, source {mid:?}"
    );
    let joint = extended.evaluate(1.0);
    assert!(
        near(joint.x, 3.0) && near(joint.y, 0.0),
        "C0 at the joint: {joint:?}"
    );
    let end = extended.evaluate(new_max);
    assert!(
        near(end.x, 5.0) && near(end.y, -2.0),
        "the new end is the boundary crossing: {end:?}"
    );
    let tail_mid = extended.evaluate(1.5);
    assert!(
        near(tail_mid.x, 4.0) && near(tail_mid.y, -1.0),
        "the added span is the straight tangent segment: {tail_mid:?}"
    );
    // G1: the tail's tangent is the source's own end tangent, (1,-1)/√2.
    let tail_tangent = extended.tangent(1.5);
    assert!(
        tail_tangent.x > 0.0 && near(tail_tangent.x, -tail_tangent.y),
        "the tail continues the end tangent: {tail_tangent:?}"
    );
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("straight tangent continuation")),
        "the rule is declared in the diagnostic: {:?}",
        outcome.diagnostics
    );
    assert_eq!(undo_depth(&app), 1, "one undo entry");

    // One undo restores the source bit-exactly.
    execute(
        &mut app,
        &mut SessionState::new(DocumentId(1), AppMode::Work),
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let (_, knots, control_points, weights) = parts(geometry_of(&app, 1));
    assert_eq!(knots, vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
    assert_eq!(control_points.len(), 4);
    assert_eq!(weights, vec![1.0, 2.0, 2.0, 1.0]);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn extend_an_open_spline_at_the_other_end_is_the_exact_mirror() {
    // Pick the left end: the tail is prepended, so the curve starts at (-2,-2)
    // (the crossing with x = -2 along (-1,-1)/√2) and the source occupies the
    // *second* span, with its own direction and control points untouched.
    let mut app = app_of(vec![
        spline(1, 0),
        line(2, point(-2.0, -10.0), point(-2.0, 10.0), 1),
    ]);
    let source = curve_of(&app, 1);
    let (t_min, t_max) = source.domain();
    let pick = source.evaluate(t_min + 0.1 * (t_max - t_min));
    let outcome = extend(&mut app, 1, &[2], pick).unwrap();
    assert_eq!(outcome.objects, vec![ObjectId(1)]);

    let (degree, knots, control_points, weights) = parts(geometry_of(&app, 1));
    assert_eq!(degree, 3);
    assert_eq!(control_points.len(), 8, "{control_points:?}");
    // The new tail is first; the source control points are the last four, in their
    // original order and direction.
    assert!(
        near(control_points[0].x, -2.0) && near(control_points[0].y, -2.0),
        "the new start is the boundary crossing: {:?}",
        control_points[0]
    );
    assert!(
        near(control_points[3].x, 0.0) && near(control_points[3].y, 0.0),
        "the tail joins the old start: {:?}",
        control_points[3]
    );
    assert_eq!(
        control_points[4..],
        vec![
            point(0.0, 0.0),
            point(1.0, 1.0),
            point(2.0, 1.0),
            point(3.0, 0.0)
        ],
        "the original control points are retained in order"
    );
    assert_eq!(weights[4..], vec![1.0, 2.0, 2.0, 1.0], "weights preserved");
    assert_eq!(knots.len(), 12);
    assert!(
        knots[8..].iter().all(|knot| near(*knot, knots[8])) && knots[8] > knots[7],
        "the new end knot has multiplicity degree+1: {knots:?}"
    );

    let extended = curve_of(&app, 1);
    let (new_min, new_max) = extended.domain();
    let start = extended.evaluate(new_min);
    assert!(
        near(start.x, -2.0) && near(start.y, -2.0),
        "the curve now starts at the crossing: {start:?}"
    );
    // The source is unchanged: its own midpoint is the second span's midpoint.
    let mid = source.evaluate(t_min + 0.5 * (t_max - t_min));
    let kept = extended.evaluate(new_min + 0.75 * (new_max - new_min));
    assert!(
        near(kept.x, mid.x) && near(kept.y, mid.y),
        "the source curve is unchanged: kept {kept:?}, source {mid:?}"
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn extend_a_spline_refuses_when_nothing_crosses_the_tangent_ray_ahead() {
    // The boundary is behind the picked end: the spline must not shrink.
    let mut app = app_of(vec![
        spline(1, 0),
        line(2, point(-2.0, -10.0), point(-2.0, 10.0), 1),
    ]);
    let source = curve_of(&app, 1);
    let (t_min, t_max) = source.domain();
    let pick = source.evaluate(t_min + 0.9 * (t_max - t_min));
    let before = snapshot(&app);
    let error = extend(&mut app, 1, &[2], pick).unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(
        error.to_string().contains("extend.no_intersection"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before, "a refusal changes nothing");
    assert_eq!(undo_depth(&app), 0, "a refusal opens no transaction");

    // A boundary that is not coplanar with the extension (a full circle in the
    // plane z = 5) never meets the z = 0 tangent ray; it is the same
    // `extend.no_intersection` refusal, never a fabricated crossing.
    let mut app = app_of(vec![
        spline(1, 0),
        ent(
            2,
            SemanticGeometry::Circle {
                center: p3(8.0, 0.0, 5.0),
                normal: p3(0.0, 0.0, 1.0),
                radius: 5.0,
            },
            1,
        ),
    ]);
    let before = snapshot(&app);
    let pick = source.evaluate(t_min + 0.9 * (t_max - t_min));
    let error = extend(&mut app, 1, &[2], pick).unwrap_err();
    assert!(
        error.to_string().contains("extend.no_intersection"),
        "an off-plane boundary is a miss, not a crossing: {error:?}"
    );
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn extend_refuses_a_closed_spline_frame_by_name() {
    // `SemanticGeometry::Spline` has no closed flag, so a closed frame (first
    // control point == last) is the only signal; there is no end to extend.
    let mut app = app_of(vec![
        ent(
            1,
            SemanticGeometry::Spline {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control_points: vec![
                    point(0.0, 0.0),
                    point(2.0, 2.0),
                    point(4.0, 2.0),
                    point(0.0, 0.0),
                ],
                weights: Vec::new(),
            },
            0,
        ),
        line(2, point(9.0, -10.0), point(9.0, 10.0), 1),
    ]);
    let before = snapshot(&app);
    let error = extend(&mut app, 1, &[2], point(0.0, 0.0)).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("extend.spline_closed"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn extend_a_spline_reports_the_sampled_boundary_diagnostic() {
    // A SPLINE boundary (a degree-1 spline here, i.e. still a straight crossing) has
    // no analytic intersection arm, so the successful EXTEND must say the crossing
    // came from the sampled fallback — exactly like a LINE/ARC target.
    let mut app = app_of(vec![
        spline(1, 0),
        ent(
            2,
            SemanticGeometry::Spline {
                degree: 1,
                knots: vec![0.0, 0.0, 1.0, 1.0],
                control_points: vec![point(5.0, -10.0), point(5.0, 10.0)],
                weights: Vec::new(),
            },
            1,
        ),
    ]);
    let source = curve_of(&app, 1);
    let (t_min, t_max) = source.domain();
    let pick = source.evaluate(t_min + 0.9 * (t_max - t_min));
    let outcome = extend(&mut app, 1, &[2], pick).unwrap();
    assert!(
        outcome.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("sampled intersection approximation")),
        "the sampled cut is reported: {:?}",
        outcome.diagnostics
    );
    let extended = curve_of(&app, 1);
    let (_, new_max) = extended.domain();
    let end = extended.evaluate(new_max);
    assert!(
        near(end.x, 5.0) && near(end.y, -2.0),
        "the crossing is still exact for a straight spline boundary: {end:?}"
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn extend_a_spline_is_refused_in_viewer_mode() {
    let mut app = app_of(vec![
        spline(1, 0),
        line(2, point(5.0, -10.0), point(5.0, 10.0), 1),
    ]);
    let before = snapshot(&app);
    let mut session = SessionState::new(DocumentId(1), AppMode::Viewer);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(1),
            boundary: vec![ref_of(2)],
            pick_point: point(3.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(snapshot(&app), before, "Viewer writes nothing");
    assert_eq!(undo_depth(&app), 0);
}
