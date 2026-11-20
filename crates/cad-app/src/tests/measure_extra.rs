//! `MEASUREGEOM` contract tests plus the shared inquiry-test fixtures.
//!
//! The `MASSPROP` contracts live in the sibling [`super::measure_massprop`]
//! module (the split keeps both files inside the code-size limit); this module
//! keeps the fixture database every one of those tests selects from and the
//! `MEASUREGEOM RADIUS`/`VOLUME` reports.
//!
//! Pinned here: the radius family of a circle and an arc, the empty/multiple/
//! wrong-kind refusals, the fact that both commands are read-only (no
//! transaction, revision bump or undo entry) and the explicit `VOLUME`
//! not-implemented refusal.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};

pub(super) const CIRCLE_R2: u128 = 1;
pub(super) const ELLIPSE_3X1: u128 = 2;
pub(super) const RECT_3X4: u128 = 3;
pub(super) const BULGED_RECT: u128 = 4;
pub(super) const OPEN_POLYLINE: u128 = 5;
pub(super) const ARC: u128 = 6;
pub(super) const CIRCLE_HIGH: u128 = 7;
pub(super) const CIRCLE_R5: u128 = 8;
pub(super) const NON_PLANAR_POLYLINE: u128 = 9;
// Region-extension fixtures (MASSPROP): a closed degree-1 spline that is exactly
// the 3x4 rectangle, its open twin, and three hatches.
pub(super) const SPLINE_SQUARE: u128 = 10;
pub(super) const SPLINE_OPEN: u128 = 11;
pub(super) const HATCH_CIRCLE: u128 = 12;
pub(super) const HATCH_ISLAND: u128 = 13;
pub(super) const HATCH_SAMPLED: u128 = 14;
// Mass-property fixtures: a rectangle rotated 45 degrees about its own centroid
// (this one is not axis aligned, so its product of inertia is nonzero), and a
// circle whose area underflows to zero.
pub(super) const ROTATED_RECT: u128 = 15;
pub(super) const TINY_CIRCLE: u128 = 16;

pub(super) fn entity(id: u128, geometry: SemanticGeometry) -> cad_db::DbEntity {
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

pub(super) fn circle(id: u128, center: Point3, radius: f64) -> cad_db::DbEntity {
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

pub(super) fn polyline(
    id: u128,
    points: Vec<Point3>,
    bulges: Vec<f64>,
    closed: bool,
) -> cad_db::DbEntity {
    entity(
        id,
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            width: 0.0,
        },
    )
}

/// A clamped-uniform spline over `control_points` (`weights` empty = all 1).
pub(super) fn spline(id: u128, control_points: Vec<Point3>, degree: u32) -> cad_db::DbEntity {
    entity(
        id,
        SemanticGeometry::Spline {
            degree,
            knots: cad_geometry::clamped_uniform_knots(control_points.len(), degree as usize),
            control_points,
            weights: Vec::new(),
            closed: false,
        },
    )
}

/// A solid hatch on the world XY plane (`z = 0`) with the given loops.
pub(super) fn hatch_entity(id: u128, loops: Vec<HatchLoop>) -> cad_db::DbEntity {
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
pub(super) fn circle_loop() -> HatchLoop {
    HatchLoop::new(
        vec![[6.0, 3.0], [5.0, 4.0], [4.0, 3.0], [5.0, 2.0]],
        vec![std::f64::consts::FRAC_PI_8.tan(); 4],
        HatchLoopFlags(HatchLoopFlags::POLYLINE),
    )
}

/// A loop whose stored `source_edges` names an arc: the import path samples such
/// an edge into vertices, so the measurement must call itself approximate.
pub(super) fn sampled_loop() -> HatchLoop {
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

pub(super) fn app_with_objects() -> (Application, SessionState) {
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
    // The 3x4 rectangle rotated by 45 degrees about its centroid (30, 0). Its
    // second moments are Ix = Iy = (9 + 16)/2 and Ixy = (9 - 16)/2.
    let cos = std::f64::consts::FRAC_1_SQRT_2;
    let sin = std::f64::consts::FRAC_1_SQRT_2;
    let rotated: Vec<Point3> = [(-1.5, -2.0), (1.5, -2.0), (1.5, 2.0), (-1.5, 2.0)]
        .iter()
        .map(|(x, y)| point(30.0 + x * cos - y * sin, x * sin + y * cos))
        .collect();
    b.insert_entity(polyline(ROTATED_RECT, rotated, vec![0.0; 4], true))
        .unwrap();
    // A radius whose square underflows to zero: a named zero-area refusal, never
    // a NaN moment.
    b.insert_entity(circle(
        TINY_CIRCLE,
        Point3 {
            x: 40.0,
            y: 0.0,
            z: 0.0,
        },
        1e-300,
    ))
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
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

pub(super) fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

pub(super) fn select(
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

pub(super) fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

pub(super) fn assert_no_write(app: &Application, session: &SessionState, before: Revision) {
    let after = app.workspace.documents[&DocumentId(1)].drawing.revision();
    assert_eq!(after, before, "no revision bump");
    assert!(
        !app.can_undo(&session.document),
        "no undo entry may be recorded"
    );
}

pub(super) fn diagnostic_text(outcome: &CommandOutcome, code: &str) -> String {
    outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("missing diagnostic {code}: {:?}", outcome.diagnostics))
        .message
        .clone()
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
