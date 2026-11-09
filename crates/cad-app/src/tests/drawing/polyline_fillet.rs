//! `FILLET POLYLINE` / `CHAMFER POLYLINE` contracts: parity, all-junction rewrite,
//! closed/open-closing polylines, arc removal, too-short skip, trim-off joins,
//! intersect-only forms and every named refusal.

use super::*;
use crate::app_drawing::{
    plan_polyline_corners, polyline_corner_tolerance, two_line_fillet_plan, PolylineCornerSpec,
};
use cad_geometry::bulge_arc_geometry;
use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

fn polyline_geometry(points: Vec<Point3>, bulges: Vec<f64>, closed: bool) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    }
}

/// A drawing with layer 0, a second layer `5`, the polyline under test (entity 1)
/// and a LINE (entity 2) for the wrong-kind refusal.
fn polyline_app(
    points: Vec<Point3>,
    bulges: Vec<f64>,
    closed: bool,
) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(5), "CORNER")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    let mut polyline = ent(1, polyline_geometry(points, bulges, closed), 0);
    polyline.object.type_key = "AcDbPolyline".into();
    b.insert_entity(polyline).unwrap();
    b.insert_entity(ent(
        2,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        1,
    ))
    .unwrap();
    let mut other = ent(
        3,
        polyline_geometry(
            vec![point(0.0, 0.0), point(5.0, 0.0), point(5.0, 5.0)],
            vec![0.0, 0.0, 0.0],
            false,
        ),
        2,
    );
    other.object.type_key = "AcDbPolyline".into();
    other.layer = LayerId(5);
    b.insert_entity(other).unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([41u8; 32]),
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

/// An L-shaped open polyline with two interior corners:
/// `(0,0) → (20,0) → (20,20) → (40,20)`.
fn l_polyline_app() -> (Application, SessionState) {
    polyline_app(
        vec![
            point(0.0, 0.0),
            point(20.0, 0.0),
            point(20.0, 20.0),
            point(40.0, 20.0),
        ],
        vec![0.0; 4],
        false,
    )
}

fn set_number(session: &mut SessionState, parameter: ToolParameter, value: f64) {
    session
        .set_tool_parameter(parameter, ToolValue::Number(value))
        .unwrap_or_else(|error| panic!("{parameter:?} = {value} refused: {error}"));
}

fn select(app: &mut Application, session: &mut SessionState, entities: &[u128]) {
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(entities.iter().copied().map(ref_of).collect()),
    )
    .unwrap();
}

fn stored_polyline(app: &Application, entity: u128) -> (Vec<Point3>, Vec<f64>, bool) {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .unwrap_or_else(|| panic!("entity {entity} must exist"))
        .geometry
    {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => (points.clone(), bulges.clone(), *closed),
        other => panic!("expected an LWPOLYLINE, got {other:?}"),
    }
}

fn stored_entity(app: &Application, entity: u128) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .unwrap_or_else(|| panic!("entity {entity} must exist"))
        .clone()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn assert_close(actual: Point3, expected: Point3, label: &str) {
    assert!(
        (actual.x - expected.x).abs() < 1e-9
            && (actual.y - expected.y).abs() < 1e-9
            && (actual.z - expected.z).abs() < 1e-9,
        "{label}: expected {expected:?}, got {actual:?}"
    );
}

fn assert_points(actual: &[Point3], expected: &[Point3]) {
    assert_eq!(actual.len(), expected.len(), "vertex count: {actual:?}");
    for (index, (a, b)) in actual.iter().zip(expected).enumerate() {
        assert_close(*a, *b, &format!("vertex {index}"));
    }
}

/// The runtime tolerance the corner construction uses.
fn tol() -> f64 {
    polyline_corner_tolerance()
}

#[test]
fn polyline_fillet_corner_matches_the_two_line_plan() {
    let radius = 2.0;
    let points = vec![point(10.0, 0.0), point(0.0, 0.0), point(0.0, 10.0)];
    let bulges = vec![0.0; 3];
    let plan = plan_polyline_corners(
        &points,
        &bulges,
        false,
        PolylineCornerSpec::Fillet { radius, trim: true },
        tol(),
    )
    .expect("the 90° corner is planable");
    let out_points = &plan.points;
    assert_eq!(out_points.len(), 4, "one corner adds exactly one vertex");
    assert_close(out_points[0], point(10.0, 0.0), "the start vertex");
    assert_close(out_points[1], point(2.0, 0.0), "t1");
    assert_close(out_points[2], point(0.0, 2.0), "t2");
    assert_close(out_points[3], point(0.0, 10.0), "the end vertex");

    let recovered = bulge_arc_geometry(
        out_points[1],
        out_points[2],
        plan.bulges[1],
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    )
    .expect("a non-zero bulge is an arc");
    let SemanticGeometry::Arc {
        center,
        radius: recovered_radius,
        sweep,
        ..
    } = recovered
    else {
        panic!("expected an ARC, got {recovered:?}");
    };

    let (first, second, inserted) = two_line_fillet_plan(
        (point(0.0, 0.0), point(10.0, 0.0)),
        point(8.0, 0.0),
        (point(0.0, 0.0), point(0.0, 10.0)),
        point(0.0, 8.0),
        radius,
        tol(),
    )
    .expect("the two-line plan is valid");
    let Some(SemanticGeometry::Arc {
        center: reference_center,
        radius: reference_radius,
        sweep: reference_sweep,
        ..
    }) = inserted
    else {
        panic!("the two-line FILLET must insert an ARC");
    };
    assert!(
        (center.x - reference_center.x).abs() < 1e-12
            && (center.y - reference_center.y).abs() < 1e-12,
        "centre {center:?} vs {reference_center:?}"
    );
    assert!(
        (recovered_radius - reference_radius).abs() < 1e-12,
        "radius {recovered_radius} vs {reference_radius}"
    );
    assert!(
        (sweep - reference_sweep).abs() < 1e-12,
        "sweep {sweep} vs {reference_sweep}"
    );
    let touched = |pair: (Point3, Point3), far: Point3| {
        let is_far = |p: Point3| (p.x - far.x).abs() < 1e-12 && (p.y - far.y).abs() < 1e-12;
        if is_far(pair.0) {
            pair.1
        } else {
            pair.0
        }
    };
    assert_close(out_points[1], touched(first, point(10.0, 0.0)), "parity t1");
    assert_close(
        out_points[2],
        touched(second, point(0.0, 10.0)),
        "parity t2",
    );
    assert_close(center, point(2.0, 2.0), "the fillet centre");
    assert!(sweep < 0.0, "a right turn takes the negative sweep");
}

#[test]
fn polyline_fillet_rounds_every_interior_corner_in_one_undo_step() {
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 5.0);
    select(&mut app, &mut session, &[1]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        CommandPayload::None,
    )
    .expect("FILLET POLYLINE runs on the open two-corner polyline");

    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 1);
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(outcome.diagnostics[0].code, "drawing.fillet_polyline");

    // The two end segments are parallel (both horizontal), so no start/end
    // closure exists; 4 vertices with 2 interior corners ⇒ 6 vertices.
    let (points, bulges, closed) = stored_polyline(&app, 1);
    assert!(!closed, "parallel end segments cannot close the polyline");
    assert_eq!(bulges.len(), points.len(), "one bulge per vertex");
    assert_points(
        &points,
        &[
            point(0.0, 0.0),
            point(15.0, 0.0),
            point(20.0, 5.0),
            point(20.0, 15.0),
            point(25.0, 20.0),
            point(40.0, 20.0),
        ],
    );
    assert!(bulges[1] > 0.0, "a left turn takes a positive bulge");
    assert!(bulges[3] < 0.0, "a right turn takes a negative bulge");
    let normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    for (segment, centre) in [(1usize, point(15.0, 5.0)), (3usize, point(25.0, 15.0))] {
        let recovered = bulge_arc_geometry(
            points[segment],
            points[segment + 1],
            bulges[segment],
            normal,
        )
        .expect("a non-zero bulge is an arc");
        let SemanticGeometry::Arc {
            center,
            radius,
            sweep,
            ..
        } = recovered
        else {
            panic!("expected an ARC, got {recovered:?}");
        };
        assert_close(center, centre, "arc centre");
        assert!((radius - 5.0).abs() < 1e-9, "arc radius {radius}");
        assert!((sweep.abs() - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    }
    for index in [0usize, 2, 4] {
        assert_eq!(bulges[index], 0.0, "segment {index} stays straight");
    }

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let (restored, restored_bulges, restored_closed) = stored_polyline(&app, 1);
    assert!(!restored_closed);
    assert_points(
        &restored,
        &[
            point(0.0, 0.0),
            point(20.0, 0.0),
            point(20.0, 20.0),
            point(40.0, 20.0),
        ],
    );
    assert_eq!(restored_bulges, vec![0.0; 4]);
}

#[test]
fn polyline_chamfer_splits_a_on_the_incoming_and_b_on_the_outgoing_arm() {
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 2.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 3.0);
    select(&mut app, &mut session, &[1]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ChamferPolyline,
        CommandPayload::None,
    )
    .expect("CHAMFER POLYLINE runs on the open two-corner polyline");
    assert_eq!(outcome.diagnostics[0].code, "drawing.chamfer_polyline");
    assert_eq!(undo_depth(&app), 1);

    let (points, bulges, closed) = stored_polyline(&app, 1);
    assert!(!closed);
    assert_points(
        &points,
        &[
            point(0.0, 0.0),
            point(18.0, 0.0),
            point(20.0, 3.0),
            point(20.0, 18.0),
            point(23.0, 20.0),
            point(40.0, 20.0),
        ],
    );
    assert_close(points[1], point(18.0, 0.0), "A on the incoming arm");
    for index in [1usize, 3] {
        assert_eq!(
            bulges[index], 0.0,
            "a chamfer joins with a straight segment"
        );
    }
}

#[test]
fn polyline_fillet_supports_a_closed_polyline() {
    let (mut app, mut session) = polyline_app(
        vec![
            point(0.0, 0.0),
            point(20.0, 0.0),
            point(20.0, 20.0),
            point(0.0, 20.0),
        ],
        vec![0.0; 4],
        true,
    );
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    select(&mut app, &mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        CommandPayload::None,
    )
    .expect("a closed polyline rounds every wrap-around corner");
    let (points, bulges, closed) = stored_polyline(&app, 1);
    assert!(closed, "the rewrite keeps the polyline closed");
    assert_eq!(points.len(), 8, "four corners add one vertex each");
    assert_eq!(bulges.len(), points.len());
    assert_points(
        &points,
        &[
            point(2.0, 0.0),
            point(18.0, 0.0),
            point(20.0, 2.0),
            point(20.0, 18.0),
            point(18.0, 20.0),
            point(2.0, 20.0),
            point(0.0, 18.0),
            point(0.0, 2.0),
        ],
    );
}

#[test]
fn polyline_fillet_closes_an_open_polyline_at_its_start_and_end_segments() {
    // The last segment ends short of the corner the two end segments' lines
    // define, and the radius fits, so the polyline closes with 4 corners.
    let (mut app, mut session) = polyline_app(
        vec![
            point(0.0, 0.0),
            point(20.0, 0.0),
            point(20.0, 20.0),
            point(0.0, 20.0),
            point(0.0, 3.0),
        ],
        vec![0.0; 5],
        false,
    );
    set_number(&mut session, ToolParameter::FilletRadius, 5.0);
    select(&mut app, &mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        CommandPayload::None,
    )
    .expect("the start/end segments are filleted and the polyline closes");
    let (points, bulges, closed) = stored_polyline(&app, 1);
    assert!(
        closed,
        "the open polyline is closed by the start/end corner"
    );
    assert_eq!(points.len(), 8);
    assert_eq!(bulges.len(), points.len());
    assert_points(
        &points,
        &[
            point(5.0, 0.0),
            point(15.0, 0.0),
            point(20.0, 5.0),
            point(20.0, 15.0),
            point(15.0, 20.0),
            point(5.0, 20.0),
            point(0.0, 15.0),
            point(0.0, 5.0),
        ],
    );
    // The closing segment is an arc, not a straight line.
    assert!(bulges[7].abs() > 0.0, "the closing corner is a fillet arc");
}

#[test]
fn polyline_fillet_removes_a_single_intervening_arc_segment() {
    // Segments: straight → arc → straight. The arc's two vertices are replaced
    // by the corner of the two straight lines.
    let (mut app, mut session) = polyline_app(
        vec![
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(10.0, -5.0),
            point(10.0, 20.0),
        ],
        vec![0.0, 0.8, 0.0, -0.5],
        false,
    );
    set_number(&mut session, ToolParameter::ChamferDistanceA, 2.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 2.0);
    select(&mut app, &mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::ChamferPolyline,
        CommandPayload::None,
    )
    .expect("the intervening arc is replaced by the two straight lines' corner");
    let (points, bulges, closed) = stored_polyline(&app, 1);
    assert!(!closed);
    assert_points(
        &points,
        &[
            point(0.0, 0.0),
            point(8.0, 0.0),
            point(10.0, 2.0),
            point(10.0, 20.0),
        ],
    );
    assert_eq!(bulges, vec![0.0; 4], "the removed arc leaves no bulge");
}

#[test]
fn polyline_zero_radius_and_zero_distances_are_the_intersect_only_form() {
    // FILLET R=0: trim/extend the straight segments to their corner, remove the
    // intervening arc, create no joining segment.
    for (id, radius, a, b) in [
        (CommandId::FilletPolyline, 0.0, 0.0, 0.0),
        (CommandId::ChamferPolyline, 0.0, 0.0, 0.0),
    ] {
        let (mut app, mut session) = polyline_app(
            vec![
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(10.0, -5.0),
                point(10.0, 20.0),
            ],
            vec![0.0, 0.8, 0.0, -0.5],
            false,
        );
        set_number(&mut session, ToolParameter::FilletRadius, radius);
        set_number(&mut session, ToolParameter::ChamferDistanceA, a);
        set_number(&mut session, ToolParameter::ChamferDistanceB, b);
        select(&mut app, &mut session, &[1]);
        execute(&mut app, &mut session, id, CommandPayload::None)
            .unwrap_or_else(|error| panic!("{id:?}: {error}"));
        let (points, bulges, closed) = stored_polyline(&app, 1);
        assert!(!closed);
        assert_points(
            &points,
            &[point(0.0, 0.0), point(10.0, 0.0), point(10.0, 20.0)],
        );
        assert_eq!(bulges, vec![0.0; 3], "no joining segment is created");
    }
}

#[test]
fn polyline_fillet_skips_a_corner_that_does_not_fit_its_segment() {
    // Segment 1 is 3 long; two R=2 corners would need 4. The greedy first
    // corner wins and the second junction is left unmodified (no refusal).
    let (mut app, mut session) = polyline_app(
        vec![
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(10.0, 3.0),
            point(30.0, 3.0),
        ],
        vec![0.0; 4],
        false,
    );
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    select(&mut app, &mut session, &[1]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        CommandPayload::None,
    )
    .expect("a too-short segment is skipped, not refused");
    assert!(
        outcome.diagnostics[0].message.contains("1 skipped"),
        "{}",
        outcome.diagnostics[0].message
    );
    let (points, bulges, _) = stored_polyline(&app, 1);
    assert_points(
        &points,
        &[
            point(0.0, 0.0),
            point(8.0, 0.0),
            point(10.0, 2.0),
            point(10.0, 3.0),
            point(30.0, 3.0),
        ],
    );
    assert!(bulges[1].abs() > 0.0, "the fitted corner is an arc");
    assert_eq!(
        bulges[2], 0.0,
        "the skipped corner keeps its straight vertex"
    );
}

#[test]
fn polyline_trim_off_inserts_joining_entities_and_leaves_the_polyline() {
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 5.0);
    session.drafting_extras.corner_modes_mut().set_trim(false);
    let before = stored_entity(&app, 1);
    select(&mut app, &mut session, &[1]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        CommandPayload::None,
    )
    .expect("No Trim inserts the two fillet arcs");
    assert_eq!(
        stored_entity(&app, 1).geometry,
        before.geometry,
        "the polyline is not trimmed"
    );
    assert_eq!(outcome.objects.len(), 2, "one arc per interior corner");
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 2);
    assert_eq!(undo_depth(&app), 1);
    for object in &outcome.objects {
        let entity = stored_entity(&app, object.0);
        assert!(matches!(entity.geometry, SemanticGeometry::Arc { .. }));
        assert_eq!(entity.space, SpaceId::Model);
    }
}

#[test]
fn polyline_chamfer_angle_method_derives_the_outgoing_distance_per_vertex() {
    let (mut app, mut session) = l_polyline_app();
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
    select(&mut app, &mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::ChamferPolyline,
        CommandPayload::None,
    )
    .expect("the angle method applies per vertex");
    let (points, _, _) = stored_polyline(&app, 1);
    let b = 2.0 * (30.0f64).to_radians().tan(); // 90° corners: b = 2·tan(30°)
    assert_points(
        &points,
        &[
            point(0.0, 0.0),
            point(18.0, 0.0),
            point(20.0, b),
            point(20.0, 20.0 - 2.0),
            point(20.0 + b, 20.0),
            point(40.0, 20.0),
        ],
    );
}

#[test]
fn polyline_fillet_and_chamfer_refuse_every_named_boundary() {
    // Not an LWPOLYLINE (the fixture's LINE, entity 2).
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    select(&mut app, &mut session, &[2]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_not_polyline",
        true,
    );

    // Two consecutive arc segments.
    let (mut app, mut session) = polyline_app(
        vec![
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(10.0, 10.0),
            point(20.0, 10.0),
        ],
        vec![0.0, 0.5, 0.5, 0.0],
        false,
    );
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_arc_chain",
        true,
    );

    // A 3D (non-`+Z` planar) polyline.
    let (mut app, mut session) = polyline_app(
        vec![
            point(0.0, 0.0),
            point(20.0, 0.0),
            Point3 {
                x: 20.0,
                y: 20.0,
                z: 5.0,
            },
        ],
        vec![0.0; 3],
        false,
    );
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_not_planar",
        true,
    );

    // Fewer than three vertices.
    let (mut app, mut session) =
        polyline_app(vec![point(0.0, 0.0), point(20.0, 0.0)], vec![0.0; 2], false);
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_too_few_vertices",
        false,
    );

    // Duplicate consecutive vertices.
    let (mut app, mut session) = polyline_app(
        vec![
            point(0.0, 0.0),
            point(20.0, 0.0),
            point(20.0, 0.0),
            point(20.0, 20.0),
        ],
        vec![0.0; 4],
        false,
    );
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_degenerate",
        false,
    );

    // Negative radius / distance (the typed channel refuses them on write, so
    // the test-only unvalidated setter is the only way to reach the guard).
    let (mut app, mut session) = l_polyline_app();
    session.force_tool_parameter(ToolParameter::FilletRadius, ToolValue::Number(-1.0));
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_radius_negative",
        false,
    );
    let (mut app, mut session) = l_polyline_app();
    session.force_tool_parameter(ToolParameter::ChamferDistanceA, ToolValue::Number(-1.0));
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferPolyline,
        "chamfer.polyline_distance_negative",
        false,
    );

    // A parameter that holds no finite number.
    let (mut app, mut session) = l_polyline_app();
    session.force_tool_parameter(ToolParameter::FilletRadius, ToolValue::Flag(true));
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_parameter_missing",
        false,
    );

    // Nothing to modify: every junction is collinear or parallel.
    let (mut app, mut session) = polyline_app(
        vec![point(0.0, 0.0), point(10.0, 0.0), point(20.0, 0.0)],
        vec![0.0; 3],
        false,
    );
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_nothing_to_modify",
        false,
    );
    // Both polyline commands are Work-only.
    assert!(CommandId::FilletPolyline.requires_work_mode());
    assert!(CommandId::ChamferPolyline.requires_work_mode());
}

#[test]
fn polyline_fillet_and_chamfer_refuse_a_bad_selection() {
    // Nothing selected.
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_selection",
        false,
    );
    // Two selected.
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    select(&mut app, &mut session, &[1, 3]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_selection",
        false,
    );
    // A payload is refused: the selection is the whole input.
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    select(&mut app, &mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("fillet.polyline_invalid_payload"),
        "{error}"
    );

    // Another document's ref, a block-instance member, a sub-element and a
    // missing entity.
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    for (reference, unsupported) in [
        (
            SelectionRef {
                document: DocumentId(2),
                entity: EntityId(1),
                instance: InstancePath::default(),
                sub_element: None,
            },
            false,
        ),
        (
            SelectionRef {
                document: DocumentId(1),
                entity: EntityId(1),
                instance: InstancePath(vec![EntityId(1)]),
                sub_element: None,
            },
            true,
        ),
        (
            SelectionRef {
                document: DocumentId(1),
                entity: EntityId(1),
                instance: InstancePath::default(),
                sub_element: Some(SubElementId {
                    source_key: "edge:0".into(),
                    topology_revision: Revision(0),
                }),
            },
            true,
        ),
        (
            SelectionRef {
                document: DocumentId(1),
                entity: EntityId(99),
                instance: InstancePath::default(),
                sub_element: None,
            },
            false,
        ),
    ] {
        let expected = if reference.entity == EntityId(99) {
            "fillet.polyline_target_missing"
        } else {
            "fillet.polyline_not_addressable"
        };
        session.selection.replace(vec![reference]);
        assert_refused(
            &mut app,
            &mut session,
            CommandId::FilletPolyline,
            expected,
            unsupported,
        );
    }
}

/// Run a command that must be refused with **no** state change at all.
fn assert_refused(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    expected_code: &str,
    unsupported: bool,
) {
    let rev = revision(app);
    let depth = undo_depth(app);
    let before = stored_entity(app, 1);
    let error = execute(app, session, id, CommandPayload::None).unwrap_err();
    let text = error.to_string();
    assert!(text.contains(expected_code), "{text}");
    if unsupported {
        assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    } else {
        assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    }
    assert_eq!(revision(app), rev, "a refused edit changes nothing");
    assert_eq!(
        undo_depth(app),
        depth,
        "a refused edit records no undo step"
    );
    let after = stored_entity(app, 1);
    assert_eq!(after.geometry, before.geometry);
    assert_eq!(after.object.revision, before.object.revision);
}

#[test]
fn a_rounded_polyline_round_trips_through_the_dxf_export() {
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 5.0);
    select(&mut app, &mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        CommandPayload::None,
    )
    .expect("FILLET POLYLINE runs");
    let (expected_points, expected_bulges, _) = stored_polyline(&app, 1);

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
        "the LWPOLYLINE is written exactly"
    );
    assert_eq!(report.counts.dropped, 0);

    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let imported = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("re-import succeeds");
    let polylines: Vec<(Vec<Point3>, Vec<f64>)> = imported
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Polyline { points, bulges, .. } => {
                Some((points.clone(), bulges.clone()))
            }
            _ => None,
        })
        .collect();
    let matching: Vec<&(Vec<Point3>, Vec<f64>)> = polylines
        .iter()
        .filter(|(points, _)| points.len() == expected_points.len())
        .collect();
    let (points, bulges) = matching[0];
    assert_eq!(matching.len(), 1);
    assert_points(points, &expected_points);
    assert_eq!(bulges.len(), expected_bulges.len());
    for (a, b) in bulges.iter().zip(&expected_bulges) {
        assert!(
            (a - b).abs() < 1e-9,
            "a preserved bulge differs: {a} vs {b}"
        );
    }
}
