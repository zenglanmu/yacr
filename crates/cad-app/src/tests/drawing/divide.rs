//! `DIVIDE` edit-command contract tests.
//!
//! Pins the equal-arc-length point placement for LINE/ARC/CIRCLE and straight
//! LWPOLYLINE, the source-retained / marks-inserted transaction shape (one
//! transaction / one undo step, on the active layer) and every named refusal
//! (nothing written, no revision bump, no undo entry).

use super::explode::part;
use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};

const TAU: f64 = std::f64::consts::TAU;

fn line_geometry(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: point(x0, y0),
        end: point(x1, y1),
    }
}

fn arc_geometry(center: Point3, radius: f64, start: f64, sweep: f64) -> SemanticGeometry {
    SemanticGeometry::Arc {
        center,
        normal: p3(0.0, 0.0, 1.0),
        radius,
        start,
        sweep,
    }
}

fn circle_geometry(center: Point3, radius: f64) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center,
        normal: p3(0.0, 0.0, 1.0),
        radius,
    }
}

fn polyline_geometry(points: Vec<Point3>, bulges: Vec<f64>, closed: bool) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    }
}

/// A work-mode app with layers 0/WALLS and the given model entities.
fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    build(entities, AppMode::Work)
}

fn viewer_app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    build(entities, AppMode::Viewer)
}

fn build(entities: Vec<DbEntity>, mode: AppMode) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([51u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), mode))
}

fn curve_entity(
    id: u128,
    layer: LayerId,
    geometry: SemanticGeometry,
    type_key: &str,
    order: i64,
) -> DbEntity {
    part(id, geometry, layer, SpaceId::Model, type_key, order)
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn entity(app: &Application, id: u128) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
        .unwrap_or_else(|| panic!("entity {id} should exist"))
}

/// Every POINT entity in the drawing, in draw order.
fn point_entities(app: &Application) -> Vec<DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .filter(|entity| matches!(entity.geometry, SemanticGeometry::Point(_)))
        .map(|entity| DbEntity::clone(entity))
        .collect()
}

fn point_positions(app: &Application) -> Vec<Point3> {
    point_entities(app)
        .into_iter()
        .map(|entity| match entity.geometry {
            SemanticGeometry::Point(position) => position,
            other => panic!("expected a POINT, got {other:?}"),
        })
        .collect()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn near_point(a: Point3, b: Point3) -> bool {
    near(a.x, b.x) && near(a.y, b.y) && near(a.z, b.z)
}

fn set_segments(session: &mut SessionState, segments: f64) {
    session
        .set_tool_parameter(ToolParameter::DivideSegments, ToolValue::Number(segments))
        .unwrap_or_else(|error| panic!("divideSegments {segments} refused: {error}"));
}

fn divide(
    app: &mut Application,
    session: &mut SessionState,
    refs: Vec<SelectionRef>,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Divide,
        CommandPayload::Divide { refs },
    )
}

fn assert_invalid(error: CadError, code: &str) {
    match error {
        CadError::InvalidInput(message) => assert!(
            message.starts_with(code),
            "expected refusal starting with {code}, got {message}"
        ),
        other => panic!("expected InvalidInput {code}, got {other:?}"),
    }
}

fn assert_unsupported(error: CadError, needle: &str) {
    match error {
        CadError::Unsupported(message) => assert!(
            message.contains(needle),
            "expected Unsupported containing {needle}, got {message}"
        ),
        other => panic!("expected Unsupported containing {needle}, got {other:?}"),
    }
}

#[test]
fn line_divide_into_three_places_two_interior_points_at_one_and_two_thirds() {
    let source = curve_entity(
        1,
        LayerId(7),
        line_geometry(0.0, 0.0, 9.0, 0.0),
        "AcDbLine",
        3,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_segments(&mut session, 3.0);
    let before_revision = revision(&app);

    let outcome = divide(&mut app, &mut session, vec![ref_of(1)]).expect("the line divides");
    assert_eq!(outcome.objects.len(), 2, "two interior marks");
    assert_eq!(
        outcome.diagnostics.len(),
        1,
        "one DIVIDE summary diagnostic"
    );
    assert_eq!(outcome.diagnostics[0].code, "drawing.divide");
    assert!(outcome.diagnostics[0].message.contains("open curve"));

    let marks = point_positions(&app);
    assert_eq!(marks.len(), 2);
    assert!(near_point(marks[0], point(3.0, 0.0)), "{:?}", marks[0]);
    assert!(near_point(marks[1], point(6.0, 0.0)), "{:?}", marks[1]);

    // The source curve is retained, untouched and still a LINE.
    let retained = entity(&app, 1);
    assert_eq!(retained.object.type_key, "AcDbLine");
    assert!(matches!(retained.geometry, SemanticGeometry::Line { .. }));
    assert_eq!(retained.layer, LayerId(7), "source layer preserved");

    assert_eq!(entity_count(&app), 3, "source + two marks");
    assert_eq!(revision(&app).0, before_revision.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "exactly one undo step");
}

#[test]
fn divide_places_the_marks_on_the_active_layer() {
    let source = curve_entity(
        1,
        LayerId(7),
        line_geometry(0.0, 0.0, 9.0, 0.0),
        "AcDbLine",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_segments(&mut session, 3.0);
    session.active_layer = LayerId(0);

    divide(&mut app, &mut session, vec![ref_of(1)]).expect("the line divides");
    let marks = point_entities(&app);
    assert_eq!(marks.len(), 2);
    for mark in &marks {
        assert_eq!(mark.layer, LayerId(0), "marks use the ACTIVE layer");
        assert_eq!(mark.space, SpaceId::Model, "marks land in model space");
        assert_eq!(mark.object.type_key, "AcDbPoint");
    }

    // Switching the active layer moves the next batch of marks with it.
    let (mut app, mut session) = app_with(vec![curve_entity(
        1,
        LayerId(0),
        line_geometry(0.0, 0.0, 9.0, 0.0),
        "AcDbLine",
        0,
    )]);
    set_segments(&mut session, 3.0);
    session.active_layer = LayerId(7);
    divide(&mut app, &mut session, vec![ref_of(1)]).expect("the line divides");
    for mark in point_entities(&app) {
        assert_eq!(mark.layer, LayerId(7), "marks follow the active layer");
    }
}

#[test]
fn circle_divide_into_four_places_the_start_parameter_and_each_quadrant() {
    let source = curve_entity(
        1,
        LayerId(0),
        circle_geometry(point(0.0, 0.0), 5.0),
        "AcDbCircle",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_segments(&mut session, 4.0);

    let outcome = divide(&mut app, &mut session, vec![ref_of(1)]).expect("the circle divides");
    assert_eq!(outcome.objects.len(), 4, "a periodic curve places N marks");
    assert!(outcome.diagnostics[0].message.contains("closed curve"));

    let marks = point_positions(&app);
    assert_eq!(marks.len(), 4);
    for (index, expected) in [
        point(5.0, 0.0),
        point(0.0, 5.0),
        point(-5.0, 0.0),
        point(0.0, -5.0),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            near_point(marks[index], expected),
            "mark {index}: {:?} != {expected:?}",
            marks[index]
        );
    }
    // The circle is retained.
    assert!(matches!(
        entity(&app, 1).geometry,
        SemanticGeometry::Circle { .. }
    ));
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn open_arc_divide_places_interior_points_only() {
    let source = curve_entity(
        1,
        LayerId(0),
        arc_geometry(point(0.0, 0.0), 4.0, 0.0, TAU / 4.0),
        "AcDbArc",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_segments(&mut session, 4.0);

    divide(&mut app, &mut session, vec![ref_of(1)]).expect("the arc divides");
    let marks = point_positions(&app);
    assert_eq!(marks.len(), 3, "an open arc places N - 1 interior marks");
    let sweep = TAU / 4.0;
    for (index, mark) in marks.iter().enumerate() {
        let fraction = (index + 1) as f64 / 4.0;
        let angle = sweep * fraction;
        let expected = point(4.0 * angle.cos(), 4.0 * angle.sin());
        assert!(
            near_point(*mark, expected),
            "mark {index}: {mark:?} != {expected:?}"
        );
    }
}

#[test]
fn straight_polylines_divide_by_cumulative_arc_length_open_and_closed() {
    // Open 0→3→3,+4 (lengths 3 + 4 = 7): N = 4 marks at 1.75, 3.5, 5.25.
    let open = curve_entity(
        1,
        LayerId(0),
        polyline_geometry(
            vec![point(0.0, 0.0), point(3.0, 0.0), point(3.0, 4.0)],
            vec![0.0, 0.0],
            false,
        ),
        "AcDbPolyline",
        0,
    );
    let (mut app, mut session) = app_with(vec![open]);
    set_segments(&mut session, 4.0);
    divide(&mut app, &mut session, vec![ref_of(1)]).expect("the open polyline divides");
    let marks = point_positions(&app);
    assert_eq!(marks.len(), 3, "an open polyline places N - 1 marks");
    for (index, expected) in [point(1.75, 0.0), point(3.0, 0.5), point(3.0, 2.25)]
        .into_iter()
        .enumerate()
    {
        assert!(
            near_point(marks[index], expected),
            "open mark {index}: {:?} != {expected:?}",
            marks[index]
        );
    }

    // Closed 4×4 square (perimeter 16): N = 4 marks at 0, 4, 8, 12.
    let closed = curve_entity(
        1,
        LayerId(0),
        polyline_geometry(
            vec![
                point(0.0, 0.0),
                point(4.0, 0.0),
                point(4.0, 4.0),
                point(0.0, 4.0),
            ],
            vec![0.0; 4],
            true,
        ),
        "AcDbPolyline",
        0,
    );
    let (mut app, mut session) = app_with(vec![closed]);
    set_segments(&mut session, 4.0);
    divide(&mut app, &mut session, vec![ref_of(1)]).expect("the closed polyline divides");
    let marks = point_positions(&app);
    assert_eq!(marks.len(), 4, "a closed polyline places N marks");
    for (index, expected) in [
        point(0.0, 0.0),
        point(4.0, 0.0),
        point(4.0, 4.0),
        point(0.0, 4.0),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            near_point(marks[index], expected),
            "closed mark {index}: {:?} != {expected:?}",
            marks[index]
        );
    }
}

#[test]
fn a_closed_polyline_that_repeats_its_first_vertex_is_not_closed_twice() {
    // The last vertex repeats the first, so no extra closing vertex is appended
    // and the perimeter is still 16 (not 20): N = 2 marks at 0 and 8.
    let source = curve_entity(
        1,
        LayerId(0),
        polyline_geometry(
            vec![
                point(0.0, 0.0),
                point(4.0, 0.0),
                point(4.0, 4.0),
                point(0.0, 4.0),
                point(0.0, 0.0),
            ],
            vec![0.0; 5],
            true,
        ),
        "AcDbPolyline",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_segments(&mut session, 2.0);
    divide(&mut app, &mut session, vec![ref_of(1)]).expect("the ring divides");
    let marks = point_positions(&app);
    assert_eq!(marks.len(), 2);
    assert!(near_point(marks[0], point(0.0, 0.0)), "{:?}", marks[0]);
    assert!(near_point(marks[1], point(4.0, 4.0)), "{:?}", marks[1]);
}

#[test]
fn a_bulged_polyline_is_refused_by_name_and_writes_nothing() {
    let source = curve_entity(
        1,
        LayerId(0),
        polyline_geometry(
            vec![point(0.0, 0.0), point(4.0, 0.0), point(4.0, 4.0)],
            vec![0.0, 1.0],
            false,
        ),
        "AcDbPolyline",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_segments(&mut session, 4.0);
    let before_revision = revision(&app);

    let error = divide(&mut app, &mut session, vec![ref_of(1)]).expect_err("bulges are refused");
    assert_unsupported(error, "divide.bulged_segment");
    assert_eq!(revision(&app), before_revision, "no revision bump");
    assert_eq!(entity_count(&app), 1, "nothing written");
    assert_eq!(undo_depth(&app), 0, "no transaction");
}

#[test]
fn a_degenerate_polyline_is_refused_by_name() {
    for (label, points) in [
        ("one vertex", vec![point(0.0, 0.0)]),
        (
            "all vertices coincide",
            vec![point(1.0, 1.0), point(1.0, 1.0)],
        ),
        (
            "non-finite vertex",
            vec![
                point(0.0, 0.0),
                Point3 {
                    x: f64::NAN,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        ),
    ] {
        let source = curve_entity(
            1,
            LayerId(0),
            polyline_geometry(points.clone(), vec![0.0; points.len()], false),
            "AcDbPolyline",
            0,
        );
        let (mut app, mut session) = app_with(vec![source]);
        set_segments(&mut session, 2.0);
        let error = divide(&mut app, &mut session, vec![ref_of(1)])
            .expect_err("a degenerate polyline is refused");
        assert_invalid(error, "divide.degenerate");
        assert_eq!(entity_count(&app), 1, "{label}: nothing written");
        assert_eq!(undo_depth(&app), 0, "{label}: no transaction");
    }
}

#[test]
fn an_out_of_range_segment_count_is_refused_defensively() {
    for segments in [1.0, 1025.0, 2.5, f64::NAN] {
        let source = curve_entity(
            1,
            LayerId(0),
            line_geometry(0.0, 0.0, 9.0, 0.0),
            "AcDbLine",
            0,
        );
        let (mut app, mut session) = app_with(vec![source]);
        // Injected past the typed channel, exactly like the HATCH parameter tests.
        session.force_tool_parameter(ToolParameter::DivideSegments, ToolValue::Number(segments));
        let error = divide(&mut app, &mut session, vec![ref_of(1)])
            .expect_err("an invalid segment count is refused");
        assert_invalid(error, "divide.invalid_segments");
        assert_eq!(entity_count(&app), 1, "{segments}: nothing written");
        assert_eq!(undo_depth(&app), 0, "{segments}: no transaction");
        // The typed channel refuses the same value.
        assert!(
            session
                .set_tool_parameter(ToolParameter::DivideSegments, ToolValue::Number(segments))
                .is_err(),
            "{segments}: the channel must refuse it"
        );
    }
}

#[test]
fn divide_needs_exactly_one_selected_curve() {
    let source = curve_entity(
        1,
        LayerId(0),
        line_geometry(0.0, 0.0, 9.0, 0.0),
        "AcDbLine",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_segments(&mut session, 3.0);
    for refs in [Vec::new(), vec![ref_of(1), ref_of(1)]] {
        let count = refs.len();
        let error = divide(&mut app, &mut session, refs).expect_err("exactly one ref is required");
        assert_invalid(error, "divide.needs_one");
        assert_eq!(entity_count(&app), 1, "{count} refs: nothing written");
        assert_eq!(undo_depth(&app), 0, "{count} refs: no transaction");
    }
}

#[test]
fn a_spline_target_is_the_explicit_unsupported_refusal() {
    let fit = vec![
        point(0.0, 0.0),
        point(1.0, 2.0),
        point(2.5, 2.5),
        point(4.0, 1.0),
        point(5.5, -1.0),
    ];
    let (knots, control_points) =
        crate::draw_tool::spline_control_points(&fit).expect("a valid fit spline");
    let spline = SemanticGeometry::Spline {
        degree: cad_geometry::FIT_SPLINE_DEGREE,
        knots,
        control_points,
        weights: vec![1.0; fit.len()],
    };
    let source = curve_entity(1, LayerId(0), spline, "AcDbSpline", 0);
    let (mut app, mut session) = app_with(vec![source]);
    let error = divide(&mut app, &mut session, vec![ref_of(1)]).expect_err("SPLINE is refused");
    assert_unsupported(error, "SPLINE is not supported");
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn an_ellipse_is_the_explicit_unsupported_refusal() {
    let ellipse = curve_entity(
        1,
        LayerId(0),
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(1.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: TAU,
        },
        "AcDbEllipse",
        0,
    );
    let (mut app, mut session) = app_with(vec![ellipse]);
    let error = divide(&mut app, &mut session, vec![ref_of(1)]).expect_err("ELLIPSE is refused");
    assert_unsupported(error, "ELLIPSE is not supported");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn divide_is_one_undo_step_and_undo_removes_the_marks() {
    let source = curve_entity(
        1,
        LayerId(0),
        line_geometry(0.0, 0.0, 9.0, 0.0),
        "AcDbLine",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_segments(&mut session, 3.0);
    let before_count = entity_count(&app);

    divide(&mut app, &mut session, vec![ref_of(1)]).expect("the line divides");
    assert_eq!(entity_count(&app), before_count + 2);
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("the division is undoable");
    assert_eq!(entity_count(&app), before_count, "both marks removed");
    assert!(point_positions(&app).is_empty(), "no POINT mark survives");
    assert_eq!(undo_depth(&app), 0);
    assert!(matches!(
        entity(&app, 1).geometry,
        SemanticGeometry::Line { .. }
    ));
}

#[test]
fn viewer_mode_refuses_divide() {
    let source = curve_entity(
        1,
        LayerId(0),
        line_geometry(0.0, 0.0, 9.0, 0.0),
        "AcDbLine",
        0,
    );
    let (mut app, mut session) = viewer_app_with(vec![source]);
    let before_revision = revision(&app);
    let error = divide(&mut app, &mut session, vec![ref_of(1)]).expect_err("DIVIDE is Work-only");
    assert!(
        matches!(error, CadError::PermissionDenied),
        "expected PermissionDenied, got {error:?}"
    );
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_missing_target_is_refused_by_name() {
    let source = curve_entity(
        1,
        LayerId(0),
        line_geometry(0.0, 0.0, 9.0, 0.0),
        "AcDbLine",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    let error = divide(&mut app, &mut session, vec![ref_of(99)]).expect_err("99 does not exist");
    assert_invalid(error, "divide.target_missing");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_sub_element_reference_is_not_addressable() {
    let source = curve_entity(
        1,
        LayerId(0),
        line_geometry(0.0, 0.0, 9.0, 0.0),
        "AcDbLine",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    let mut reference = ref_of(1);
    reference.sub_element = Some(cad_domain::SubElementId {
        source_key: "edge".into(),
        topology_revision: Revision(0),
    });
    let error = divide(&mut app, &mut session, vec![reference])
        .expect_err("a sub-element is not addressable");
    assert_invalid(error, "divide.not_addressable");
    assert_eq!(undo_depth(&app), 0);
}
