//! Bulged (arc-segment) LWPOLYLINE `OFFSET` contract tests.
//!
//! The arc arm of OFFSET (`app_drawing::offset_polyline_bulge`): a bulged segment
//! is offset **exactly** — a concentric arc of `R ± d`, never a sampled or
//! polyline-approximated one — and the joins between consecutive offset segments
//! are the analytic line/line miter, the line/circle and the circle/circle
//! intersections, each taken at the point **nearest the source vertex**.
//!
//! The pins:
//!
//! * outward and inward offsets keep the arc's centre, change its radius by
//!   exactly the picked distance and keep its sweep (a tangential/untouched join is
//!   a pure trim, so the sweep is unchanged);
//! * a line/arc join is the exact analytic intersection (within 1e-12 of the closed
//!   form) and both pieces are trimmed to it;
//! * arc/arc joins work both tangent-continuously (one point) and at an angle (two
//!   candidates, the nearer one wins);
//! * a closed ring joins its wrap corner too and stays closed;
//! * a clockwise ring (negative sweeps, `+Z` bulge frame) produces the same
//!   geometry as its counter-clockwise twin — the signs are never mirrored;
//! * one transaction / one undo step, source untouched;
//! * every refusal is named and changes nothing (revision, undo depth, entity
//!   count);
//! * the offset result survives an export → re-import with its points and bulges.

use super::offset::app_with_drawing;
use super::offset_polyline::{created_polyline, polyline_app, undo_depth};
use super::*;
use cad_geometry::bulge_arc_geometry;
use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

/// The `+Z` normal the DXF bulge convention uses for 2D data.
const Z: Point3 = Point3 {
    x: 0.0,
    y: 0.0,
    z: 1.0,
};

/// The bulge of a **quarter** circle arc (`sweep = +π/2`).
fn quarter_bulge() -> f64 {
    (std::f64::consts::PI / 8.0).tan()
}

/// The bulge of a **60°** arc.
fn sixth_bulge() -> f64 {
    (std::f64::consts::PI / 12.0).tan()
}

/// One completed OFFSET, kept together so a test can assert the stored geometry,
/// the source and the history in one place.
struct Offset {
    app: Application,
    points: Vec<Point3>,
    bulges: Vec<f64>,
    closed: bool,
    outcome: CommandOutcome,
}

/// Run `OffsetEntity` on the single polyline of a fresh drawing.
fn run(points: Vec<Point3>, bulges: Vec<f64>, closed: bool, side: Point3) -> Offset {
    let mut app = polyline_app(points, bulges, closed);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: side,
        },
    )
    .expect("OFFSET runs");
    let (points, bulges, closed) = created_polyline(&app, &outcome);
    Offset {
        app,
        points,
        bulges,
        closed,
        outcome,
    }
}

/// Run `OffsetEntity` and keep the refusal plus the untouched application.
fn run_err(
    points: Vec<Point3>,
    bulges: Vec<f64>,
    closed: bool,
    side: Point3,
) -> (CadError, Application) {
    let mut app = polyline_app(points, bulges, closed);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: side,
        },
    )
    .expect_err("OFFSET must refuse");
    (error, app)
}

/// A refusal must write nothing: same revision, same entity count, no undo step.
fn assert_untouched(app: &Application, revision_before: Revision) {
    assert_eq!(revision(app), revision_before, "no database write");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        1,
        "nothing may be created"
    );
    assert_eq!(undo_depth(app), 0, "zero undo steps");
}

/// The refusal string of an `Unsupported` refusal naming `code`.
fn assert_refusal(error: &CadError, code: &str) {
    match error {
        CadError::Unsupported(reason) => assert!(
            reason.contains(code),
            "the refusal must name {code}: {reason}"
        ),
        other => panic!("expected an Unsupported refusal, got {other:?}"),
    }
}

/// Recover segment `index` through the single `bulge → arc` authority.
fn recovered(points: &[Point3], bulges: &[f64], index: usize) -> (Point3, f64, f64) {
    let start = points[index];
    let end = points[(index + 1) % points.len()];
    match bulge_arc_geometry(start, end, bulges[index], Z).expect("the segment is an arc") {
        SemanticGeometry::Arc {
            center,
            radius,
            sweep,
            ..
        } => (center, radius, sweep),
        other => panic!("expected an arc, got {other:?}"),
    }
}

fn assert_point(actual: Point3, expected: Point3, tolerance: f64) {
    assert!(
        (actual.x - expected.x).abs() <= tolerance && (actual.y - expected.y).abs() <= tolerance,
        "expected {expected:?}, got {actual:?} (tolerance {tolerance})"
    );
}

/// The quarter arc `(10,0) → (0,10)` about the origin (R = 10, sweep +π/2)
/// followed by the **tangent-continuous** horizontal line to `(-10,10)`.
fn arc_then_tangent_line() -> (Vec<Point3>, Vec<f64>) {
    (
        vec![point(10.0, 0.0), point(0.0, 10.0), point(-10.0, 10.0)],
        vec![quarter_bulge(), 0.0],
    )
}

/// The same quarter arc followed by a **sharp** corner into the vertical line up
/// to `(0,20)` (the arc arrives along `-x`, the line leaves along `+y`).
fn arc_then_sharp_line() -> (Vec<Point3>, Vec<f64>) {
    (
        vec![point(10.0, 0.0), point(0.0, 10.0), point(0.0, 20.0)],
        vec![quarter_bulge(), 0.0],
    )
}

#[test]
fn offset_bulged_arc_and_tangent_line_outward_keeps_centre_and_sweep() {
    let (points, bulges) = arc_then_tangent_line();
    let offset = run(points, bulges, false, point(14.0, 0.0));

    assert!(!offset.closed, "an open polyline stays open");
    assert_eq!(offset.points.len(), 3, "n vertices in, n vertices out");
    // The arc: same centre, R + d = 14, and the sweep is untouched (the join is
    // the tangency, i.e. the source vertex moved along its own radius).
    let (center, radius, sweep) = recovered(&offset.points, &offset.bulges, 0);
    assert_point(center, point(0.0, 0.0), 1e-12);
    assert!((radius - 14.0).abs() < 1e-12, "R' = R + d, got {radius}");
    assert!(
        (sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-12,
        "the arc keeps its sweep, got {sweep}"
    );
    assert!(
        (offset.bulges[0] - quarter_bulge()).abs() < 1e-12,
        "the bulge (sweep sign included) is preserved: {}",
        offset.bulges[0]
    );
    // The picks the open ends: the arc start along its own radius, the line end
    // shifted parallel. The tangent join lands exactly on the tangency point.
    assert_point(offset.points[0], point(14.0, 0.0), 1e-12);
    assert_point(offset.points[1], point(0.0, 14.0), 1e-12);
    assert_point(offset.points[2], point(-10.0, 14.0), 1e-12);
    // The applied distance is reported exactly.
    assert!(
        offset.outcome.diagnostics[0].message.contains("4.0000"),
        "the diagnostic reports the exact distance: {}",
        offset.outcome.diagnostics[0].message
    );
}

#[test]
fn offset_bulged_arc_and_tangent_line_inward_shrinks_the_radius() {
    let (points, bulges) = arc_then_tangent_line();
    let offset = run(points, bulges, false, point(6.0, 0.0));

    let (center, radius, sweep) = recovered(&offset.points, &offset.bulges, 0);
    assert_point(center, point(0.0, 0.0), 1e-12);
    assert!((radius - 6.0).abs() < 1e-12, "R' = R - d, got {radius}");
    assert!(
        (sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-12,
        "the inward arc keeps its sweep, got {sweep}"
    );
    assert_point(offset.points[1], point(0.0, 6.0), 1e-12);
    assert_point(offset.points[2], point(-10.0, 6.0), 1e-12);
}

#[test]
fn offset_bulged_line_arc_join_is_the_analytic_intersection() {
    // The offset line is x = 4 (the source line x = 0 shifted by `s·m`), so the
    // join with the concentric circle of radius 14 is (4, +sqrt(14² - 4²)) — the
    // nearer of the two roots to the source vertex (0, 10).
    let (points, bulges) = arc_then_sharp_line();
    let offset = run(points, bulges, false, point(14.0, 0.0));

    let expected = Point3 {
        x: 4.0,
        y: 180.0_f64.sqrt(),
        z: 0.0,
    };
    assert_point(offset.points[1], expected, 1e-12);

    // Both pieces are trimmed: the arc ends at the join (a shorter sweep than
    // π/2, not a full quarter) and the line starts there.
    let (center, radius, sweep) = recovered(&offset.points, &offset.bulges, 0);
    assert_point(center, point(0.0, 0.0), 1e-12);
    assert!((radius - 14.0).abs() < 1e-12);
    let expected_sweep = expected.y.atan2(expected.x);
    assert!(
        (sweep - expected_sweep).abs() < 1e-12,
        "the arc is trimmed to the join: {sweep} vs {expected_sweep}"
    );
    assert!(
        sweep < std::f64::consts::FRAC_PI_2,
        "the inward-trimmed sweep is shorter than the source sweep"
    );
    // The open end of the line segment: (0,20) shifted by (4,0).
    assert_point(offset.points[2], point(4.0, 20.0), 1e-12);
}

#[test]
fn offset_bulged_tangent_continuous_arcs_join_at_their_tangent_point() {
    // Two quarter arcs about (0,0) R=10 and (0,5) R=5, tangent at (0,10).
    let points = vec![point(10.0, 0.0), point(0.0, 10.0), point(-5.0, 5.0)];
    let bulges = vec![quarter_bulge(), quarter_bulge()];
    let offset = run(points, bulges, false, point(12.0, 0.0));

    // Both radii grow by exactly the picked distance and both sweeps survive: a
    // tangency is one join, never an "ambiguous" pair.
    let (first_center, first_radius, first_sweep) = recovered(&offset.points, &offset.bulges, 0);
    let (second_center, second_radius, second_sweep) = recovered(&offset.points, &offset.bulges, 1);
    assert_point(first_center, point(0.0, 0.0), 1e-12);
    assert_point(second_center, point(0.0, 5.0), 1e-12);
    assert!((first_radius - 12.0).abs() < 1e-12, "{first_radius}");
    assert!((second_radius - 7.0).abs() < 1e-12, "{second_radius}");
    assert!((first_sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    assert!((second_sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    // The internal tangency of the two offset circles is the tangent point moved
    // out onto the radius.
    assert_point(offset.points[1], point(0.0, 12.0), 1e-12);
    // The open ends move with their own segment.
    assert_point(offset.points[0], point(12.0, 0.0), 1e-12);
    assert_point(offset.points[2], point(-7.0, 5.0), 1e-12);
}

/// The two intersections of two circles (the test's own closed form, so the
/// planner is compared against the definition rather than against itself).
fn circle_circle(
    first: Point3,
    first_radius: f64,
    second: Point3,
    second_radius: f64,
) -> [Point3; 2] {
    let dx = second.x - first.x;
    let dy = second.y - first.y;
    let distance = dx.hypot(dy);
    let along = (first_radius * first_radius - second_radius * second_radius + distance * distance)
        / (2.0 * distance);
    let height = (first_radius * first_radius - along * along)
        .max(0.0)
        .sqrt();
    let dir = (dx / distance, dy / distance);
    let base = (first.x + along * dir.0, first.y + along * dir.1);
    [
        point(base.0 - height * dir.1, base.1 + height * dir.0),
        point(base.0 + height * dir.1, base.1 - height * dir.0),
    ]
}

#[test]
fn offset_bulged_arcs_meeting_at_an_angle_pick_the_nearer_join() {
    // Arc 1: the quarter arc about (0,0) R=10 from (10,0) to (0,10).
    // Arc 2: a 60° arc about (5,10) R=5 starting at (0,10) (a sharp 90° corner:
    // the incoming tangent is -x, the outgoing one is -y).
    let corner = 5.0 * (240.0_f64.to_radians()).cos();
    let points = vec![
        point(10.0, 0.0),
        point(0.0, 10.0),
        point(5.0 + corner, 10.0 + 5.0 * (240.0_f64.to_radians()).sin()),
    ];
    let bulges = vec![quarter_bulge(), sixth_bulge()];
    // The pick is 2 units outside arc 1 (and 7.2 outside arc 2).
    let offset = run(points, bulges, false, point(12.0, 0.0));

    let roots = circle_circle(point(0.0, 0.0), 12.0, point(5.0, 10.0), 7.0);
    // The source vertex is (0, 10): the second root is the nearer one.
    let source_vertex = point(0.0, 10.0);
    let (nearer, farther) =
        if distance_of(roots[0], source_vertex) < distance_of(roots[1], source_vertex) {
            (roots[0], roots[1])
        } else {
            (roots[1], roots[0])
        };
    assert!(
        distance_of(nearer, source_vertex) < distance_of(farther, source_vertex),
        "the fixture must have one strictly nearer root"
    );
    assert_point(offset.points[1], nearer, 1e-12);

    // Both arcs keep their centre and take the picked distance.
    let (first_center, first_radius, first_sweep) = recovered(&offset.points, &offset.bulges, 0);
    let (second_center, second_radius, second_sweep) = recovered(&offset.points, &offset.bulges, 1);
    assert_point(first_center, point(0.0, 0.0), 1e-12);
    assert_point(second_center, point(5.0, 10.0), 1e-12);
    assert!((first_radius - 12.0).abs() < 1e-12, "{first_radius}");
    assert!((second_radius - 7.0).abs() < 1e-12, "{second_radius}");
    // Both pieces are trimmed to the join: the sweeps are re-derived from the
    // centre angles of the join point, in each arc's own traversal direction.
    let expected_first = nearer.y.atan2(nearer.x);
    let expected_second = (nearer.y - 10.0).atan2(nearer.x - 5.0) - 240.0_f64.to_radians();
    assert!(
        (first_sweep - expected_first).abs() < 1e-12,
        "arc 1 sweep {first_sweep} vs {expected_first}"
    );
    assert!(
        (second_sweep - expected_second.abs()).abs() < 1e-12,
        "arc 2 sweep {second_sweep} vs {expected_second}"
    );
    // An outward miter *extends* both neighbours — the arc analogue of the
    // straight arm's longer segments — so both sweeps may exceed their sources.
    assert!(first_sweep > std::f64::consts::FRAC_PI_2);
    assert!(second_sweep > 60.0_f64.to_radians());
    // The open end of arc 2 stays on its own radius at the source end angle.
    let end_angle = 240.0_f64.to_radians();
    assert_point(
        offset.points[2],
        point(5.0 + 7.0 * end_angle.cos(), 10.0 + 7.0 * end_angle.sin()),
        1e-12,
    );
}

#[test]
fn offset_bulged_arcs_meeting_at_an_angle_shrink_when_offset_inward() {
    let corner = 5.0 * (240.0_f64.to_radians()).cos();
    let points = vec![
        point(10.0, 0.0),
        point(0.0, 10.0),
        point(5.0 + corner, 10.0 + 5.0 * (240.0_f64.to_radians()).sin()),
    ];
    let bulges = vec![quarter_bulge(), sixth_bulge()];
    // 1 unit inside arc 1.
    let offset = run(points, bulges, false, point(9.0, 0.0));

    let roots = circle_circle(point(0.0, 0.0), 9.0, point(5.0, 10.0), 4.0);
    let source_vertex = point(0.0, 10.0);
    let nearer = if distance_of(roots[0], source_vertex) < distance_of(roots[1], source_vertex) {
        roots[0]
    } else {
        roots[1]
    };
    assert_point(offset.points[1], nearer, 1e-12);

    let (first_center, first_radius, first_sweep) = recovered(&offset.points, &offset.bulges, 0);
    let (second_center, second_radius, second_sweep) = recovered(&offset.points, &offset.bulges, 1);
    assert_point(first_center, point(0.0, 0.0), 1e-12);
    assert_point(second_center, point(5.0, 10.0), 1e-12);
    assert!((first_radius - 9.0).abs() < 1e-12);
    assert!((second_radius - 4.0).abs() < 1e-12);
    // A concave corner trims both arcs shorter than their sources.
    assert!(
        first_sweep < std::f64::consts::FRAC_PI_2 && first_sweep > 0.0,
        "{first_sweep}"
    );
    assert!(
        second_sweep < 60.0_f64.to_radians() && second_sweep > 0.0,
        "{second_sweep}"
    );
}

fn distance_of(first: Point3, second: Point3) -> f64 {
    (first.x - second.x).hypot(first.y - second.y)
}

/// The 10 × 10 ring `(0,0) (10,0) (10,10) (0,10)` whose **bottom** segment is an
/// arc bulging 2 units outward (below the chord): centre `(5,5)`, R = 5·√2,
/// sweep +π/2.
fn bulged_ring(counter_clockwise: bool) -> (Vec<Point3>, Vec<f64>) {
    let bulge = quarter_bulge();
    if counter_clockwise {
        (
            vec![
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(10.0, 10.0),
                point(0.0, 10.0),
            ],
            vec![bulge, 0.0, 0.0, 0.0],
        )
    } else {
        // The same ring traversed clockwise: the same physical arc is now the
        // third segment, so its sweep is negative in the `+Z` bulge frame.
        (
            vec![
                point(0.0, 10.0),
                point(10.0, 10.0),
                point(10.0, 0.0),
                point(0.0, 0.0),
            ],
            vec![0.0, 0.0, -bulge, 0.0],
        )
    }
}

/// The pick that offsets the bulged ring 2 units **outward** (below the arc):
/// the arc's apex `(5, 5 − 5√2)` plus 2 units further out.
fn bulged_ring_outward_pick() -> Point3 {
    point(5.0, 5.0 - 5.0 * 2.0_f64.sqrt() - 2.0)
}

#[test]
fn offset_bulged_closed_ring_joins_the_wrap_corner_and_stays_closed() {
    let (points, bulges) = bulged_ring(true);
    let offset = run(points, bulges, true, bulged_ring_outward_pick());

    assert!(offset.closed, "a closed ring stays closed");
    assert_eq!(offset.points.len(), 4, "n vertices in, n vertices out");
    assert_eq!(offset.bulges.len(), 4);
    // The two straight/straight corners are the exact miters (vertices 2 and 3).
    assert_point(offset.points[2], point(12.0, 12.0), 1e-12);
    assert_point(offset.points[3], point(-2.0, 12.0), 1e-12);
    // The wrap corner (vertex 0) is the arc/line join, and vertex 1 is its
    // mirror: both are the intersections of the offset side line `x = ±2` with
    // the concentric arc of radius 5√2 + 2 about (5, 5).
    let radius = 5.0 * 2.0_f64.sqrt() + 2.0;
    let y = 5.0 - (radius * radius - 49.0).sqrt();
    assert_point(offset.points[0], point(-2.0, y), 1e-12);
    assert_point(offset.points[1], point(12.0, y), 1e-12);
    // The arc keeps its centre, takes the picked distance, and its sweep grows a
    // little at both convex joins (the straight arm's segments grow too).
    let (center, recovered_radius, sweep) = recovered(&offset.points, &offset.bulges, 0);
    assert_point(center, point(5.0, 5.0), 1e-12);
    assert!(
        (recovered_radius - radius).abs() < 1e-12,
        "R' = 5√2 + 2, got {recovered_radius}"
    );
    assert!(
        sweep > std::f64::consts::FRAC_PI_2 && sweep < std::f64::consts::PI,
        "the outward miter extends the arc without wrapping: {sweep}"
    );
    assert!(offset.bulges[0] > quarter_bulge());
}

#[test]
fn offset_bulged_clockwise_ring_matches_the_counter_clockwise_geometry() {
    let (ccw_points, ccw_bulges) = bulged_ring(true);
    let counter_clockwise = run(ccw_points, ccw_bulges, true, bulged_ring_outward_pick());
    let (cw_points, cw_bulges) = bulged_ring(false);
    let clockwise = run(cw_points, cw_bulges, true, bulged_ring_outward_pick());

    // The negative-sweep ring must offset to the same shape, with a negative
    // bulge: nothing is mirrored by the `+Z` bulge frame.
    assert!(clockwise.closed);
    let mut expected = vec![
        counter_clockwise.points[0],
        counter_clockwise.points[1],
        counter_clockwise.points[2],
        counter_clockwise.points[3],
    ];
    for actual in &clockwise.points {
        let index = expected
            .iter()
            .position(|candidate| distance_of(*candidate, *actual) < 1e-9)
            .unwrap_or_else(|| panic!("{actual:?} is not one of the counter-clockwise corners"));
        expected.remove(index);
    }
    assert!(expected.is_empty(), "all four corners match");
    let (center, radius, sweep) = recovered(&clockwise.points, &clockwise.bulges, 2);
    assert_point(center, point(5.0, 5.0), 1e-12);
    assert!((radius - (5.0 * 2.0_f64.sqrt() + 2.0)).abs() < 1e-12);
    assert!(
        sweep < 0.0,
        "the clockwise arc keeps its negative sweep: {sweep}"
    );
    assert!(clockwise.bulges[2] < 0.0);
}

#[test]
fn offset_bulged_polyline_is_one_transaction_and_one_undo_step() {
    let (points, bulges) = arc_then_sharp_line();
    let offset = run(points, bulges, false, point(14.0, 0.0));
    let copy = EntityId(offset.outcome.objects[0].0);
    assert_eq!(undo_depth(&offset.app), 1);
    assert_eq!(
        offset.app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        2
    );
    // The source is untouched.
    match &offset.app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .expect("the source exists")
        .geometry
    {
        SemanticGeometry::Polyline {
            points: source_points,
            bulges: source_bulges,
            closed,
        } => {
            assert_eq!(source_points.len(), 3);
            assert_eq!(source_bulges[0], quarter_bulge());
            assert!(!closed);
        }
        other => panic!("expected a polyline, got {other:?}"),
    }

    let mut app = offset.app;
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(copy)
        .is_none());
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        1,
        "one undo removes the whole bulged offset"
    );
}

#[test]
fn offset_arc_that_would_vanish_is_refused_by_name() {
    // A semicircle of radius 1 (bulge 1) plus a vertical line. The pick is 3 units
    // from the line, so the whole polyline offsets by 3 — more than the arc's
    // radius. The arc is refused, not deleted or blended into its neighbour.
    let points = vec![point(0.0, 0.0), point(2.0, 0.0), point(2.0, 10.0)];
    let bulges = vec![1.0, 0.0, 0.0];
    let ref_before = polyline_app(points.clone(), bulges.clone(), false);
    let before = revision(&ref_before);
    let (error, app) = run_err(points, bulges, false, point(-1.0, 5.0));
    assert_refusal(&error, "offset.arc_collapsed");
    assert_untouched(&app, before);
}

#[test]
fn offset_join_with_no_candidate_is_refused_by_name() {
    // Two arcs of the same centre and radius: offset concentrically they become
    // one circle, which has no join point at all.
    let points = vec![point(-5.0, 0.0), point(0.0, 5.0), point(5.0, 0.0)];
    let bulges = vec![-quarter_bulge(), -quarter_bulge()];
    let ref_before = polyline_app(points.clone(), bulges.clone(), false);
    let before = revision(&ref_before);
    let (error, app) = run_err(points, bulges, false, point(0.0, 8.0));
    assert_refusal(&error, "offset.join_failed");
    assert_untouched(&app, before);
}

#[test]
fn offset_join_with_two_equally_near_candidates_is_refused_by_name() {
    // The quarter arc (10,0)→(0,10) followed by the horizontal line to (10,10).
    // Offsetting by 0.5 puts the line at y = 9.5, which cuts the concentric
    // circle of radius 10.5 at x = ±√20: both points are exactly as near the
    // source vertex (0,10), so there is no join to pick.
    let points = vec![point(10.0, 0.0), point(0.0, 10.0), point(10.0, 10.0)];
    let bulges = vec![quarter_bulge(), 0.0];
    let ref_before = polyline_app(points.clone(), bulges.clone(), false);
    let before = revision(&ref_before);
    let (error, app) = run_err(points, bulges, false, point(8.0, 9.5));
    assert_refusal(&error, "offset.join_failed");
    assert_untouched(&app, before);
}

#[test]
fn offset_bulged_polyline_with_collinear_chords_refuses_the_plane() {
    // Two collinear chords and a bulge: the arc's plane is a free rotation about
    // the chord, so an assumed plane would be a guess.
    let points = vec![point(0.0, 0.0), point(5.0, 0.0), point(10.0, 0.0)];
    let bulges = vec![0.5, 0.0, 0.0];
    let ref_before = polyline_app(points.clone(), bulges.clone(), false);
    let before = revision(&ref_before);
    let (error, app) = run_err(points, bulges, false, point(5.0, 3.0));
    assert_refusal(&error, "offset.ambiguous_plane");
    assert_untouched(&app, before);
}

#[test]
fn offset_bulged_polyline_with_a_full_turn_bulge_is_refused_by_name() {
    // `bulge = tan(sweep/4)` reaches a full turn only as the bulge grows without
    // bound, so a huge finite bulge is the non-constructible case (a polyline
    // segment can never span a whole circle).
    let points = vec![point(10.0, 0.0), point(0.0, 10.0), point(0.0, 20.0)];
    let bulges = vec![1e300, 0.0, 0.0];
    let ref_before = polyline_app(points.clone(), bulges.clone(), false);
    let before = revision(&ref_before);
    let (error, app) = run_err(points, bulges, false, point(14.0, 0.0));
    assert_refusal(&error, "offset.bulge_segment");
    assert_untouched(&app, before);
}

#[test]
fn offset_bulged_polyline_refuses_a_vertex_off_its_own_plane() {
    // The quarter arc, then a vertex lifted out of the plane: there is no single
    // offset side for a skew vertex. (Three vertices can never be skew — a
    // triangle always has a plane — so the polyline needs a fourth.)
    let points = vec![
        point(10.0, 0.0),
        point(0.0, 10.0),
        point(-10.0, 10.0),
        p3(-10.0, 0.0, 4.0),
    ];
    let bulges = vec![quarter_bulge(), 0.0, 0.0, 0.0];
    let ref_before = polyline_app(points.clone(), bulges.clone(), false);
    let before = revision(&ref_before);
    let (error, app) = run_err(points, bulges, false, point(14.0, 0.0));
    assert_refusal(&error, "offset.non_planar");
    assert_untouched(&app, before);
}

#[test]
fn offset_bulged_polyline_result_round_trips_through_the_dxf_export() {
    let (points, bulges) = bulged_ring(true);
    let source_points = points.clone();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let mut source = ent(
        1,
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed: true,
        },
        0,
    );
    source.object.type_key = "AcDbPolyline".into();
    b.insert_entity(source).unwrap();
    let mut app = app_with_drawing(b.finish().unwrap());

    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: bulged_ring_outward_pick(),
        },
    )
    .expect("OFFSET runs");
    let (expected_points, expected_bulges, _) = created_polyline(&app, &outcome);

    let drawing = app.workspace.documents[&DocumentId(1)].drawing.clone();
    let (bytes, report) = export(&ExportRequest {
        database: &drawing,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: Some(UnitContext::drawing_units()),
    })
    .expect("export succeeds");
    assert!(
        report.counts.exact >= 1,
        "the z-constant LWPOLYLINE is written exactly (bulges preserved): {:?}",
        report.counts
    );
    assert_eq!(
        report.counts.dropped, 0,
        "nothing is silently dropped on export"
    );

    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    };
    let imported = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("re-import succeeds");
    let matching: Vec<(Vec<Point3>, Vec<f64>)> = imported
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            // The source ring re-imports too; the offset is the one whose vertices
            // are no longer the source's.
            SemanticGeometry::Polyline { points, bulges, .. }
                if points.len() == expected_points.len()
                    && points
                        .iter()
                        .zip(&source_points)
                        .any(|(imported, source)| distance_of(*imported, *source) > 1e-9) =>
            {
                Some((points.clone(), bulges.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "exactly one re-imported LWPOLYLINE is the offset copy"
    );
    let (imported_points, imported_bulges) = &matching[0];
    for (index, (actual, expected)) in imported_points.iter().zip(&expected_points).enumerate() {
        assert_point(*actual, *expected, 1e-9);
        let _ = index;
    }
    assert_eq!(imported_bulges.len(), expected_bulges.len());
    for (index, (actual, expected)) in imported_bulges.iter().zip(&expected_bulges).enumerate() {
        assert!(
            (actual - expected).abs() < 1e-9,
            "bulge {index}: {actual} vs {expected} (the arc segments keep their sweep)"
        );
    }
}
