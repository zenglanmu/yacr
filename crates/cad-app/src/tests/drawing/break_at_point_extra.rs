//! Extended `BREAKATPOINT` contracts: the curve kinds that became splittable
//! after the original LINE/ARC/open-LWPOLYLINE lane (CIRCLE, full/partial
//! ELLIPSE, open SPLINE, closed LWPOLYLINE), plus the named refusals that change
//! nothing (closed SPLINE, non-curve kinds, endpoints).
//!
//! Split out of `modify_extra.rs` (the 1000-line code gate). Each test asserts
//! the stored geometry and exactly one transaction / one undo step.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_geometry::NurbsCurve;

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn ent(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

fn build(entities: Vec<DbEntity>, mode: AppMode) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    (app, SessionState::new(DocumentId(1), mode))
}

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn geometry(app: &Application, id: u128) -> SemanticGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("entity must exist")
        .geometry
        .clone()
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p3(x0, y0, 0.0),
        end: p3(x1, y1, 0.0),
    }
}

fn spline() -> SemanticGeometry {
    SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            p3(0.0, 0.0, 0.0),
            p3(2.0, 5.0, 0.0),
            p3(6.0, -3.0, 0.0),
            p3(9.0, 2.0, 0.0),
        ],
        weights: vec![1.0, 0.75, 1.25, 1.0],
        closed: false,
    }
}

fn closed_rectangle() -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points: vec![
            p3(0.0, 0.0, 0.0),
            p3(6.0, 0.0, 0.0),
            p3(6.0, 3.0, 0.0),
            p3(0.0, 3.0, 0.0),
        ],
        bulges: vec![0.0; 4],
        closed: true,
        width: 0.0,
    }
}

fn break_at_point(
    app: &mut Application,
    session: &mut SessionState,
    target: u128,
    point: Point3,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(
            CommandId::BreakAtPoint,
            CommandPayload::BreakAtPoint {
                target: ref_of(target),
                point,
            },
        ),
    )
}

fn assert_point_close(actual: Point3, expected: Point3, label: &str) {
    assert!(
        (actual.x - expected.x).abs() <= 1e-9
            && (actual.y - expected.y).abs() <= 1e-9
            && (actual.z - expected.z).abs() <= 1e-9,
        "{label}: expected {expected:?}, got {actual:?}"
    );
}

fn spline_point(geometry: &SemanticGeometry, t: f64) -> Point3 {
    let SemanticGeometry::Spline {
        degree,
        knots,
        control_points,
        weights,
        ..
    } = geometry
    else {
        panic!("expected a spline fixture");
    };
    let curve = NurbsCurve::new(
        *degree as usize,
        control_points.clone(),
        knots.clone(),
        weights.clone(),
    )
    .expect("the spline fixture is valid");
    let (lo, hi) = curve.domain();
    curve.evaluate(lo + t * (hi - lo))
}

#[test]
fn breakatpoint_splits_an_open_spline_into_two_exact_sub_splines() {
    let source = spline();
    let point = spline_point(&source, 0.5);
    let (mut app, mut session) = build(vec![ent(1, source.clone(), 0)], AppMode::Work);
    break_at_point(&mut app, &mut session, 1, point).unwrap();
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(entity_count(&app), 2);
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let new_id = drawing
        .entities()
        .find(|entity| entity.id != EntityId(1))
        .expect("a second piece")
        .id;
    let SemanticGeometry::Spline {
        degree: first_degree,
        control_points: first_points,
        ..
    } = geometry(&app, 1)
    else {
        panic!("first piece is a SPLINE");
    };
    let SemanticGeometry::Spline {
        degree: second_degree,
        control_points: second_points,
        ..
    } = geometry(&app, new_id.0)
    else {
        panic!("second piece is a SPLINE");
    };
    assert_eq!(first_degree, 3);
    assert_eq!(second_degree, 3);
    assert!(first_points.len() >= 4 && second_points.len() >= 4);
    assert_point_close(*first_points.last().unwrap(), point, "first piece end");
    assert_point_close(second_points[0], point, "second piece start");

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    assert_eq!(entity_count(&app), 1);
    assert_eq!(geometry(&app, 1), source);
}

#[test]
fn breakatpoint_opens_a_circle_into_one_full_turn_arc_at_the_break_point() {
    let circle = SemanticGeometry::Circle {
        center: p3(0.0, 0.0, 0.0),
        normal: p3(0.0, 0.0, 1.0),
        radius: 5.0,
    };
    let (mut app, mut session) = build(vec![ent(1, circle, 0)], AppMode::Work);
    break_at_point(&mut app, &mut session, 1, p3(0.0, 5.0, 0.0)).unwrap();
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(
        entity_count(&app),
        1,
        "a single point cannot leave two non-degenerate pieces of a closed loop"
    );
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap();
    assert_eq!(entity.object.type_key, "AcDbArc");
    let SemanticGeometry::Arc {
        center,
        radius,
        start,
        sweep,
        ..
    } = entity.geometry
    else {
        panic!("the circle must open into an ARC");
    };
    assert_point_close(center, p3(0.0, 0.0, 0.0), "centre");
    assert!((radius - 5.0).abs() <= 1e-9);
    assert!(
        (start - std::f64::consts::FRAC_PI_2).abs() <= 1e-9,
        "the arc starts at the projected break point, got {start}"
    );
    assert!(
        (sweep.abs() - std::f64::consts::TAU).abs() <= 1e-9,
        "one full turn keeps the loop, got {sweep}"
    );
}

#[test]
fn breakatpoint_opens_a_closed_polyline_into_one_open_loop_at_the_break_point() {
    let (mut app, mut session) = build(vec![ent(1, closed_rectangle(), 0)], AppMode::Work);
    // The midpoint of the bottom edge.
    break_at_point(&mut app, &mut session, 1, p3(3.0, 0.0, 0.0)).unwrap();
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(entity_count(&app), 1);
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
        ..
    } = geometry(&app, 1)
    else {
        panic!("the closed polyline opens into one open polyline");
    };
    assert!(!closed, "the loop is opened at the break point");
    assert_eq!(bulges.len(), points.len() - 1, "one bulge per kept segment");
    assert_point_close(points[0], p3(3.0, 0.0, 0.0), "loop start");
    assert_point_close(*points.last().unwrap(), p3(3.0, 0.0, 0.0), "loop end");
    assert!(points.len() >= 5, "every vertex of the rectangle is kept");
}

#[test]
fn breakatpoint_splits_a_partial_ellipse_and_opens_a_full_ellipse() {
    let half_ellipse = SemanticGeometry::Ellipse {
        center: p3(0.0, 0.0, 0.0),
        normal: p3(0.0, 0.0, 1.0),
        major_axis: p3(5.0, 0.0, 0.0),
        ratio: 0.5,
        start: 0.0,
        sweep: std::f64::consts::PI,
    };
    let (mut app, mut session) = build(vec![ent(1, half_ellipse, 0)], AppMode::Work);
    break_at_point(&mut app, &mut session, 1, p3(0.0, 2.5, 0.0)).unwrap();
    assert_eq!(entity_count(&app), 2);
    for id in [1u128, 2] {
        let SemanticGeometry::Ellipse { sweep, .. } = geometry(&app, id) else {
            panic!("both pieces are elliptical arcs");
        };
        assert!(
            (sweep - std::f64::consts::FRAC_PI_2).abs() <= 1e-9,
            "half of the half ellipse, got {sweep}"
        );
    }

    let full_ellipse = SemanticGeometry::Ellipse {
        center: p3(0.0, 0.0, 0.0),
        normal: p3(0.0, 0.0, 1.0),
        major_axis: p3(5.0, 0.0, 0.0),
        ratio: 0.5,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    };
    let (mut app, mut session) = build(vec![ent(1, full_ellipse, 0)], AppMode::Work);
    break_at_point(&mut app, &mut session, 1, p3(0.0, 2.5, 0.0)).unwrap();
    assert_eq!(entity_count(&app), 1);
    let SemanticGeometry::Ellipse { start, sweep, .. } = geometry(&app, 1) else {
        panic!("the full ellipse opens into one elliptical arc");
    };
    assert!((start - std::f64::consts::FRAC_PI_2).abs() <= 1e-9);
    assert!((sweep.abs() - std::f64::consts::TAU).abs() <= 1e-9);
}

#[test]
fn breakatpoint_refuses_unrepresentable_targets_and_endpoints_without_writing() {
    let mut closed_spline = spline();
    if let SemanticGeometry::Spline { closed, .. } = &mut closed_spline {
        *closed = true;
    }
    let text = SemanticGeometry::Text {
        text: "HELLO".into(),
        position: p3(1.0, 2.0, 0.0),
        style: cad_domain::StyleId(0),
        height: 3.0,
        rotation: 0.0,
        font: None,
        h_align: cad_domain::TextAlignH::Left,
        v_align: cad_domain::TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
        line_spacing: cad_domain::TextLineSpacing::default(),
        mirrored: false,
    };
    let cases: Vec<(SemanticGeometry, &str)> = vec![
        (closed_spline, "breakatpoint.closed"),
        (text, "breakatpoint.unsupported"),
    ];
    for (geometry, expected) in cases {
        let (mut app, mut session) = build(vec![ent(1, geometry, 0)], AppMode::Work);
        let rev = revision(&app);
        let err = break_at_point(&mut app, &mut session, 1, p3(1.0, 0.0, 0.0)).unwrap_err();
        match err {
            CadError::InvalidInput(reason) => {
                assert!(reason.starts_with(expected), "{expected}: {reason}")
            }
            CadError::Unsupported(reason) => {
                assert!(reason.starts_with(expected), "{expected}: {reason}")
            }
            other => panic!("unexpected refusal {other:?}"),
        }
        assert_eq!(revision(&app), rev);
        assert_eq!(undo_depth(&app), 0);
        assert_eq!(entity_count(&app), 1);
    }

    // A break point at a LINE endpoint is refused too.
    let (mut app, mut session) = build(vec![ent(1, line(0.0, 0.0, 10.0, 0.0), 0)], AppMode::Work);
    let rev = revision(&app);
    let err = break_at_point(&mut app, &mut session, 1, p3(0.0, 0.0, 0.0)).unwrap_err();
    assert!(
        matches!(err, CadError::InvalidInput(ref reason) if reason.starts_with("breakatpoint.point_at_endpoint"))
    );
    assert_eq!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 0);
}
