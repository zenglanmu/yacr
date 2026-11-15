//! `JOIN CLOSE` with a SPLINE among the sources.
//!
//! The domain SPLINE carries the closed flag (DXF group 70 bit 1), so closing a
//! NURBS weld is a stored fact rather than a duplicated endpoint. These tests pin:
//! the mixed LINE + SPLINE weld and the SPLINE + SPLINE weld both produce **one
//! closed spline** whose control polygon is the chain's own, the one-transaction /
//! one-undo-step shape, undo restoring the sources, and the named refusals when
//! the close is impossible (a gap is never bridged).

use super::*;
use cad_geometry::NurbsCurve;

fn spline_entity(id: u128, control_points: Vec<Point3>, order: i64) -> DbEntity {
    part(
        id,
        spline_geometry(control_points, 3),
        LayerId(0),
        SpaceId::Model,
        "AcDbSpline",
        order,
    )
}

/// A cubic single-span spline from `head` to `tail` bowing through two inner
/// control points (the exact shape does not matter beyond its two ends).
fn open_cubic(head: Point3, inner_a: Point3, inner_b: Point3, tail: Point3) -> Vec<Point3> {
    vec![head, inner_a, inner_b, tail]
}

fn welded(app: &Application) -> (Vec<Point3>, bool) {
    match one_geometry(app) {
        SemanticGeometry::Spline {
            control_points,
            closed,
            ..
        } => (control_points, closed),
        other => panic!("expected one SPLINE, got {other:?}"),
    }
}

fn evaluate_weld(app: &Application, t: f64) -> Point3 {
    match one_geometry(app) {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
            ..
        } => NurbsCurve::new(degree as usize, control_points, knots, weights)
            .expect("valid weld")
            .evaluate(t),
        other => panic!("expected one SPLINE, got {other:?}"),
    }
}

// --- LINE + SPLINE ---------------------------------------------------------

/// A LINE closing a cubic that returns to the line's start: an already closed
/// chain, welded from two open sources.
fn line_and_spline_loop() -> (Application, SessionState) {
    app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        spline_entity(
            2,
            open_cubic(
                point(4.0, 0.0),
                point(6.0, 3.0),
                point(-2.0, 3.0),
                point(0.0, 0.0),
            ),
            1,
        ),
    ])
}

#[test]
fn join_close_welds_a_line_and_a_spline_into_one_closed_spline() {
    let (mut app, mut session) = line_and_spline_loop();
    let before_revision = revision(&app);
    select(&mut app, &mut session, &[1, 2]);

    let outcome = join_with(&mut app, &mut session, true, JoinMode::Multiple)
        .expect("the contiguous LINE + SPLINE chain closes");
    assert_eq!(outcome.objects.len(), 1, "exactly one entity is produced");

    let (control_points, closed) = welded(&app);
    assert!(closed, "JOIN CLOSE stores the closed flag");
    assert_eq!(
        control_points.first().copied(),
        Some(point(0.0, 0.0)),
        "the chain starts at the LINE's start"
    );
    assert_eq!(
        control_points.last().copied(),
        Some(point(0.0, 0.0)),
        "the ends already met, so the welded polygon ends at the start"
    );
    // Two spans (LINE then the cubic): [0, 1] is the LINE, [1, 2] the spline.
    assert!(
        near(evaluate_weld(&app, 0.5), point(2.0, 0.0)),
        "the first span is still the LINE"
    );
    assert!(near(evaluate_weld(&app, 1.0), point(4.0, 0.0)));
    assert!(near(evaluate_weld(&app, 2.0), point(0.0, 0.0)));

    assert_eq!(entity_count(&app), 1);
    assert_eq!(revision(&app).0, before_revision.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "exactly one undo step");
}

#[test]
fn undo_restores_both_sources_of_a_closed_weld() {
    let (mut app, mut session) = line_and_spline_loop();
    select(&mut app, &mut session, &[1, 2]);
    join_with(&mut app, &mut session, true, JoinMode::Multiple).expect("the chain closes");
    assert_eq!(entity_count(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("the closed weld is undoable");
    assert_eq!(entity_count(&app), 2, "both sources are restored");
    assert_eq!(undo_depth(&app), 0);
    let mut ids: Vec<u128> = model_geometry(&app)
        .into_iter()
        .map(|(id, _)| id.0)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2], "the original ids come back");
}

// --- SPLINE + SPLINE -------------------------------------------------------

fn two_spline_loop() -> (Application, SessionState) {
    app_with(vec![
        spline_entity(
            1,
            open_cubic(
                point(0.0, 0.0),
                point(2.0, 1.0),
                point(2.0, -1.0),
                point(4.0, 0.0),
            ),
            0,
        ),
        spline_entity(
            2,
            open_cubic(
                point(4.0, 0.0),
                point(6.0, 3.0),
                point(-2.0, 3.0),
                point(0.0, 0.0),
            ),
            1,
        ),
    ])
}

#[test]
fn join_close_welds_two_splines_in_multiple_mode() {
    let (mut app, mut session) = two_spline_loop();
    select(&mut app, &mut session, &[1, 2]);
    join_with(&mut app, &mut session, true, JoinMode::Multiple).expect("the two cubic spans close");

    let (control_points, closed) = welded(&app);
    assert!(closed);
    assert_eq!(control_points.first().copied(), Some(point(0.0, 0.0)));
    assert_eq!(
        control_points.last().copied(),
        Some(point(0.0, 0.0)),
        "the stored polygon is the chain's own; no endpoint was invented"
    );
    // Each source is one cubic span, so the weld is exactly two spans.
    assert_eq!(control_points.len(), 8);
    assert!(near(evaluate_weld(&app, 0.0), point(0.0, 0.0)));
    assert!(near(evaluate_weld(&app, 1.0), point(4.0, 0.0)));
    assert!(near(evaluate_weld(&app, 2.0), point(0.0, 0.0)));
    assert_eq!(undo_depth(&app), 1, "one undo step");
}

#[test]
fn join_close_welds_two_splines_in_source_object_mode() {
    // The first pick is the source object; `Close` routes a pure-SPLINE
    // selection through the NURBS weld because only it can store the flag.
    let (mut app, mut session) = two_spline_loop();
    select(&mut app, &mut session, &[1, 2]);
    join_with(&mut app, &mut session, true, JoinMode::SourceObject)
        .expect("a SPLINE source closes with a touching SPLINE");

    let (_, closed) = welded(&app);
    assert!(closed);
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 1);
}

// --- Refusals --------------------------------------------------------------

#[test]
fn join_close_refuses_a_gap_between_pieces_by_name() {
    // The cubic runs (10, 0) → (14, 0): it does not touch the LINE's end at
    // (4, 0), so the pieces are not contiguous.
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        spline_entity(
            2,
            open_cubic(
                point(10.0, 0.0),
                point(12.0, 3.0),
                point(12.0, -3.0),
                point(14.0, 0.0),
            ),
            1,
        ),
    ]);
    let before_revision = revision(&app);
    select(&mut app, &mut session, &[1, 2]);
    let error = join_with(&mut app, &mut session, true, JoinMode::Multiple)
        .expect_err("JOIN never bridges a gap");
    let message = format!("{error:?}");
    assert!(message.contains("join.not_contiguous"), "{message}");
    assert_eq!(entity_count(&app), 2, "nothing was written");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0, "no empty undo entry");
}

#[test]
fn join_close_refuses_an_open_chain_by_name() {
    // The pieces are contiguous, but the chain's two ends are (0, 0) and
    // (10, 0): closing it would need a bridge segment.
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        spline_entity(
            2,
            open_cubic(
                point(4.0, 0.0),
                point(7.0, 2.0),
                point(9.0, 2.0),
                point(10.0, 0.0),
            ),
            1,
        ),
    ]);
    let before_revision = revision(&app);
    select(&mut app, &mut session, &[1, 2]);
    let error = join_with(&mut app, &mut session, true, JoinMode::Multiple)
        .expect_err("an open chain cannot be closed");
    let message = format!("{error:?}");
    assert!(message.contains("join.not_closed"), "{message}");
    assert_eq!(entity_count(&app), 2, "nothing was written");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_closed_spline_source_is_refused_by_name() {
    // A closed spline has no free end: the flag alone is enough, even though this
    // control polygon does not repeat its first point.
    let closed_source = SemanticGeometry::Spline {
        degree: 3,
        knots: clamped_uniform_knots(4, 3),
        control_points: vec![
            point(0.0, 0.0),
            point(4.0, 0.0),
            point(4.0, 4.0),
            point(0.0, 4.0),
        ],
        weights: vec![1.0; 4],
        closed: true,
    };
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            closed_source,
            LayerId(0),
            SpaceId::Model,
            "AcDbSpline",
            0,
        ),
        line_entity(2, LayerId(0), line_geometry(0.0, 4.0, -4.0, 4.0), 1),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    let error = join(&mut app, &mut session).expect_err("a closed spline cannot be joined");
    let message = format!("{error:?}");
    assert!(message.contains("join.closed_object"), "{message}");
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}

// --- The open welds are unchanged ------------------------------------------

#[test]
fn an_open_mixed_weld_stays_open() {
    // The same LINE + SPLINE chain without `Close` keeps the exact weld and the
    // open flag, so the closed path did not make every weld closed.
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        spline_entity(
            2,
            open_cubic(
                point(4.0, 0.0),
                point(7.0, 2.0),
                point(9.0, 2.0),
                point(10.0, 0.0),
            ),
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    join_with(&mut app, &mut session, false, JoinMode::Multiple).expect("an open chain welds");
    let (control_points, closed) = welded(&app);
    assert!(!closed, "an open weld is not marked closed");
    assert_eq!(control_points.first().copied(), Some(point(0.0, 0.0)));
    assert_eq!(control_points.last().copied(), Some(point(10.0, 0.0)));
}
