//! `LENGTHEN` edit-command contract tests.
//!
//! Pins the three modes (Delta / Percent / Total), the declared v1 rule that the
//! **end** endpoint always moves, the in-place update of the **same** entity
//! (identity / draw order / layer / type key preserved, one transaction / one
//! undo step) and every named refusal (nothing written, no revision bump, no undo
//! entry).

use super::explode::part;
use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};

const TAU: f64 = std::f64::consts::TAU;

/// LENGTHEN mode `Delta` on the typed channel.
const DELTA: f64 = 0.0;
/// LENGTHEN mode `Percent` on the typed channel.
const PERCENT: f64 = 1.0;
/// LENGTHEN mode `Total` on the typed channel.
const TOTAL: f64 = 2.0;

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
            identity: DocumentIdentity::Sha256([52u8; 32]),
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

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn set_mode_value(session: &mut SessionState, mode: f64, value: f64) {
    session
        .set_tool_parameter(ToolParameter::LengthenMode, ToolValue::Number(mode))
        .unwrap_or_else(|error| panic!("lengthenMode {mode} refused: {error}"));
    session
        .set_tool_parameter(ToolParameter::LengthenValue, ToolValue::Number(value))
        .unwrap_or_else(|error| panic!("lengthenValue {value} refused: {error}"));
}

fn lengthen(
    app: &mut Application,
    session: &mut SessionState,
    refs: Vec<SelectionRef>,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Lengthen,
        CommandPayload::Lengthen { refs },
    )
}

/// The length of a LINE/ARC geometry (chord length / radius · |sweep|).
fn geometry_length(geometry: &SemanticGeometry) -> f64 {
    match geometry {
        SemanticGeometry::Line { start, end } => (end.x - start.x)
            .hypot(end.y - start.y)
            .hypot(end.z - start.z),
        SemanticGeometry::Arc { radius, sweep, .. } => radius * sweep.abs(),
        other => panic!("expected a LINE or ARC, got {other:?}"),
    }
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

fn line_source() -> DbEntity {
    curve_entity(
        1,
        LayerId(7),
        line_geometry(0.0, 0.0, 5.0, 0.0),
        "AcDbLine",
        4,
    )
}

#[test]
fn delta_mode_moves_the_end_endpoint_and_preserves_the_entity() {
    let (mut app, mut session) = app_with(vec![line_source()]);
    set_mode_value(&mut session, DELTA, 1.0);
    let before_revision = revision(&app);
    let before_count = entity_count(&app);

    let outcome = lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("the line lengthens");
    assert_eq!(
        outcome.objects,
        vec![ObjectId(1)],
        "the SAME entity is updated"
    );
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, "drawing.lengthen");
    assert!(outcome.diagnostics[0].message.contains("Delta"));

    let after = entity(&app, 1);
    match &after.geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(
                near(start.x, 0.0) && near(start.y, 0.0),
                "the START endpoint is kept: {start:?}"
            );
            assert!(
                near(end.x, 6.0) && near(end.y, 0.0),
                "the END endpoint moves to length 6: {end:?}"
            );
        }
        other => panic!("expected a LINE, got {other:?}"),
    }
    // Identity, draw order, layer, space and type key are preserved.
    assert_eq!(after.id, EntityId(1));
    assert_eq!(after.draw_order, 4);
    assert_eq!(after.layer, LayerId(7));
    assert_eq!(after.space, SpaceId::Model);
    assert_eq!(after.object.type_key, "AcDbLine");
    assert!(near(geometry_length(&after.geometry), 6.0));

    assert_eq!(entity_count(&app), before_count, "no entity added/removed");
    assert_eq!(revision(&app).0, before_revision.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "exactly one undo step");
}

#[test]
fn percent_mode_uses_a_percentage_of_the_old_length() {
    let source = curve_entity(
        1,
        LayerId(0),
        line_geometry(0.0, 0.0, 4.0, 0.0),
        "AcDbLine",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_mode_value(&mut session, PERCENT, 150.0);
    lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("150% of length 4 is 6");
    assert!(near(geometry_length(&entity(&app, 1).geometry), 6.0));
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn total_mode_sets_the_absolute_length() {
    let source = curve_entity(
        1,
        LayerId(0),
        line_geometry(0.0, 0.0, 3.0, 0.0),
        "AcDbLine",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_mode_value(&mut session, TOTAL, 5.0);
    lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("total length 5");
    assert!(near(geometry_length(&entity(&app, 1).geometry), 5.0));
}

#[test]
fn a_negative_delta_shortens_toward_the_kept_end() {
    let (mut app, mut session) = app_with(vec![line_source()]);
    set_mode_value(&mut session, DELTA, -2.0);
    lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("length 5 - 2 = 3");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(near(start.x, 0.0) && near(start.y, 0.0), "start kept");
            assert!(near(end.x, 3.0), "end shortened to 3: {end:?}");
        }
        other => panic!("expected a LINE, got {other:?}"),
    }
}

#[test]
fn arc_delta_grows_the_sweep_and_keeps_centre_radius_and_start() {
    let source = curve_entity(
        1,
        LayerId(7),
        arc_geometry(point(10.0, 5.0), 2.0, 0.0, 1.0),
        "AcDbArc",
        2,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_mode_value(&mut session, DELTA, 2.0);
    lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("old 2 + 2 = 4");

    let after = entity(&app, 1);
    match &after.geometry {
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            ..
        } => {
            assert!(near(center.x, 10.0) && near(center.y, 5.0), "centre kept");
            assert!(near(*radius, 2.0), "radius kept");
            assert!(near(*start, 0.0), "start angle kept");
            assert!(
                near(*sweep, 2.0),
                "new sweep = new length / radius = 4 / 2: {sweep}"
            );
        }
        other => panic!("expected an ARC, got {other:?}"),
    }
    assert!(near(geometry_length(&after.geometry), 4.0));
    assert_eq!(after.object.type_key, "AcDbArc", "ARC stays an ARC");
    assert_eq!(after.draw_order, 2);
    assert_eq!(after.layer, LayerId(7));
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn an_arc_sweep_of_exactly_one_turn_stays_an_arc() {
    let source = curve_entity(
        1,
        LayerId(0),
        arc_geometry(point(0.0, 0.0), 1.0, 0.0, 0.5),
        "AcDbArc",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_mode_value(&mut session, TOTAL, 360.0);
    lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("one full turn is accepted");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Arc { sweep, .. } => assert!(near(*sweep, TAU), "{sweep}"),
        other => panic!("expected an ARC, got {other:?}"),
    }
    assert_eq!(entity(&app, 1).object.type_key, "AcDbArc");
}

#[test]
fn a_non_positive_new_length_is_refused_by_name() {
    for (label, mode, value) in [
        ("delta to exactly zero", DELTA, -5.0),
        ("delta below zero", DELTA, -6.0),
    ] {
        let (mut app, mut session) = app_with(vec![line_source()]);
        session.force_tool_parameter(ToolParameter::LengthenMode, ToolValue::Number(mode));
        session.force_tool_parameter(ToolParameter::LengthenValue, ToolValue::Number(value));
        let before_revision = revision(&app);
        let error = lengthen(&mut app, &mut session, vec![ref_of(1)])
            .expect_err("a non-positive new length is refused");
        assert_invalid(error, "lengthen.non_positive");
        assert_eq!(revision(&app), before_revision, "{label}: no revision bump");
        assert_eq!(undo_depth(&app), 0, "{label}: no transaction");
    }
}

#[test]
fn an_arc_beyond_one_full_turn_overflows_by_name() {
    let source = curve_entity(
        1,
        LayerId(0),
        arc_geometry(point(0.0, 0.0), 1.0, 0.0, 0.5),
        "AcDbArc",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    // Total included angle 361° would sweep more than one full turn.
    set_mode_value(&mut session, TOTAL, 361.0);
    let before_revision = revision(&app);
    let error = lengthen(&mut app, &mut session, vec![ref_of(1)])
        .expect_err("more than one turn is refused");
    assert_invalid(error, "lengthen.sweep_overflow");
    assert_eq!(revision(&app), before_revision, "no revision bump");
    assert_eq!(undo_depth(&app), 0, "no transaction");
    assert!(matches!(
        entity(&app, 1).geometry,
        SemanticGeometry::Arc { sweep, .. } if near(sweep, 0.5)
    ));
}

#[test]
fn degenerate_line_and_arc_targets_are_refused() {
    let cases: Vec<(&str, SemanticGeometry, &str)> = vec![
        (
            "zero-length line",
            line_geometry(2.0, 2.0, 2.0, 2.0),
            "AcDbLine",
        ),
        (
            "zero-sweep arc",
            arc_geometry(point(0.0, 0.0), 2.0, 0.0, 0.0),
            "AcDbArc",
        ),
        (
            "zero-radius arc",
            arc_geometry(point(0.0, 0.0), 0.0, 0.0, 1.0),
            "AcDbArc",
        ),
    ];
    for (label, geometry, type_key) in cases {
        let (mut app, mut session) =
            app_with(vec![curve_entity(1, LayerId(0), geometry, type_key, 0)]);
        set_mode_value(&mut session, DELTA, 1.0);
        let error =
            lengthen(&mut app, &mut session, vec![ref_of(1)]).expect_err("degenerate is refused");
        assert_invalid(error, "lengthen.degenerate");
        assert_eq!(undo_depth(&app), 0, "{label}: no transaction");
    }
}

#[test]
fn closed_objects_are_refused_and_open_curves_are_supported() {
    // Circles and full ellipses have no ends, so LENGTHEN refuses them by name.
    let circle = curve_entity(
        1,
        LayerId(0),
        SemanticGeometry::Circle {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 3.0,
        },
        "AcDbCircle",
        0,
    );
    let (mut app, mut session) = app_with(vec![circle]);
    set_mode_value(&mut session, DELTA, 1.0);
    let error = lengthen(&mut app, &mut session, vec![ref_of(1)]).expect_err("CIRCLE is refused");
    assert_invalid(error, "lengthen.closed");
    assert_eq!(undo_depth(&app), 0);

    let closed_polyline = curve_entity(
        1,
        LayerId(0),
        SemanticGeometry::Polyline {
            points: vec![point(0.0, 0.0), point(4.0, 0.0), point(4.0, 4.0)],
            bulges: vec![0.0; 3],
            closed: true,
            width: 0.0,
        },
        "AcDbPolyline",
        0,
    );
    let (mut app, mut session) = app_with(vec![closed_polyline]);
    set_mode_value(&mut session, DELTA, 1.0);
    let error = lengthen(&mut app, &mut session, vec![ref_of(1)])
        .expect_err("a closed polyline is refused");
    assert_invalid(error, "lengthen.closed");
    assert_eq!(undo_depth(&app), 0);

    let full_ellipse = curve_entity(
        1,
        LayerId(0),
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(4.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: TAU,
        },
        "AcDbEllipse",
        0,
    );
    let (mut app, mut session) = app_with(vec![full_ellipse]);
    set_mode_value(&mut session, DELTA, 1.0);
    let error =
        lengthen(&mut app, &mut session, vec![ref_of(1)]).expect_err("a full ellipse is refused");
    assert_invalid(error, "lengthen.closed");
    assert_eq!(undo_depth(&app), 0);

    // An open polyline is supported: delta +1 extends the end by one unit.
    let open_polyline = curve_entity(
        1,
        LayerId(0),
        SemanticGeometry::Polyline {
            points: vec![point(0.0, 0.0), point(4.0, 0.0)],
            bulges: vec![0.0],
            closed: false,
            width: 0.0,
        },
        "AcDbPolyline",
        0,
    );
    let (mut app, mut session) = app_with(vec![open_polyline]);
    set_mode_value(&mut session, DELTA, 1.0);
    lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("an open polyline lengthens");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Polyline { points, .. } => {
            assert!(near(points.last().unwrap().x, 5.0), "{:?}", points);
        }
        other => panic!("expected a Polyline, got {other:?}"),
    }

    // An open spline is supported for shortening; extending a NURBS curve has no
    // continuation semantics and is refused by name.
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
    let (mut app, mut session) =
        app_with(vec![curve_entity(1, LayerId(0), spline, "AcDbSpline", 0)]);
    set_mode_value(&mut session, DELTA, 1.0);
    let error = lengthen(&mut app, &mut session, vec![ref_of(1)])
        .expect_err("extending a spline is refused");
    assert_invalid(error, "lengthen.spline_extend_unsupported");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn spline_trim_keeps_an_exact_sub_curve() {
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
    let (mut app, mut session) =
        app_with(vec![curve_entity(1, LayerId(0), spline, "AcDbSpline", 0)]);
    set_mode_value(&mut session, PERCENT, 50.0);
    lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("50% of the spline is kept");
    assert!(matches!(
        entity(&app, 1).geometry,
        SemanticGeometry::Spline { .. }
    ));
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn lengthen_needs_exactly_one_selected_curve() {
    let (mut app, mut session) = app_with(vec![line_source()]);
    set_mode_value(&mut session, DELTA, 1.0);
    for refs in [Vec::new(), vec![ref_of(1), ref_of(1)]] {
        let count = refs.len();
        let error = lengthen(&mut app, &mut session, refs).expect_err("exactly one ref is needed");
        assert_invalid(error, "lengthen.needs_one");
        assert_eq!(undo_depth(&app), 0, "{count} refs: no transaction");
    }
}

#[test]
fn an_invalid_mode_or_amount_is_refused_defensively() {
    let cases: Vec<(&str, Option<f64>, Option<f64>)> = vec![
        ("fractional mode", Some(0.5), Some(1.0)),
        ("NaN mode", Some(f64::NAN), Some(1.0)),
        ("zero amount", Some(DELTA), Some(0.0)),
        ("NaN amount", Some(DELTA), Some(f64::NAN)),
        ("negative percent", Some(PERCENT), Some(-50.0)),
        ("zero total", Some(TOTAL), Some(0.0)),
    ];
    for (label, mode, value) in cases {
        let (mut app, mut session) = app_with(vec![line_source()]);
        if let Some(mode) = mode {
            session.force_tool_parameter(ToolParameter::LengthenMode, ToolValue::Number(mode));
        }
        if let Some(value) = value {
            session.force_tool_parameter(ToolParameter::LengthenValue, ToolValue::Number(value));
        }
        let error =
            lengthen(&mut app, &mut session, vec![ref_of(1)]).expect_err("invalid settings refuse");
        assert_invalid(error, "lengthen.invalid_value");
        assert_eq!(undo_depth(&app), 0, "{label}: no transaction");
        assert_eq!(entity_count(&app), 1, "{label}: nothing written");
    }
    // Mode 3 (Dynamic) is a valid channel value, but the static `Lengthen`
    // payload has no points to project; it is a named payload refusal.
    let (mut app, mut session) = app_with(vec![line_source()]);
    session.force_tool_parameter(ToolParameter::LengthenMode, ToolValue::Number(3.0));
    let error = lengthen(&mut app, &mut session, vec![ref_of(1)])
        .expect_err("the static payload cannot resolve the dynamic mode");
    assert_invalid(error, "lengthen.invalid_payload");
    assert_eq!(
        undo_depth(&app),
        0,
        "dynamic static payload: no transaction"
    );

    // The typed channel itself refuses the remaining invalid values.
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    assert!(session
        .set_tool_parameter(ToolParameter::LengthenMode, ToolValue::Number(4.0))
        .is_err());
    assert!(session
        .set_tool_parameter(ToolParameter::LengthenValue, ToolValue::Number(0.0))
        .is_err());
}

#[test]
fn lengthen_is_one_undo_step_and_undo_restores_the_original_geometry() {
    let (mut app, mut session) = app_with(vec![line_source()]);
    set_mode_value(&mut session, DELTA, 1.0);
    let before_count = entity_count(&app);

    lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("the line lengthens");
    assert!(near(geometry_length(&entity(&app, 1).geometry), 6.0));
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("the lengthen is undoable");
    assert_eq!(entity_count(&app), before_count);
    assert_eq!(undo_depth(&app), 0);
    match &entity(&app, 1).geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(
                near(start.x, 0.0) && near(end.x, 5.0),
                "original line restored"
            );
        }
        other => panic!("expected the original LINE, got {other:?}"),
    }
}

#[test]
fn viewer_mode_refuses_lengthen() {
    let (mut app, mut session) = viewer_app_with(vec![line_source()]);
    let before_revision = revision(&app);
    let error =
        lengthen(&mut app, &mut session, vec![ref_of(1)]).expect_err("LENGTHEN is Work-only");
    assert!(
        matches!(error, CadError::PermissionDenied),
        "expected PermissionDenied, got {error:?}"
    );
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_missing_target_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![line_source()]);
    set_mode_value(&mut session, DELTA, 1.0);
    let error = lengthen(&mut app, &mut session, vec![ref_of(99)]).expect_err("99 does not exist");
    assert_invalid(error, "lengthen.target_missing");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_sub_element_reference_is_not_addressable() {
    let (mut app, mut session) = app_with(vec![line_source()]);
    set_mode_value(&mut session, DELTA, 1.0);
    let mut reference = ref_of(1);
    reference.sub_element = Some(cad_domain::SubElementId {
        source_key: "edge".into(),
        topology_revision: Revision(0),
    });
    let error = lengthen(&mut app, &mut session, vec![reference])
        .expect_err("a sub-element is not addressable");
    assert_invalid(error, "lengthen.not_addressable");
    assert_eq!(undo_depth(&app), 0);
}

fn select_one(session: &mut SessionState, entity: u128) {
    session.selection.replace([ref_of(entity)]);
}

fn lengthen_dynamic(
    app: &mut Application,
    session: &mut SessionState,
    entity: u128,
    pick: Point3,
    new_end: Point3,
) -> CadResult<CommandOutcome> {
    select_one(session, entity);
    execute(
        app,
        session,
        CommandId::Lengthen,
        CommandPayload::Points(vec![pick, new_end]),
    )
}

/// The static DE/PER/TOT shape: one pick selects the end that moves.
fn lengthen_static_pick(
    app: &mut Application,
    session: &mut SessionState,
    entity: u128,
    pick: Point3,
) -> CadResult<CommandOutcome> {
    select_one(session, entity);
    execute(
        app,
        session,
        CommandId::Lengthen,
        CommandPayload::Points(vec![pick]),
    )
}

#[test]
fn a_static_pick_moves_the_endpoint_nearest_it() {
    // Pick near the START: the start moves and the end stays at x = 5.
    let (mut app, mut session) = app_with(vec![line_source()]);
    set_mode_value(&mut session, DELTA, 2.0);
    lengthen_static_pick(&mut app, &mut session, 1, point(1.0, 0.0))
        .expect("delta from the nearest endpoint");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(near(start.x, -2.0), "start moved: {start:?}");
            assert!(near(end.x, 5.0), "end kept: {end:?}");
        }
        other => panic!("expected a LINE, got {other:?}"),
    }
    assert!(near(geometry_length(&entity(&app, 1).geometry), 7.0));

    // Pick near the END: the end moves, the start stays.
    let (mut app, mut session) = app_with(vec![line_source()]);
    set_mode_value(&mut session, PERCENT, 50.0);
    lengthen_static_pick(&mut app, &mut session, 1, point(4.0, 0.0))
        .expect("percent from the nearest endpoint");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(near(start.x, 0.0), "start kept: {start:?}");
            assert!(near(end.x, 2.5), "end shortened to half: {end:?}");
        }
        other => panic!("expected a LINE, got {other:?}"),
    }
}

#[test]
fn total_on_an_arc_measured_from_the_nearest_pick_keeps_the_far_end() {
    let source = curve_entity(
        1,
        LayerId(0),
        arc_geometry(point(0.0, 0.0), 5.0, 0.0, std::f64::consts::FRAC_PI_2),
        "AcDbArc",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_mode_value(&mut session, TOTAL, 45.0);
    // The pick is at the start (5, 0), so the start moves back from the end.
    lengthen_static_pick(&mut app, &mut session, 1, point(5.0, 0.0))
        .expect("total angle from the nearest endpoint");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Arc { start, sweep, .. } => {
            assert!(near(*sweep, std::f64::consts::FRAC_PI_4), "{sweep}");
            assert!(near(*start, std::f64::consts::FRAC_PI_4), "{start}");
        }
        other => panic!("expected an ARC, got {other:?}"),
    }
}

#[test]
fn a_static_polyline_pick_trims_the_near_end_and_keeps_the_far_length() {
    let source = curve_entity(
        1,
        LayerId(0),
        SemanticGeometry::Polyline {
            points: vec![point(0.0, 0.0), point(4.0, 0.0), point(4.0, 4.0)],
            bulges: vec![0.0, 0.0],
            closed: false,
            width: 0.0,
        },
        "AcDbPolyline",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_mode_value(&mut session, DELTA, -3.0);
    lengthen_static_pick(&mut app, &mut session, 1, point(0.0, 0.0))
        .expect("trim from the nearest end");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Polyline { points, .. } => {
            assert!(
                near(points[0].x, 3.0) && near(points[0].y, 0.0),
                "{:?}",
                points[0]
            );
            assert!(
                near(points[1].x, 4.0) && near(points[1].y, 0.0),
                "{:?}",
                points[1]
            );
            assert!(near(points[2].y, 4.0), "{:?}", points[2]);
        }
        other => panic!("expected a Polyline, got {other:?}"),
    }
}

#[test]
fn an_elliptical_arc_static_pick_moves_the_nearest_end() {
    let source = curve_entity(
        1,
        LayerId(0),
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(5.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::FRAC_PI_2,
        },
        "AcDbEllipse",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_mode_value(&mut session, DELTA, 1.0);
    lengthen_static_pick(&mut app, &mut session, 1, point(5.0, 0.0))
        .expect("delta from the nearest ellipse end");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Ellipse { start, sweep, .. } => {
            assert!(*sweep > std::f64::consts::FRAC_PI_2, "{sweep}");
            assert!(*start < 0.0, "the start moved back: {start}");
        }
        other => panic!("expected an ELLIPSE, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn arc_total_mode_reads_the_amount_as_an_included_angle_in_degrees() {
    let source = curve_entity(
        1,
        LayerId(0),
        arc_geometry(point(0.0, 0.0), 2.0, 0.0, 1.0),
        "AcDbArc",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_mode_value(&mut session, TOTAL, 90.0);
    lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("90° is the new included angle");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Arc { sweep, .. } => {
            assert!(near(*sweep, std::f64::consts::FRAC_PI_2), "{sweep}");
        }
        other => panic!("expected an ARC, got {other:?}"),
    }
}

#[test]
fn percent_mode_on_an_arc_scales_the_sweep() {
    let source = curve_entity(
        1,
        LayerId(0),
        arc_geometry(point(0.0, 0.0), 2.0, 0.0, 1.0),
        "AcDbArc",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    set_mode_value(&mut session, PERCENT, 50.0);
    lengthen(&mut app, &mut session, vec![ref_of(1)]).expect("50% of length 2 is 1");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Arc { radius, sweep, .. } => {
            assert!(near(*radius, 2.0));
            assert!(near(*sweep, 0.5), "{sweep}");
        }
        other => panic!("expected an ARC, got {other:?}"),
    }
}

#[test]
fn a_mode_that_leaves_the_curve_at_the_same_length_is_unchanged() {
    for (label, mode, value) in [
        ("percent 100", PERCENT, 100.0),
        ("total equal to the current length", TOTAL, 5.0),
    ] {
        let (mut app, mut session) = app_with(vec![line_source()]);
        set_mode_value(&mut session, mode, value);
        let before = revision(&app);
        let outcome = lengthen(&mut app, &mut session, vec![ref_of(1)]).expect(label);
        assert!(outcome.changes.is_none(), "{label}: no ChangeSet");
        assert_eq!(outcome.diagnostics[0].code, "lengthen.unchanged", "{label}");
        assert_eq!(revision(&app), before, "{label}: no revision bump");
        assert_eq!(undo_depth(&app), 0, "{label}: no undo entry");
    }
}

#[test]
fn dynamic_mode_moves_the_end_nearest_the_new_point() {
    // The new point (12,3) projects onto (12,0), which is beyond the end, so the
    // end moves and the line becomes 0..12.
    let (mut app, mut session) = app_with(vec![line_source()]);
    lengthen_dynamic(&mut app, &mut session, 1, point(5.0, 0.0), point(12.0, 3.0))
        .expect("the dynamic end lengthens the line");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(near(start.x, 0.0) && near(start.y, 0.0));
            assert!(near(end.x, 12.0) && near(end.y, 0.0), "{end:?}");
        }
        other => panic!("expected a LINE, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn dynamic_mode_can_move_the_start_when_the_new_point_is_near_it() {
    let (mut app, mut session) = app_with(vec![line_source()]);
    lengthen_dynamic(&mut app, &mut session, 1, point(5.0, 0.0), point(-2.0, 1.0))
        .expect("the nearer end is the start");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(near(start.x, -2.0) && near(start.y, 0.0), "{start:?}");
            assert!(near(end.x, 5.0), "{end:?}");
        }
        other => panic!("expected a LINE, got {other:?}"),
    }
    assert!(near(geometry_length(&entity(&app, 1).geometry), 7.0));
}

#[test]
fn dynamic_mode_on_an_arc_projects_the_new_end_radially() {
    let source = curve_entity(
        1,
        LayerId(0),
        arc_geometry(point(0.0, 0.0), 5.0, 0.0, std::f64::consts::FRAC_PI_2),
        "AcDbArc",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    let angle = 0.75 * std::f64::consts::PI;
    lengthen_dynamic(
        &mut app,
        &mut session,
        1,
        point(5.0, 0.0),
        point(5.0 * angle.cos(), 5.0 * angle.sin()),
    )
    .expect("the dynamic arc end is projected");
    match &entity(&app, 1).geometry {
        SemanticGeometry::Arc { sweep, .. } => assert!(near(*sweep, angle), "{sweep}"),
        other => panic!("expected an ARC, got {other:?}"),
    }
}

#[test]
fn dynamic_mode_unchanged_and_projection_failures_are_named() {
    let (mut app, mut session) = app_with(vec![line_source()]);
    let before = revision(&app);
    let outcome = lengthen_dynamic(&mut app, &mut session, 1, point(2.5, 0.0), point(5.0, 1.0))
        .expect("the picked end is already there");
    assert!(outcome.changes.is_none());
    assert_eq!(outcome.diagnostics[0].code, "lengthen.unchanged");
    assert_eq!(revision(&app), before);

    let source = curve_entity(
        1,
        LayerId(0),
        arc_geometry(point(0.0, 0.0), 5.0, 0.0, 1.0),
        "AcDbArc",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    let error = lengthen_dynamic(&mut app, &mut session, 1, point(5.0, 0.0), point(0.0, 0.0))
        .expect_err("the centre has no radial projection");
    assert_invalid(error, "lengthen.projection_failed");
    assert_eq!(undo_depth(&app), 0);
}
