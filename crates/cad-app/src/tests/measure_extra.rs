//! `MASSPROP` / `MEASUREGEOM` contract tests (AutoCAD 2026 inquiry, 2026-11-07).
//!
//! Pinned here: the exact analytic values (circle, ellipse, straight and bulged
//! closed polylines), the coplanarity rule, every named refusal with **zero**
//! state change, the fact that both commands are read-only (no transaction,
//! revision bump or undo entry) and the explicit `VOLUME` not-implemented
//! refusal.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};

const CIRCLE_R2: u128 = 1;
const ELLIPSE_3X1: u128 = 2;
const RECT_3X4: u128 = 3;
const BULGED_RECT: u128 = 4;
const OPEN_POLYLINE: u128 = 5;
const ARC: u128 = 6;
const CIRCLE_HIGH: u128 = 7;
const CIRCLE_R5: u128 = 8;
const NON_PLANAR_POLYLINE: u128 = 9;
// Region-extension fixtures (MASSPROP): a closed degree-1 spline that is exactly
// the 3x4 rectangle, its open twin, and three hatches.
const SPLINE_SQUARE: u128 = 10;
const SPLINE_OPEN: u128 = 11;
const HATCH_CIRCLE: u128 = 12;
const HATCH_ISLAND: u128 = 13;
const HATCH_SAMPLED: u128 = 14;

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

fn circle(id: u128, center: Point3, radius: f64) -> cad_db::DbEntity {
    entity(
        id,
        SemanticGeometry::Circle {
            center,
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

/// A clamped-uniform spline over `control_points` (`weights` empty = all 1).
fn spline(id: u128, control_points: Vec<Point3>, degree: u32) -> cad_db::DbEntity {
    let knots = cad_geometry::clamped_uniform_knots(control_points.len(), degree as usize);
    entity(
        id,
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights: Vec::new(),
        },
    )
}

/// A solid hatch on the world XY plane (`z = 0`) with the given loops.
fn hatch_entity(id: u128, loops: Vec<HatchLoop>) -> cad_db::DbEntity {
    let hatch = HatchGeometry::new(
        loops,
        HatchPlane::new(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            0.0,
        ),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::Predefined,
    )
    .expect("fixture hatch must be valid");
    entity(id, SemanticGeometry::Hatch(Box::new(hatch)))
}

/// A radius-1 circle at `(5, 3)` in the hatch plane: four exact quarter arcs.
fn circle_loop() -> HatchLoop {
    HatchLoop::new(
        vec![[6.0, 3.0], [5.0, 4.0], [4.0, 3.0], [5.0, 2.0]],
        vec![std::f64::consts::FRAC_PI_8.tan(); 4],
        HatchLoopFlags(HatchLoopFlags::POLYLINE),
    )
}

/// A loop whose stored `source_edges` names an arc: the import path samples such
/// an edge into vertices, so the measurement must call itself approximate.
fn sampled_loop() -> HatchLoop {
    let mut loop_ = HatchLoop::new(
        vec![[8.0, 0.0], [10.0, 0.0], [10.0, 2.0], [8.0, 2.0]],
        vec![0.0; 4],
        HatchLoopFlags(HatchLoopFlags::POLYLINE),
    );
    loop_.source_edges = Some(vec![BoundaryEdge::CircularArc {
        center: [9.0, 1.0],
        radius: 1.0,
        start_angle: 0.0,
        end_angle: std::f64::consts::TAU,
        ccw: true,
    }]);
    loop_
}

fn app_with_objects() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(circle(CIRCLE_R2, point(0.0, 0.0), 2.0))
        .unwrap();
    b.insert_entity(circle(CIRCLE_R5, point(0.0, 0.0), 5.0))
        .unwrap();
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
        RECT_3X4,
        vec![
            point(0.0, 0.0),
            point(3.0, 0.0),
            point(3.0, 4.0),
            point(0.0, 4.0),
        ],
        vec![0.0; 4],
        true,
    ))
    .unwrap();
    // A 3x4 rectangle whose bottom edge is replaced by a semicircular cap of
    // radius 1.5 bowing **outside** (bulge +1 is right of the chord).
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
        ARC,
        SemanticGeometry::Arc {
            center: point(20.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 4.0,
            start: 0.0,
            sweep: std::f64::consts::FRAC_PI_2,
        },
    ))
    .unwrap();
    b.insert_entity(polyline(
        NON_PLANAR_POLYLINE,
        vec![
            point(0.0, 0.0),
            point(3.0, 0.0),
            Point3 {
                x: 3.0,
                y: 4.0,
                z: 1.0,
            },
            point(0.0, 4.0),
        ],
        vec![0.0; 4],
        true,
    ))
    .unwrap();
    b.insert_entity(circle(
        CIRCLE_HIGH,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 5.0,
        },
        1.0,
    ))
    .unwrap();
    // A degree-1 clamped spline whose control net closes on itself: the sampled
    // curve is exactly the 3x4 rectangle (a good exactness control for the
    // sampled-region path), and its open twin is the refusal fixture.
    b.insert_entity(spline(
        SPLINE_SQUARE,
        vec![
            point(0.0, 0.0),
            point(3.0, 0.0),
            point(3.0, 4.0),
            point(0.0, 4.0),
            point(0.0, 0.0),
        ],
        1,
    ))
    .unwrap();
    b.insert_entity(spline(
        SPLINE_OPEN,
        vec![point(0.0, 0.0), point(3.0, 0.0), point(3.0, 4.0)],
        1,
    ))
    .unwrap();
    // HATCH: a circle of radius 1 at (5, 3) as four exact quarter-arc bulges.
    b.insert_entity(hatch_entity(HATCH_CIRCLE, vec![circle_loop()]))
        .unwrap();
    // HATCH: the 3x4 rectangle with a 2x2 island, both written counter-clockwise
    // (nesting parity, not the stored winding, decides which one subtracts).
    b.insert_entity(hatch_entity(
        HATCH_ISLAND,
        vec![
            HatchLoop::new(
                vec![[0.0, 0.0], [3.0, 0.0], [3.0, 4.0], [0.0, 4.0]],
                vec![0.0; 4],
                HatchLoopFlags(HatchLoopFlags::POLYLINE | HatchLoopFlags::EXTERNAL),
            ),
            HatchLoop::new(
                vec![[1.0, 1.0], [2.0, 1.0], [2.0, 3.0], [1.0, 3.0]],
                vec![0.0; 4],
                HatchLoopFlags(HatchLoopFlags::POLYLINE),
            ),
        ],
    ))
    .unwrap();
    // HATCH: an imported loop whose analytic source edge is an arc, i.e. a
    // boundary the importer sampled into vertices (reported as approximate).
    b.insert_entity(hatch_entity(HATCH_SAMPLED, vec![sampled_loop()]))
        .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([71u8; 32]),
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

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn assert_no_write(app: &Application, session: &SessionState, before: Revision) {
    let after = app.workspace.documents[&DocumentId(1)].drawing.revision();
    assert_eq!(after, before, "no revision bump");
    assert!(
        !app.can_undo(&session.document),
        "no undo entry may be recorded"
    );
}

fn diagnostic_text(outcome: &CommandOutcome, code: &str) -> String {
    outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("missing diagnostic {code}: {:?}", outcome.diagnostics))
        .message
        .clone()
}

#[test]
fn massprop_reports_a_circle_exactly_and_viewer_legal() {
    let (mut app, _) = app_with_objects();
    let mut session = SessionState::new(DocumentId(1), AppMode::Viewer);
    assert!(!CommandId::MassProp.requires_work_mode());
    let before = revision(&app);
    select(&mut app, &mut session, &[CIRCLE_R2]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("MASSPROP is Viewer-legal");

    let object = diagnostic_text(&outcome, "massprop.object");
    assert!(object.contains("area=12.566371"), "{object}");
    assert!(object.contains("perimeter=12.566371"), "{object}");
    assert!(
        object.contains("bbox min=(-2.000000, -2.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("max=(2.000000, 2.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("centroid=(0.000000, 0.000000, 0.000000)"),
        "{object}"
    );
    let not_implemented = diagnostic_text(&outcome, "massprop.not_implemented");
    assert!(
        not_implemented.contains("moments/products of inertia"),
        "{not_implemented}"
    );
    assert!(
        not_implemented.contains("NOT IMPLEMENTED"),
        "{not_implemented}"
    );
    let summary = diagnostic_text(&outcome, "massprop.summary");
    assert!(summary.contains("1 region(s)"), "{summary}");
    assert!(
        outcome.measurement.is_none(),
        "no scalar measurement record"
    );
    assert!(outcome.point_report.is_none());
    assert!(outcome.changes.is_none());
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_ellipse_perimeter_is_labelled_approximate() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    select(&mut app, &mut session, &[ELLIPSE_3X1]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("a full ellipse is supported");

    let object = diagnostic_text(&outcome, "massprop.object");
    assert!(object.contains("area=9.424778"), "{object}");
    assert!(
        object.contains("perimeter=~"),
        "the ellipse perimeter is approximate: {object}"
    );
    let approx = diagnostic_text(&outcome, "massprop.approx_ellipse_perimeter");
    assert!(approx.contains("Ramanujan"), "{approx}");
    assert!(
        object.contains("bbox min=(7.000000, -1.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("max=(13.000000, 1.000000, 0.000000)"),
        "{object}"
    );
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_closed_polyline_area_perimeter_and_centroid_are_exact() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    select(&mut app, &mut session, &[RECT_3X4]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("a closed straight polyline is supported");
    let object = diagnostic_text(&outcome, "massprop.object");
    assert!(object.contains("area=12.000000"), "{object}");
    assert!(object.contains("perimeter=14.000000"), "{object}");
    assert!(
        object.contains("centroid=(1.500000, 2.000000, 0.000000)"),
        "{object}"
    );
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_bulged_polyline_uses_the_exact_arc_area_length_and_centroid() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    select(&mut app, &mut session, &[BULGED_RECT]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("a bulged closed polyline is supported");
    let object = diagnostic_text(&outcome, "massprop.object");
    // 3x4 chord ring + a semicircular cap (r = 1.5): 12 + pi*1.5^2/2.
    let area = 12.0 + std::f64::consts::PI * 1.5 * 1.5 / 2.0;
    assert!(object.contains(&format!("area={area:.6}")), "{object}");
    // Straight edges 4 + 3 + 4 = 11 plus the semicircle's pi*1.5.
    let perimeter = 11.0 + std::f64::consts::PI * 1.5;
    assert!(
        object.contains(&format!("perimeter={perimeter:.6}")),
        "{object}"
    );
    // The cap bows outside the bottom edge, so the bounding box reaches y = -1.5
    // (a vertex-only box would stop at y = 0).
    assert!(
        object.contains("bbox min=(0.000000, -1.500000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("max=(3.000000, 4.000000, 0.000000)"),
        "{object}"
    );
    // Area-weighted centroid: (12*(1.5,2) + 1.5^2*pi/2*(1.5,-4r/3pi)) / area.
    let cap_area = std::f64::consts::PI * 1.5 * 1.5 / 2.0;
    let y = (12.0 * 2.0 + cap_area * (-4.0 * 1.5 / (3.0 * std::f64::consts::PI))) / area;
    assert!(
        object.contains(&format!("centroid=(1.500000, {y:.6}, 0.000000)")),
        "{object}"
    );
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_summary_combines_coplanar_regions_and_refuses_non_coplanar_ones() {
    let (mut app, mut session) = app_with_objects();
    select(&mut app, &mut session, &[CIRCLE_R2, RECT_3X4]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("both regions are on z = 0");
    let summary = diagnostic_text(&outcome, "massprop.summary");
    let total = std::f64::consts::PI * 4.0 + 12.0;
    assert!(summary.contains("2 region(s)"), "{summary}");
    assert!(summary.contains(&format!("area={total:.6}")), "{summary}");

    let (mut app, mut session) = app_with_objects();
    select(&mut app, &mut session, &[CIRCLE_R2, CIRCLE_HIGH]).unwrap();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("z = 0 and z = 5 are not coplanar");
    let text = format!("{error:?}");
    assert!(text.contains("massprop.not_coplanar"), "{text}");
}

#[test]
fn massprop_names_every_unsupported_input() {
    let (mut app, mut session) = app_with_objects();
    let empty = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("an empty selection is refused");
    assert!(
        format!("{empty:?}").contains("massprop.selection"),
        "{empty:?}"
    );

    select(&mut app, &mut session, &[OPEN_POLYLINE]).unwrap();
    let open = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("an open polyline encloses no area");
    assert!(
        format!("{open:?}").contains("massprop.open_polyline"),
        "{open:?}"
    );

    select(&mut app, &mut session, &[ARC]).unwrap();
    let arc = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("an arc is an open curve");
    assert!(format!("{arc:?}").contains("massprop.arc"), "{arc:?}");

    select(&mut app, &mut session, &[CIRCLE_R2]).unwrap();
    let payload = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    )
    .expect_err("MASSPROP takes no payload");
    assert!(
        format!("{payload:?}").contains("massprop.invalid_payload"),
        "{payload:?}"
    );

    // A block-instance member has no addressable entity identity.
    let refs = vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(CIRCLE_R2),
        instance: InstancePath(vec![EntityId(99)]),
        sub_element: None,
    }];
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(refs),
    )
    .unwrap();
    let not_addressable = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("a block member is not addressable");
    assert!(
        format!("{not_addressable:?}").contains("massprop.not_addressable"),
        "{not_addressable:?}"
    );
}

#[test]
fn measuregeom_radius_reports_circle_and_arc_families() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    assert!(!CommandId::MeasureGeom.requires_work_mode());
    select(&mut app, &mut session, &[CIRCLE_R2]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MeasureGeom,
        CommandPayload::MeasureGeom(MeasureGeomOption::Radius),
    )
    .expect("a circle has a radius report");
    let circle = diagnostic_text(&outcome, "measuregeom.radius");
    assert!(
        circle.contains("centre=(0.000000, 0.000000, 0.000000)"),
        "{circle}"
    );
    assert!(circle.contains("radius=2.000000"), "{circle}");
    assert!(circle.contains("diameter=4.000000"), "{circle}");
    assert!(circle.contains("circumference=12.566371"), "{circle}");
    assert!(!circle.contains("arc_length"), "{circle}");
    assert_no_write(&app, &session, before);

    select(&mut app, &mut session, &[ARC]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MeasureGeom,
        CommandPayload::MeasureGeom(MeasureGeomOption::Radius),
    )
    .expect("an arc has a radius report");
    let arc = diagnostic_text(&outcome, "measuregeom.radius");
    assert!(arc.contains("radius=4.000000"), "{arc}");
    // A quarter arc of r = 4 has length 2*pi.
    assert!(arc.contains("arc_length=6.283185"), "{arc}");
    assert_no_write(&app, &session, before);
}

#[test]
fn measuregeom_radius_is_viewer_legal_and_writes_nothing() {
    let (mut app, _) = app_with_objects();
    let mut session = SessionState::new(DocumentId(1), AppMode::Viewer);
    let before = revision(&app);
    select(&mut app, &mut session, &[CIRCLE_R2]).unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::MeasureGeom,
        CommandPayload::MeasureGeom(MeasureGeomOption::Radius),
    )
    .expect("MEASUREGEOM RADIUS is a read-only report and Viewer-legal");
    assert_no_write(&app, &session, before);
}

#[test]
fn measuregeom_radius_refuses_empty_multiple_and_wrong_kinds() {
    let (mut app, mut session) = app_with_objects();
    let empty = execute(
        &mut app,
        &mut session,
        CommandId::MeasureGeom,
        CommandPayload::MeasureGeom(MeasureGeomOption::Radius),
    )
    .expect_err("an empty selection is refused");
    assert!(
        format!("{empty:?}").contains("measuregeom.radius.selection_empty"),
        "{empty:?}"
    );

    select(&mut app, &mut session, &[CIRCLE_R2, ARC]).unwrap();
    let multiple = execute(
        &mut app,
        &mut session,
        CommandId::MeasureGeom,
        CommandPayload::MeasureGeom(MeasureGeomOption::Radius),
    )
    .expect_err("two objects are refused");
    assert!(
        format!("{multiple:?}").contains("measuregeom.radius.selection_multiple"),
        "{multiple:?}"
    );

    select(&mut app, &mut session, &[RECT_3X4]).unwrap();
    let wrong = execute(
        &mut app,
        &mut session,
        CommandId::MeasureGeom,
        CommandPayload::MeasureGeom(MeasureGeomOption::Radius),
    )
    .expect_err("a polyline has no radius");
    assert!(
        format!("{wrong:?}").contains("measuregeom.radius.not_arc_circle"),
        "{wrong:?}"
    );
}

#[test]
fn measuregeom_volume_is_an_explicit_unsupported_and_bare_needs_option() {
    let (mut app, mut session) = app_with_objects();
    let volume = execute(
        &mut app,
        &mut session,
        CommandId::MeasureGeom,
        CommandPayload::MeasureGeom(MeasureGeomOption::Volume),
    )
    .expect_err("VOLUME is not implemented");
    assert!(
        format!("{volume:?}").contains("measuregeom.volume_unsupported"),
        "{volume:?}"
    );

    let bare = execute(
        &mut app,
        &mut session,
        CommandId::MeasureGeom,
        CommandPayload::None,
    )
    .expect_err("a bare MEASUREGEOM needs an option");
    assert!(
        format!("{bare:?}").contains("measuregeom.needs_option"),
        "{bare:?}"
    );

    // The four tool options are routed by the session, not by a command payload;
    // a direct command-layer call is refused by name instead of starting a tool
    // from the wrong layer.
    let tool_payload = execute(
        &mut app,
        &mut session,
        CommandId::MeasureGeom,
        CommandPayload::MeasureGeom(MeasureGeomOption::Distance),
    )
    .expect_err("DISTANCE starts the session tool, not a command report");
    assert!(
        format!("{tool_payload:?}").contains("measuregeom.invalid_payload"),
        "{tool_payload:?}"
    );
    assert_eq!(session.tool_snapshot(), ToolSnapshot::Idle);
    assert!(!app.can_undo(&session.document));
}

// --- MASSPROP region extension (2026-11-12) ------------------------------

#[test]
fn massprop_closed_spline_is_a_labelled_sample_ring() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    select(&mut app, &mut session, &[SPLINE_SQUARE]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("a closed spline is measured through its sample ring");

    let object = diagnostic_text(&outcome, "massprop.object");
    assert!(object.contains("(closed spline (sampled))"), "{object}");
    // The degree-1 control net closes on itself, so the sample ring is exactly the
    // 3x4 rectangle: 12 area, 14 perimeter, centroid in its centre.
    assert!(object.contains("area=12.000000"), "{object}");
    assert!(
        object.contains("perimeter=~14.000000"),
        "the sampled perimeter is labelled approximate: {object}"
    );
    assert!(
        object.contains("centroid=(1.500000, 2.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("bbox min=(0.000000, 0.000000, 0.000000)"),
        "{object}"
    );
    let note = diagnostic_text(&outcome, "massprop.approx_spline_region");
    assert!(note.contains("sample ring"), "{note}");
    assert!(note.contains("not closed-form"), "{note}");
    assert!(
        !outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "massprop.approx_ellipse_perimeter"),
        "a spline must not borrow the ellipse's approximation note"
    );
    // The summary counts it as approximate because its perimeter is sampled.
    let summary = diagnostic_text(&outcome, "massprop.summary");
    assert!(summary.contains("perimeter=~"), "{summary}");
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_refuses_an_open_or_unevaluable_spline_by_name() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);

    select(&mut app, &mut session, &[SPLINE_OPEN]).unwrap();
    let open = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("an open spline encloses no region");
    assert!(
        format!("{open:?}").contains("massprop.open_spline"),
        "{open:?}"
    );
    assert_no_write(&app, &session, before);

    // A degree-0 spline cannot be evaluated; MASSPROP refuses instead of falling
    // back to a clamped-uniform curve (the display tessellator's fallback).
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(2));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(spline(64, vec![point(0.0, 0.0), point(1.0, 0.0)], 0))
        .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([72u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    select(&mut app, &mut session, &[64]).unwrap();
    let invalid = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect_err("a degree-0 spline has no evaluable curve");
    assert!(
        format!("{invalid:?}").contains("massprop.spline_invalid"),
        "{invalid:?}"
    );
}

#[test]
fn massprop_hatch_region_is_the_exact_net_loop_area() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    select(&mut app, &mut session, &[HATCH_CIRCLE]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("a hatch region is measured from its own loops");

    let object = diagnostic_text(&outcome, "massprop.object");
    assert!(object.contains("(hatch region)"), "{object}");
    // Four exact quarter arcs of r = 1: area pi, perimeter 2*pi.
    assert!(
        object.contains(&format!("area={:.6}", std::f64::consts::PI)),
        "{object}"
    );
    assert!(
        object.contains(&format!("perimeter={:.6}", std::f64::consts::TAU)),
        "{object}"
    );
    assert!(
        object.contains("bbox min=(4.000000, 2.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("max=(6.000000, 4.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        object.contains("centroid=(5.000000, 3.000000, 0.000000)"),
        "{object}"
    );
    // A created hatch has analytic loop data, so nothing is labelled approximate.
    assert!(
        !outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "massprop.approx_hatch_boundary"),
        "{:?}",
        outcome.diagnostics
    );
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_hatch_subtracts_islands_by_nesting_parity_not_winding() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);
    select(&mut app, &mut session, &[HATCH_ISLAND]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("the two-loop hatch is a region");

    let object = diagnostic_text(&outcome, "massprop.object");
    // Both loops are written counter-clockwise; nesting parity still subtracts
    // the 2x2 island from the 3x4 rectangle: 12 - 2 = 10.
    assert!(object.contains("area=10.000000"), "{object}");
    // Only the loop carrying the external flag contributes to the perimeter.
    assert!(object.contains("perimeter=14.000000"), "{object}");
    assert!(
        object.contains("centroid=(1.500000, 2.000000, 0.000000)"),
        "{object}"
    );
    assert!(
        !outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "massprop.hatch_perimeter_rule"),
        "{:?}",
        outcome.diagnostics
    );
    assert_no_write(&app, &session, before);
}

#[test]
fn massprop_hatch_names_a_sampled_import_boundary_as_approximate() {
    let (mut app, mut session) = app_with_objects();
    select(&mut app, &mut session, &[HATCH_SAMPLED]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MassProp,
        CommandPayload::None,
    )
    .expect("the imported hatch is still a region");
    let note = diagnostic_text(&outcome, "massprop.approx_hatch_boundary");
    assert!(note.contains("sampled"), "{note}");
    assert!(note.contains("not exact"), "{note}");
    // The fixture's loops carry no external flag bit, so the outer-boundary rule
    // is stated instead of guessed silently.
    let rule = diagnostic_text(&outcome, "massprop.hatch_perimeter_rule");
    assert!(rule.contains("largest-area loop"), "{rule}");
}

#[test]
fn measuregeom_radius_completes_the_arc_family() {
    let (mut app, mut session) = app_with_objects();
    let before = revision(&app);

    select(&mut app, &mut session, &[ARC]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MeasureGeom,
        CommandPayload::MeasureGeom(MeasureGeomOption::Radius),
    )
    .expect("an arc reports its radius family");
    let arc = diagnostic_text(&outcome, "measuregeom.radius");
    // A quarter arc of r = 4: sweep 90 degrees, chord 2*4*sin(45) = 5.656854.
    assert!(arc.contains("sweep_deg=90.000000"), "{arc}");
    assert!(arc.contains("chord=5.656854"), "{arc}");
    assert!(
        !arc.contains("area="),
        "an arc encloses no area, so none is printed: {arc}"
    );
    assert_no_write(&app, &session, before);

    let (mut app, mut session) = app_with_objects();
    select(&mut app, &mut session, &[CIRCLE_R2]).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MeasureGeom,
        CommandPayload::MeasureGeom(MeasureGeomOption::Radius),
    )
    .expect("a circle reports its radius family");
    let circle = diagnostic_text(&outcome, "measuregeom.radius");
    assert!(circle.contains("area=12.566371"), "{circle}");
}
