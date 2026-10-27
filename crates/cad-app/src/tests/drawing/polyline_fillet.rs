//! `FILLET POLYLINE` / `CHAMFER POLYLINE` contracts (the AutoCAD Polyline mode of
//! the two corner commands, as selection-based selectors).
//!
//! The pins:
//!
//! * **parity** — a 90° corner of two straight segments planned through
//!   [`plan_polyline_corners`] produces the same centre, radius, sweep and tangent
//!   points as the two-line `plan_fillet` on the same arms (within 1e-12), with
//!   the arc recovered through the single `bulge → arc` authority;
//! * every interior corner of an open polyline is rewritten, the vertex count
//!   grows by exactly one per corner, the untouched geometry and the end vertices
//!   are unchanged, and the whole rewrite is one transaction / one undo step;
//! * the chamfer split points are exactly `a` from the corner on the **incoming**
//!   arm and `b` on the **outgoing** arm (our declared orientation rule);
//! * every refusal is named and changes nothing (revision, undo depth and the
//!   stored entity bytes);
//! * the modified polyline survives an export → re-import with its points and
//!   bulges.

use super::*;
use crate::app_drawing::{
    plan_polyline_corners, polyline_corner_tolerance, two_line_fillet_plan, PolylineCornerSpec,
};
use cad_geometry::bulge_arc_geometry;
use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

const STRAIGHT_BULGE_ZERO: f64 = 0.0;

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

/// The runtime tolerance the corner construction uses (the command hands the
/// planner the very same value), so the tests compare against the real predicate.
fn tol() -> f64 {
    polyline_corner_tolerance()
}

#[test]
fn polyline_fillet_corner_matches_the_two_line_plan() {
    // The 90° corner `(10,0) → (0,0) → (0,10)` is the two-line L corner with the
    // arms `+x` and `+y`, so the polyline planner must reproduce the two-line
    // plan exactly: `d = R / tan(45°) = R`, centre `(R, R)`, sweep `-π/2`.
    let radius = 2.0;
    let points = vec![point(10.0, 0.0), point(0.0, 0.0), point(0.0, 10.0)];
    let bulges = vec![0.0; 3];
    let (out_points, out_bulges) = plan_polyline_corners(
        &points,
        &bulges,
        PolylineCornerSpec::Fillet { radius },
        tol(),
    )
    .expect("the 90° corner is planable");
    assert_eq!(out_points.len(), 4, "one corner adds exactly one vertex");
    assert_close(out_points[0], point(10.0, 0.0), "the start vertex");
    assert_close(out_points[1], point(2.0, 0.0), "t1");
    assert_close(out_points[2], point(0.0, 2.0), "t2");
    assert_close(out_points[3], point(0.0, 10.0), "the end vertex");

    // Recover the arc through the single bulge → arc authority.
    let recovered = bulge_arc_geometry(
        out_points[1],
        out_points[2],
        out_bulges[1],
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

    // The two-line reference on the very same arms.
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

    // Parity within 1e-12 (centre, radius, sweep).
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
    // ... and the tangent points are the two-line edit's new endpoints (the far
    // ends of the two picked lines never move, so the *other* endpoint of each
    // retargeted line is the tangent point).
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
    // The arc turns toward the corner (the removed material), not away from it.
    assert_close(center, point(2.0, 2.0), "the fillet centre");
    assert!(
        sweep < 0.0,
        "a right turn takes the negative sweep: {sweep}"
    );
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

    // One `update_entity`, one transaction, one undo step.
    assert_eq!(
        outcome.changes.as_ref().unwrap().changes.len(),
        1,
        "the polyline is rewritten in place (no insert, no delete)"
    );
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(
        outcome.diagnostics[0].code, "drawing.fillet_polyline",
        "the diagnostic names the command"
    );
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .unwrap()
            .layer,
        LayerId(0),
        "the rewrite keeps the entity's own layer"
    );

    // 4 vertices with 2 interior corners ⇒ 4 + 2 = 6 vertices.
    let (points, bulges, closed) = stored_polyline(&app, 1);
    assert!(!closed, "the polyline stays open");
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
    // The two end vertices never move and the two end segments keep their length.
    assert_close(points[0], point(0.0, 0.0), "start vertex");
    assert_close(points[5], point(40.0, 20.0), "end vertex");

    // The joining segments are arcs of the session radius, turning toward the
    // corner: the first corner turns left (positive bulge), the second right
    // (negative bulge).
    assert!(
        bulges[1] > 0.0,
        "a left turn takes a positive bulge: {}",
        bulges[1]
    );
    assert!(
        bulges[3] < 0.0,
        "a right turn takes a negative bulge: {}",
        bulges[3]
    );
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
        // |sweep| = π - phi = π/2 for a 90° corner.
        assert!((sweep.abs() - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    }
    // Every untouched segment keeps its stored (straight) bulge.
    for index in [0usize, 2, 4] {
        assert_eq!(bulges[index], STRAIGHT_BULGE_ZERO, "segment {index}");
    }

    // One undo restores the original polyline exactly.
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
    // Our declared orientation rule: `A` along the arm arriving at the corner,
    // `B` along the arm leaving it. Corner (20,0) arrives on +x and leaves on +y,
    // so t1 = (20 - 2, 0) and t2 = (20, 0 + 3); the reversed reading would give
    // (17,0)/(20,2) and is asserted absent below.
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
    assert_points(&points[1..3], &[point(18.0, 0.0), point(20.0, 3.0)]);
    assert_points(&points[3..5], &[point(20.0, 18.0), point(23.0, 20.0)]);
    // The reverse orientation would be a different, wrong geometry.
    assert_close(
        points[1],
        point(18.0, 0.0),
        "A is measured on the incoming arm",
    );
    assert!(
        (points[1].x - 17.0).abs() > 1e-9,
        "A must not be measured on the outgoing arm"
    );
    // A chamfer joins with a straight segment.
    for index in [1usize, 3] {
        assert_eq!(bulges[index], STRAIGHT_BULGE_ZERO, "segment {index}");
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
    assert_eq!(
        after.geometry, before.geometry,
        "a refused edit leaves the entity bytes unchanged"
    );
    assert_eq!(after.object.revision, before.object.revision);
}

#[test]
fn polyline_fillet_and_chamfer_refuse_every_unsupported_shape_by_name() {
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

    // A closed polyline (v1 is open only).
    let (mut app, mut session) = polyline_app(
        vec![point(0.0, 0.0), point(20.0, 0.0), point(20.0, 20.0)],
        vec![0.0; 3],
        true,
    );
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_closed",
        true,
    );

    // A bulged (arc) segment.
    let (mut app, mut session) = polyline_app(
        vec![point(0.0, 0.0), point(20.0, 0.0), point(20.0, 20.0)],
        vec![0.0, 0.25, 0.0],
        false,
    );
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferPolyline,
        "chamfer.polyline_bulged_segment",
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
}

#[test]
fn polyline_fillet_and_chamfer_refuse_degenerate_input_by_name() {
    // Fewer than three vertices: nothing to round.
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

    // Duplicate consecutive vertices (a zero-length segment has no direction).
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

    // A collinear interior vertex: the two arms are collinear, there is no
    // corner (AutoCAD has nothing to round there either).
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
        "fillet.polyline_collinear_vertex",
        false,
    );

    // ... and the doubling-back spike (the arms collinear, same direction).
    let (mut app, mut session) = polyline_app(
        vec![point(0.0, 0.0), point(20.0, 0.0), point(10.0, 0.0)],
        vec![0.0; 3],
        false,
    );
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_collinear_vertex",
        false,
    );
}

#[test]
fn polyline_fillet_and_chamfer_refuse_an_out_of_range_corner_size() {
    // FILLET: `R = 0` is the two-line "move to the corner" form, which has no
    // polyline equivalent — refused by name, never silently reused.
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_radius_not_positive",
        false,
    );

    // CHAMFER: `A = B = 0` is the same corner-only form.
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 0.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 0.0);
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferPolyline,
        "chamfer.polyline_distances_invalid",
        false,
    );

    // FILLET: the radius does not fit the segments (`d = R = 20` on a 20-long
    // segment). AutoCAD would auto-reduce it; we refuse.
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 20.0);
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_too_large",
        false,
    );

    // CHAMFER: `A + B` on one segment exceeds the segment length (the two
    // neighbouring corners would overlap).
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 11.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 11.0);
    select(&mut app, &mut session, &[1]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferPolyline,
        "chamfer.polyline_too_large",
        false,
    );
}

#[test]
fn polyline_fillet_refuses_a_missing_session_parameter_by_name() {
    // The typed channel validates on write, so an unusable value can only arrive
    // through the test-only unvalidated setter; the command's own guard must
    // still refuse it instead of inventing a radius.
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
}

#[test]
fn polyline_fillet_refuses_a_selection_that_is_not_exactly_one_entity() {
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
}

#[test]
fn polyline_fillet_refuses_an_unaddressable_reference() {
    // Another document's ref.
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    session.selection.replace(vec![SelectionRef {
        document: DocumentId(2),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    }]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_not_addressable",
        false,
    );

    // A block-instance member (the stored entity is not the addressable object).
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    session.selection.replace(vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath(vec![EntityId(1)]),
        sub_element: None,
    }]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_not_addressable",
        true,
    );

    // A sub-element ref.
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    session.selection.replace(vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: Some(SubElementId {
            source_key: "edge:0".into(),
            topology_revision: Revision(0),
        }),
    }]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_not_addressable",
        true,
    );

    // A ref that names no stored entity.
    let (mut app, mut session) = l_polyline_app();
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    session.selection.replace(vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(99),
        instance: InstancePath::default(),
        sub_element: None,
    }]);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletPolyline,
        "fillet.polyline_target_missing",
        false,
    );
}

#[test]
fn polyline_fillet_and_chamfer_are_work_only_and_refused_in_viewer() {
    let (mut app, mut session) = l_polyline_app();
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    select(&mut app, &mut viewer, &[1]);
    let rev = revision(&app);
    for id in [CommandId::FilletPolyline, CommandId::ChamferPolyline] {
        let error = execute(&mut app, &mut viewer, id, CommandPayload::None).unwrap_err();
        assert_eq!(error, CadError::PermissionDenied, "{id:?}");
    }
    assert_eq!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 0);
    assert!(CommandId::FilletPolyline.requires_work_mode());
    assert!(CommandId::ChamferPolyline.requires_work_mode());
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
    // The fixture also stores a second (untouched) polyline; the rounded one is
    // the single imported polyline with the rewritten vertex count.
    let matching: Vec<&(Vec<Point3>, Vec<f64>)> = polylines
        .iter()
        .filter(|(points, _)| points.len() == expected_points.len())
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "exactly one re-imported LWPOLYLINE has the rounded vertex count"
    );
    let (points, bulges) = matching[0];
    assert_points(points, &expected_points);
    assert_eq!(bulges.len(), expected_bulges.len());
    for (index, (a, b)) in bulges.iter().zip(&expected_bulges).enumerate() {
        assert!(
            (a - b).abs() < 1e-9,
            "bulge {index}: {a} vs {b} (the arc segments keep their sweep)"
        );
    }
}
