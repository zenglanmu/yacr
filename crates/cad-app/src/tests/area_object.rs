//! `AREA OBJECT` contract tests: the exact area of one closed entity.
//!
//! The command is the **selection-based** form of AutoCAD's `AREA` → `Object`
//! (a declared divergence: no interactive pick prompt). Pinned here: the exact
//! values (circle, ellipse, flat and bulged closed polylines), the per-kind
//! `inputs`/`plane` convention, every named refusal (with zero state change) and
//! the fact that measuring opens **no** transaction, undo entry or revision bump.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};

fn entity(id: u128, geometry: SemanticGeometry) -> cad_db::DbEntity {
    cad_db::DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(id),
            type_key: "AcDbTest".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: id as i64,
    }
}

fn circle(id: u128, radius: f64) -> cad_db::DbEntity {
    entity(
        id,
        SemanticGeometry::Circle {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius,
        },
    )
}

fn polyline(id: u128, points: Vec<Point3>, bulges: Vec<f64>, closed: bool) -> cad_db::DbEntity {
    entity(
        id,
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        },
    )
}

/// Ids of every fixture entity (see [`app_with_objects`]).
const CIRCLE_R2: u128 = 1;
const ELLIPSE_3X1: u128 = 2;
const RECT_4X3: u128 = 3;
const BULGED_RECT: u128 = 4;
const OPEN_POLYLINE: u128 = 5;
const SPLINE: u128 = 6;
const NON_PLANAR_POLYLINE: u128 = 7;
const ELLIPTICAL_ARC: u128 = 8;
const TINY_CIRCLE: u128 = 9;
const HUGE_CIRCLE: u128 = 10;
const COLLINEAR_POLYLINE: u128 = 11;
const LINE: u128 = 12;
const CROSSED_POLYLINE: u128 = 13;

fn app_with_objects() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(circle(CIRCLE_R2, 2.0)).unwrap();
    b.insert_entity(entity(
        ELLIPSE_3X1,
        SemanticGeometry::Ellipse {
            center: point(10.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            major_axis: Point3 {
                x: 3.0,
                y: 0.0,
                z: 0.0,
            },
            ratio: 1.0 / 3.0,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
    ))
    .unwrap();
    b.insert_entity(polyline(
        RECT_4X3,
        vec![
            point(0.0, 0.0),
            point(4.0, 0.0),
            point(4.0, 3.0),
            point(0.0, 3.0),
        ],
        vec![0.0; 4],
        true,
    ))
    .unwrap();
    // The same rectangle with a semicircular cap on its 3-unit first edge:
    // r = 1.5, so the cap adds pi * 1.5^2 / 2.
    b.insert_entity(polyline(
        BULGED_RECT,
        vec![
            point(0.0, 0.0),
            point(3.0, 0.0),
            point(3.0, 4.0),
            point(0.0, 4.0),
        ],
        vec![1.0, 0.0, 0.0, 0.0],
        true,
    ))
    .unwrap();
    b.insert_entity(polyline(
        OPEN_POLYLINE,
        vec![point(0.0, 0.0), point(1.0, 0.0), point(1.0, 1.0)],
        vec![0.0; 3],
        false,
    ))
    .unwrap();
    b.insert_entity(entity(
        SPLINE,
        SemanticGeometry::Spline {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control_points: vec![
                point(0.0, 0.0),
                point(1.0, 1.0),
                point(2.0, 1.0),
                point(3.0, 0.0),
            ],
            weights: vec![1.0; 4],
        },
    ))
    .unwrap();
    b.insert_entity(polyline(
        NON_PLANAR_POLYLINE,
        vec![
            point(0.0, 0.0),
            point(4.0, 0.0),
            Point3 {
                x: 4.0,
                y: 3.0,
                z: 1.0,
            },
            point(0.0, 3.0),
        ],
        vec![0.0; 4],
        true,
    ))
    .unwrap();
    b.insert_entity(entity(
        ELLIPTICAL_ARC,
        SemanticGeometry::Ellipse {
            center: point(20.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            major_axis: Point3 {
                x: 3.0,
                y: 0.0,
                z: 0.0,
            },
            ratio: 1.0 / 3.0,
            start: 0.0,
            sweep: std::f64::consts::PI,
        },
    ))
    .unwrap();
    // A radius so small that pi*r^2 underflows to zero: a zero area, refused.
    b.insert_entity(circle(TINY_CIRCLE, 1e-200)).unwrap();
    // A radius whose area overflows to infinity: refused, never reported.
    b.insert_entity(circle(HUGE_CIRCLE, f64::MAX)).unwrap();
    // Three collinear vertices: no winding, so no plane and no bulge frame.
    b.insert_entity(polyline(
        COLLINEAR_POLYLINE,
        vec![point(0.0, 0.0), point(1.0, 0.0), point(2.0, 0.0)],
        vec![0.0; 3],
        true,
    ))
    .unwrap();
    b.insert_entity(entity(
        LINE,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(4.0, 0.0),
        },
    ))
    .unwrap();
    // A closed ring whose chords cross each other (and whose winding is defined).
    b.insert_entity(polyline(
        CROSSED_POLYLINE,
        vec![
            point(0.0, 0.0),
            point(4.0, 0.0),
            point(0.0, 3.0),
            point(3.0, 3.0),
        ],
        vec![0.0; 4],
        true,
    ))
    .unwrap();

    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([11u8; 32]),
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

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn select(
    app: &mut Application,
    session: &mut SessionState,
    entities: &[u128],
) -> CadResult<CommandOutcome> {
    let refs: Vec<SelectionRef> = entities
        .iter()
        .map(|id| SelectionRef {
            document: DocumentId(1),
            entity: EntityId(*id),
            instance: InstancePath::default(),
            sub_element: None,
        })
        .collect();
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(refs),
    )
}

fn area_object(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::AreaObject, CommandPayload::None)
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

/// Select exactly one entity and measure it.
fn measure_one(app: &mut Application, session: &mut SessionState, id: u128) -> CadResult<f64> {
    select(app, session, &[id]).expect("selection");
    let outcome = area_object(app, session)?;
    let record = outcome.measurement.expect("a measurement record");
    assert_eq!(
        record.algorithm,
        cad_db::MeasurementAlgorithm::ObjectArea,
        "object area must never be reported as a polygon/picked measurement"
    );
    Ok(record.value)
}

#[test]
fn a_circle_reports_pi_r_squared_with_no_inputs_and_no_plane() {
    let (mut app, mut session) = app_with_objects();
    let before_revision = revision(&app);

    select(&mut app, &mut session, &[CIRCLE_R2]).expect("select the circle");
    let outcome = area_object(&mut app, &mut session).expect("AREA OBJECT runs");

    let record = outcome.measurement.as_ref().expect("a measurement record");
    assert_eq!(record.algorithm, cad_db::MeasurementAlgorithm::ObjectArea);
    assert!(
        (record.value - 4.0 * std::f64::consts::PI).abs() < 1e-9,
        "{}",
        record.value
    );
    // A closed curve has no vertices: an empty input list is the honest report,
    // and there is no ring to project, so no plane is invented either.
    assert!(record.inputs.is_empty(), "{:?}", record.inputs);
    assert!(record.plane.is_none());
    assert_eq!(record.source, GeometrySource::Analytic);
    assert_eq!(record.precision, Precision::Analytic);
    assert_eq!(record.units, UnitContext::drawing_units());
    // The session records it as its last measurement, like the interactive paths.
    assert_eq!(session.last_measurement(), Some(record));
    assert_eq!(
        outcome.diagnostics[0].code, "measure.object_area",
        "{:?}",
        outcome.diagnostics
    );
    assert_eq!(outcome.diagnostics[0].object, Some(ObjectId(CIRCLE_R2)));
    // A measurement writes nothing: no transaction, no undo entry, no revision.
    assert!(outcome.changes.is_none());
    assert!(outcome.objects.is_empty());
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn an_ellipse_honours_the_stored_ratio() {
    let (mut app, mut session) = app_with_objects();
    let value = measure_one(&mut app, &mut session, ELLIPSE_3X1).expect("ellipse area");
    // a = 3, b = a * 1/3 = 1 => pi * 3 * 1.
    assert!((value - 3.0 * std::f64::consts::PI).abs() < 1e-9, "{value}");
}

#[test]
fn a_closed_rectangle_polyline_reports_its_exact_area() {
    let (mut app, mut session) = app_with_objects();
    select(&mut app, &mut session, &[RECT_4X3]).expect("select");
    let outcome = area_object(&mut app, &mut session).expect("AREA OBJECT runs");
    let record = outcome.measurement.as_ref().expect("a record");
    assert!((record.value - 12.0).abs() < 1e-9, "{}", record.value);
    // The ring's **own** vertices travel (world coordinates, no duplicated closing
    // vertex, never synthesised picks), plus the ring's own plane.
    assert_eq!(record.inputs.len(), 4);
    assert_eq!(record.inputs[0], point(0.0, 0.0));
    assert_eq!(record.inputs[2], point(4.0, 3.0));
    let plane = record.plane.expect("the ring's own plane");
    assert_eq!(plane.origin, point(0.0, 0.0));
    // Right-handed: u x v is the ring normal (+Z for an XY-parallel ring).
    let normal = camera::cross(plane.u, plane.v);
    assert!((normal.z - 1.0).abs() < 1e-12, "{normal:?}");
}

#[test]
fn a_closed_bulged_polyline_reports_the_analytic_bulge_area() {
    let (mut app, mut session) = app_with_objects();
    let value = measure_one(&mut app, &mut session, BULGED_RECT).expect("bulged area");
    // 3 x 4 rectangle + a semicircular cap on the 3-unit edge: r = 1.5.
    let expected = 12.0 + std::f64::consts::PI * 1.5 * 1.5 / 2.0;
    assert!((value - expected).abs() < 1e-9, "{value} != {expected}");
}

#[test]
fn measuring_needs_exactly_one_selected_addressable_entity() {
    let (mut app, mut session) = app_with_objects();
    let before_revision = revision(&app);

    // Nothing selected.
    assert!(session.selection.is_empty());
    let error = area_object(&mut app, &mut session).expect_err("0 selected is a refusal");
    assert!(
        error.to_string().contains("area.object.selection_empty"),
        "{error}"
    );

    // Two selected: no partial result, no "first one wins".
    select(&mut app, &mut session, &[CIRCLE_R2, RECT_4X3]).expect("select two");
    assert_eq!(session.selection.len(), 2);
    let error = area_object(&mut app, &mut session).expect_err("2 selected is a refusal");
    assert!(
        error.to_string().contains("area.object.selection_multiple"),
        "{error}"
    );

    // A payload is refused: the selection is the only input.
    select(&mut app, &mut session, &[CIRCLE_R2]).expect("select");
    let error = execute(
        &mut app,
        &mut session,
        CommandId::AreaObject,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    )
    .expect_err("a payload is refused");
    assert!(
        error.to_string().contains("area.object.invalid_payload"),
        "{error}"
    );

    // A sub-element has no addressable entity identity, so it has no stored area.
    session.selection.replace(vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(CIRCLE_R2),
        instance: InstancePath::default(),
        sub_element: Some(SubElementId {
            source_key: "face:0".into(),
            topology_revision: Revision(0),
        }),
    }]);
    let error = area_object(&mut app, &mut session).expect_err("a sub-element ref is refused");
    assert!(
        error.to_string().contains("area.object.not_addressable"),
        "{error}"
    );

    // A block-instance member is not addressable either.
    session.selection.replace(vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(CIRCLE_R2),
        instance: InstancePath(vec![EntityId(1)]),
        sub_element: None,
    }]);
    let error = area_object(&mut app, &mut session).expect_err("an instance member is refused");
    assert!(
        error.to_string().contains("area.object.not_addressable"),
        "{error}"
    );

    // A ref for another document is refused rather than resolved here.
    session.selection.replace(vec![SelectionRef {
        document: DocumentId(2),
        entity: EntityId(CIRCLE_R2),
        instance: InstancePath::default(),
        sub_element: None,
    }]);
    let error = area_object(&mut app, &mut session).expect_err("another document's ref");
    assert!(
        error.to_string().contains("area.object.target_missing"),
        "{error}"
    );

    // A stale ref (the entity does not exist) is refused by name too.
    session.selection.replace(vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(9999),
        instance: InstancePath::default(),
        sub_element: None,
    }]);
    let error = area_object(&mut app, &mut session).expect_err("a missing entity");
    assert!(
        error.to_string().contains("area.object.target_missing"),
        "{error}"
    );

    // No refusal measured anything or wrote anything.
    assert!(session.last_measurement().is_none());
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn unsupported_and_degenerate_kinds_are_refused_by_name() {
    let (mut app, mut session) = app_with_objects();
    let before_revision = revision(&app);

    for (id, code) in [
        (OPEN_POLYLINE, "area.object.polyline_open"),
        (SPLINE, "area.object.unsupported_kind"),
        (LINE, "area.object.unsupported_kind"),
        (ELLIPTICAL_ARC, "area.object.ellipse_arc"),
        (NON_PLANAR_POLYLINE, "area.object.polyline_non_planar"),
        (COLLINEAR_POLYLINE, "area.object.polyline_plane"),
        (CROSSED_POLYLINE, "area.object.self_intersecting"),
        (TINY_CIRCLE, "area.object.zero_area"),
        (HUGE_CIRCLE, "area.object.degenerate"),
    ] {
        select(&mut app, &mut session, &[id]).expect("select");
        let error = area_object(&mut app, &mut session)
            .err()
            .unwrap_or_else(|| panic!("entity {id} must be refused"));
        assert!(
            error.to_string().contains(code),
            "entity {id}: expected {code}, got {error}"
        );
        // A refusal records nothing: no last measurement, no state change.
        assert!(session.last_measurement().is_none(), "entity {id}");
    }

    // Every refusal left the drawing and history untouched.
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn an_ellipse_with_a_degenerate_ratio_or_axis_is_refused() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for (id, major_axis, ratio) in [
        // A zero-length major axis.
        (1u128, 0.0, 0.5),
        // A ratio that is not positive: the minor semi-axis is undefined.
        (2, 3.0, 0.0),
        (3, 3.0, -0.5),
    ] {
        b.insert_entity(entity(
            id,
            SemanticGeometry::Ellipse {
                center: point(0.0, 0.0),
                normal: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                major_axis: Point3 {
                    x: major_axis,
                    y: 0.0,
                    z: 0.0,
                },
                ratio,
                start: 0.0,
                sweep: std::f64::consts::TAU,
            },
        ))
        .unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([12u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);

    for id in [1u128, 2, 3] {
        select(&mut app, &mut session, &[id]).expect("select");
        let error = area_object(&mut app, &mut session).expect_err("a degenerate ellipse");
        assert!(
            error.to_string().contains("area.object.degenerate"),
            "entity {id}: {error}"
        );
    }
}

#[test]
fn an_ellipse_ratio_above_one_is_measured_verbatim_and_not_swapped() {
    // A source may name the longer axis "minor" (ratio > 1). The value is still
    // exactly pi * a * (a * ratio); nothing is swapped and nothing is refused.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity(
        1,
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            major_axis: Point3 {
                x: 3.0,
                y: 0.0,
                z: 0.0,
            },
            ratio: 1.5,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
    ))
    .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([13u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);

    let value = measure_one(&mut app, &mut session, 1).expect("the ratio is used verbatim");
    let expected = std::f64::consts::PI * 3.0 * 4.5;
    assert!((value - expected).abs() < 1e-9, "{value} != {expected}");
}

#[test]
fn area_object_is_work_only_like_the_other_measurements() {
    assert!(CommandId::AreaObject.requires_work_mode());
    let (mut app, mut session) = app_with_objects();
    session.switch_mode(AppMode::Viewer).expect("Viewer");
    select(&mut app, &mut session, &[CIRCLE_R2]).expect("select");
    let error = area_object(&mut app, &mut session).expect_err("Viewer refuses AREA OBJECT");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert!(session.last_measurement().is_none());
}
