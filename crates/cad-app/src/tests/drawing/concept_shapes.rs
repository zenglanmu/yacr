//! SketchUp construction shapes: `RECTANGROTATED` and `PIE`.
//!
//! Both commands are kernel-free: they build one closed straight `AcDbPolyline`
//! from the three captured points. These contracts pin the exact corners, the
//! sampled arc, the one-transaction insert and every named refusal.

use super::*;
use crate::tests::inquiry::app_from_builder;
use cad_db::{DrawingDatabaseBuilder, Layer};

fn app() -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    let (app, session, _) = app_from_builder(builder);
    (app, session)
}

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    id: ExtendedCommandId,
    points: Vec<Point3>,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Extended(id), CommandPayload::Points(points)),
    )
}

fn refusal_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

/// The one created row's closed polyline points.
fn created_points(app: &Application, outcome: &CommandOutcome) -> (Vec<Point3>, bool) {
    assert_eq!(outcome.objects.len(), 1);
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .expect("the created row exists");
    match &entity.geometry {
        SemanticGeometry::Polyline { points, closed, .. } => (points.clone(), *closed),
        other => panic!("expected a polyline, got {other:?}"),
    }
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn a_rotated_rectangle_uses_the_first_edge_and_the_width_point() {
    let (mut app, mut session) = app();
    let outcome = run(
        &mut app,
        &mut session,
        ExtendedCommandId::RectangRotated,
        vec![p3(0.0, 0.0, 0.0), p3(3.0, 0.0, 0.0), p3(0.0, 2.0, 0.0)],
    )
    .expect("the rectangle is created");
    let (points, closed) = created_points(&app, &outcome);
    assert!(closed);
    assert_eq!(
        points,
        vec![
            p3(0.0, 0.0, 0.0),
            p3(3.0, 0.0, 0.0),
            p3(3.0, 2.0, 0.0),
            p3(0.0, 2.0, 0.0),
        ]
    );
}

#[test]
fn a_rotated_rectangle_lies_on_the_plane_its_three_clicks_span() {
    let (mut app, mut session) = app();
    let outcome = run(
        &mut app,
        &mut session,
        ExtendedCommandId::RectangRotated,
        vec![p3(0.0, 0.0, 0.0), p3(0.0, 3.0, 0.0), p3(0.0, 0.0, 2.0)],
    )
    .expect("the vertical rectangle is created");
    let (points, _) = created_points(&app, &outcome);
    assert_eq!(
        points,
        vec![
            p3(0.0, 0.0, 0.0),
            p3(0.0, 3.0, 0.0),
            p3(0.0, 3.0, 2.0),
            p3(0.0, 0.0, 2.0),
        ],
        "the width runs along the plane normal's perpendicular, not the world Z"
    );
}

#[test]
fn a_rotated_rectangle_refuses_degenerate_clicks_by_name() {
    let (mut app, mut session) = app();
    let cases = [
        (
            "zero edge",
            vec![p3(1.0, 1.0, 0.0), p3(1.0, 1.0, 0.0), p3(0.0, 2.0, 0.0)],
        ),
        (
            "collinear",
            vec![p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0), p3(2.0, 0.0, 0.0)],
        ),
        (
            "zero width",
            vec![p3(0.0, 0.0, 0.0), p3(2.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)],
        ),
    ];
    for (label, points) in cases {
        let before = revision(&app);
        let text = refusal_text(
            run(
                &mut app,
                &mut session,
                ExtendedCommandId::RectangRotated,
                points,
            )
            .unwrap_err(),
        );
        let text = text
            .strip_prefix("rectangrotated.degenerate:")
            .unwrap_or_else(|| panic!("{label}: expected rectangrotated.degenerate, got {text}"));
        assert!(!text.is_empty(), "{label}");
        assert_eq!(revision(&app), before, "{label}: no write");
    }
    let text = refusal_text(
        run(
            &mut app,
            &mut session,
            ExtendedCommandId::RectangRotated,
            vec![p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)],
        )
        .unwrap_err(),
    );
    assert!(
        text.starts_with("rectangrotated.requires_points:"),
        "{text}"
    );
}

#[test]
fn a_pie_is_a_closed_centre_plus_sampled_arc() {
    let (mut app, mut session) = app();
    let outcome = run(
        &mut app,
        &mut session,
        ExtendedCommandId::Pie,
        vec![p3(0.0, 0.0, 0.0), p3(2.0, 0.0, 0.0), p3(0.0, 2.0, 0.0)],
    )
    .expect("the pie is created");
    let (points, closed) = created_points(&app, &outcome);
    assert!(closed);
    // A quarter turn samples ceil(0.25 · 24) = 6 segments: the centre plus seven
    // arc points.
    assert_eq!(points.len(), 8);
    assert_eq!(
        points[0],
        p3(0.0, 0.0, 0.0),
        "the centre is the first vertex"
    );
    assert_eq!(
        points[1],
        p3(2.0, 0.0, 0.0),
        "the arc starts at the radius click"
    );
    let last = *points.last().unwrap();
    assert!((last.x).abs() < 1e-9 && (last.y - 2.0).abs() < 1e-9 && last.z.abs() < 1e-9);
    // Shoelace area of the sampled quarter disc: ≈ π for r = 2 (within the chord
    // approximation), and strictly less than the exact disc.
    let area = 0.5
        * points
            .iter()
            .enumerate()
            .map(|(index, a)| {
                let b = points[(index + 1) % points.len()];
                a.x * b.y - b.x * a.y
            })
            .sum::<f64>();
    assert!(
        (area - std::f64::consts::PI).abs() < 0.05,
        "sampled quarter-disc area {area}"
    );
    assert!(area < std::f64::consts::PI, "chords under-cut the disc");
}

#[test]
fn a_pie_refuses_degenerate_clicks_by_name() {
    let (mut app, mut session) = app();
    let cases = [
        (
            "zero radius",
            vec![p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 0.0), p3(0.0, 1.0, 0.0)],
        ),
        (
            "zero sweep",
            vec![p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0), p3(2.0, 0.0, 0.0)],
        ),
    ];
    for (label, points) in cases {
        let before = revision(&app);
        let text =
            refusal_text(run(&mut app, &mut session, ExtendedCommandId::Pie, points).unwrap_err());
        assert!(
            text.starts_with("pie.degenerate:"),
            "{label}: expected pie.degenerate, got {text}"
        );
        assert_eq!(revision(&app), before, "{label}: no write");
    }
    let text = refusal_text(
        run(
            &mut app,
            &mut session,
            ExtendedCommandId::Pie,
            vec![p3(0.0, 0.0, 0.0)],
        )
        .unwrap_err(),
    );
    assert!(text.starts_with("pie.requires_points:"), "{text}");
}

#[test]
fn both_shapes_are_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = app();
    run(
        &mut app,
        &mut session,
        ExtendedCommandId::RectangRotated,
        vec![p3(0.0, 0.0, 0.0), p3(2.0, 0.0, 0.0), p3(0.0, 1.0, 0.0)],
    )
    .expect("the rectangle is created");
    run(
        &mut app,
        &mut session,
        ExtendedCommandId::Pie,
        vec![p3(5.0, 0.0, 0.0), p3(6.0, 0.0, 0.0), p3(5.0, 1.0, 0.0)],
    )
    .expect("the pie is created");
    assert_eq!(revision(&app), Revision(2), "one bump per command");
    assert_eq!(
        app.history[&DocumentId(1)].undo_depth(),
        2,
        "one undo entry per command"
    );
}
