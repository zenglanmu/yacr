//! TRIM/EXTEND contract tests: the ELLIPSE targets, the POLYLINE targets and the
//! named target-kind refusals added in the 2026-11 completion batches.
//!
//! Split out of `trim_extend_curves.rs` so both test files stay inside the
//! code-size limit. The shared fixtures/assertions (`app_of`, `trim`, `extend`,
//! `snapshot`, ...) are reused from that module.

use super::trim_extend_curves::{
    angle_near, app_of, extend, geometry_of, line, near, snapshot, trim, undo_depth,
};
use super::*;

const PI: f64 = std::f64::consts::PI;
const TAU: f64 = std::f64::consts::TAU;

// ---------------------------------------------------------------------------
// The 2026-11-11 completion batch: ELLIPSE targets for TRIM/EXTEND, the lifted
// arc half-turn bound, and the named target-kind refusals.
// ---------------------------------------------------------------------------

/// An ELLIPSE target at the origin: major radius 4 along +x, ratio 0.5, spanning
/// `[start, start + sweep]` (a full ellipse when `sweep == TAU`).
fn ellipse_target(id: u128, start: f64, sweep: f64, order: i64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(4.0, 0.0, 0.0),
            ratio: 0.5,
            start,
            sweep,
        },
        order,
    )
}

/// `(center, major_axis, ratio, start, sweep)` of an ELLIPSE.
fn expect_ellipse(app: &Application, id: u128) -> (Point3, Point3, f64, f64, f64) {
    match geometry_of(app, id) {
        SemanticGeometry::Ellipse {
            center,
            major_axis,
            ratio,
            start,
            sweep,
            ..
        } => (center, major_axis, ratio, start, sweep),
        other => panic!("expected an ELLIPSE, got {other:?}"),
    }
}

/// A solid HATCH over a 4x4 square (used only as an unsupported *target*).
fn solid_hatch() -> SemanticGeometry {
    let hatch = HatchGeometry::new(
        vec![HatchLoop::new(
            vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
            vec![0.0; 4],
            HatchLoopFlags::default(),
        )],
        HatchPlane::default(),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::UserDefined,
    )
    .expect("valid hatch");
    SemanticGeometry::Hatch(Box::new(hatch))
}

#[test]
fn trim_a_full_ellipse_target_keeps_the_picked_elliptical_arc() {
    // The ellipse is cut by the vertical line x = 2 at the eccentric angles
    // ±60°; the pick sits on the +x end (eccentric angle 0), so the kept piece is
    // the elliptical arc from -60° to +60° — the same stored frame, ratio and
    // normal, only `start`/`sweep` change (never a refit, never a polyline).
    let mut app = app_of(vec![
        ellipse_target(1, 0.0, TAU, 0),
        line(2, point(2.0, -5.0), point(2.0, 5.0), 1),
    ]);
    let outcome = trim(&mut app, 1, &[2], point(4.0, 0.0)).unwrap();
    assert_eq!(outcome.diagnostics[0].code, "drawing.trim");
    let (center, major_axis, ratio, start, sweep) = expect_ellipse(&app, 1);
    assert_eq!(center, point(0.0, 0.0), "the centre is unchanged");
    assert_eq!(major_axis, p3(4.0, 0.0, 0.0), "the major axis is unchanged");
    assert!(near(ratio, 0.5), "the ratio is unchanged: {ratio}");
    assert!(
        angle_near(start, -PI / 3.0),
        "the kept piece starts at -60°: {start}"
    );
    assert!(
        near(sweep.abs(), TAU / 3.0),
        "the kept piece spans 120°: {sweep}"
    );
    assert_eq!(undo_depth(&app), 1, "one undo step");
}

#[test]
fn trim_an_elliptical_arc_target_at_one_cut_keeps_the_pick_side() {
    // Upper half of the same ellipse (eccentric 0..180°), cut once by x = 0 at 90°.
    // The pick at the +x end keeps 0..90° (an elliptical quarter arc).
    let mut app = app_of(vec![
        ellipse_target(1, 0.0, PI, 0),
        line(2, point(0.0, -5.0), point(0.0, 5.0), 1),
    ]);
    trim(&mut app, 1, &[2], point(4.0, 0.0)).unwrap();
    let (_, _, _, start, sweep) = expect_ellipse(&app, 1);
    assert!(angle_near(start, 0.0), "start {start}");
    assert!(near(sweep, PI / 2.0), "sweep {sweep}");
}

#[test]
fn extend_an_elliptical_arc_target_to_the_nearest_line_ahead() {
    // Elliptical quarter arc 0..90° (from (4,0) to (0,2)); the end is picked and
    // the line y = -1 crosses the ellipse at 210° and 330°, so the nearest crossing
    // ahead (210°) grows the sweep from 90° to 210° on the same stored frame.
    let mut app = app_of(vec![
        ellipse_target(1, 0.0, PI / 2.0, 0),
        line(2, point(-6.0, -1.0), point(6.0, -1.0), 1),
    ]);
    extend(&mut app, 1, &[2], point(0.0, 2.0)).unwrap();
    let (center, major_axis, ratio, start, sweep) = expect_ellipse(&app, 1);
    assert_eq!(center, point(0.0, 0.0));
    assert_eq!(major_axis, p3(4.0, 0.0, 0.0));
    assert!(near(ratio, 0.5));
    assert!(angle_near(start, 0.0), "start {start}");
    assert!(
        near(sweep, 7.0 * PI / 6.0),
        "the end grows from 90° to 210°: {sweep}"
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn extend_refuses_a_full_ellipse_target_because_it_has_no_ends() {
    let mut app = app_of(vec![
        ellipse_target(1, 0.0, TAU, 0),
        line(2, point(-6.0, -1.0), point(6.0, -1.0), 1),
    ]);
    let before = snapshot(&app);
    let error = extend(&mut app, 1, &[2], point(4.0, 0.0)).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("closed and has no endpoint"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before, "a refusal changes nothing");
    assert_eq!(undo_depth(&app), 0);
}

/// An LWPOLYLINE entity (the name avoids the `line` fixture).
fn polyline(id: u128, points: Vec<Point3>, bulges: Vec<f64>, closed: bool, order: i64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        },
        order,
    )
}

/// `(points, bulges, closed)` of a POLYLINE.
fn expect_polyline(app: &Application, id: u128) -> (Vec<Point3>, Vec<f64>, bool) {
    match geometry_of(app, id) {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => (points, bulges, closed),
        other => panic!("expected a POLYLINE, got {other:?}"),
    }
}

fn revision_of(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn trim_an_open_bulged_polyline_keeps_the_picked_arc_window_exactly() {
    // The target is the upper semicircle (0,0) -> (4,0) (bulge -1, so the apex is
    // (2,2)) followed by the straight segment (4,0) -> (8,0). One horizontal LINE
    // y = 1 crosses the arc twice, at the eccentric parameter t = 1/6 and 5/6
    // (x = 2 -/+ sqrt(3)). TRIM keeps the window containing the pick — here the
    // apex cap — so the survivor is that exact sub-arc of the same circle, in
    // place, with its sub-bulge tan(-PI/6).
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(polyline(
        1,
        vec![point(0.0, 0.0), point(4.0, 0.0), point(8.0, 0.0)],
        vec![-1.0, 0.0],
        false,
        0,
    ))
    .unwrap();
    b.insert_entity(line(2, point(-5.0, 1.0), point(9.0, 1.0), 1))
        .unwrap();
    let attributes = non_default_attributes();
    b.set_entity_render_attributes(EntityId(1), attributes.clone())
        .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = app_of(Vec::new());
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    let before = revision_of(&app);

    let outcome = trim(&mut app, 1, &[2], point(2.0, 2.0)).unwrap();
    assert_eq!(outcome.diagnostics[0].code, "drawing.trim");
    assert_eq!(revision_of(&app).0, before.0 + 1, "one transaction");
    assert_eq!(outcome.objects, vec![ObjectId(1)], "one in-place survivor");

    let root = 3.0f64.sqrt();
    let (points, bulges, closed) = expect_polyline(&app, 1);
    assert!(!closed, "the surviving piece is open");
    assert_eq!(points.len(), 2, "{points:?}");
    assert!(
        near(points[0].x, 2.0 - root) && near(points[0].y, 1.0),
        "the kept window starts at the left cut: {:?}",
        points[0]
    );
    assert!(
        near(points[1].x, 2.0 + root) && near(points[1].y, 1.0),
        "the kept window ends at the right cut: {:?}",
        points[1]
    );
    assert_eq!(bulges.len(), 1);
    assert!(
        near(bulges[0], (-PI / 6.0).tan()),
        "the kept sub-arc is exact: {bulges:?}"
    );

    // Id, layer, space and display attributes survive the in-place replacement,
    // and the change is exactly one geometry update (no new entity).
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap();
    assert_eq!(
        (entity.layer, entity.space.clone()),
        (LayerId(0), SpaceId::Model)
    );
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_render_attributes(EntityId(1)),
        attributes
    );
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        2
    );
    let changes = outcome.changes.as_ref().expect("one committed transaction");
    assert_eq!(changes.after.0, before.0 + 1);
    assert_eq!(
        changes.changes,
        vec![cad_db::ObjectChange::Update(
            ObjectId(1),
            cad_db::ChangeMask::GEOMETRY
        )],
        "one geometry update: {:?}",
        changes.changes
    );
    assert_eq!(undo_depth(&app), 1, "one undo entry");

    // One undo restores the whole source polyline.
    execute(
        &mut app,
        &mut SessionState::new(DocumentId(1), AppMode::Work),
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let (points, bulges, closed) = expect_polyline(&app, 1);
    assert!(!closed);
    assert_eq!(
        points,
        vec![point(0.0, 0.0), point(4.0, 0.0), point(8.0, 0.0)]
    );
    assert_eq!(bulges, vec![-1.0, 0.0]);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn trim_an_open_bulged_polyline_keeps_the_window_the_pick_projects_into() {
    // The same target and the same two cuts as the test above; only the pick moves.
    // The projection therefore decides which window survives: a pick on the
    // straight segment keeps the right window (arc end + straight), a pick on the
    // left arc keeps the left window (start + arc), each as the exact sub-geometry.
    let root = 3.0f64.sqrt();
    let sub_bulge = (-PI / 24.0).tan();
    let source = || {
        app_of(vec![
            polyline(
                1,
                vec![point(0.0, 0.0), point(4.0, 0.0), point(8.0, 0.0)],
                vec![-1.0, 0.0],
                false,
                0,
            ),
            line(2, point(-5.0, 1.0), point(9.0, 1.0), 1),
        ])
    };

    // Pick on the straight segment (u = 1.5): the window (5/6, 2) survives.
    let mut app = source();
    trim(&mut app, 1, &[2], point(6.0, 0.0)).unwrap();
    let (points, bulges, closed) = expect_polyline(&app, 1);
    assert!(!closed);
    assert_eq!(points.len(), 3, "{points:?}");
    assert!(
        near(points[0].x, 2.0 + root) && near(points[0].y, 1.0),
        "{:?}",
        points[0]
    );
    assert!(
        near(points[1].x, 4.0) && near(points[1].y, 0.0),
        "the source vertex is preserved: {:?}",
        points[1]
    );
    assert!(
        near(points[2].x, 8.0) && near(points[2].y, 0.0),
        "{:?}",
        points[2]
    );
    assert_eq!(bulges.len(), 2);
    assert!(near(bulges[0], sub_bulge), "{bulges:?}");
    assert!(near(bulges[1], 0.0), "the straight segment stays straight");

    // Pick on the left arc (u = 1/12): the window (0, 1/6) survives, starting at
    // the source's own first vertex.
    let mut app = source();
    let left = point(
        2.0 + 2.0 * (11.0 * PI / 12.0).cos(),
        2.0 * (11.0 * PI / 12.0).sin(),
    );
    trim(&mut app, 1, &[2], left).unwrap();
    let (points, bulges, closed) = expect_polyline(&app, 1);
    assert!(!closed);
    assert_eq!(points.len(), 2, "{points:?}");
    assert!(
        near(points[0].x, 0.0) && near(points[0].y, 0.0),
        "{:?}",
        points[0]
    );
    assert!(
        near(points[1].x, 2.0 - root) && near(points[1].y, 1.0),
        "{:?}",
        points[1]
    );
    assert_eq!(bulges.len(), 1);
    assert!(near(bulges[0], sub_bulge), "{bulges:?}");
}

#[test]
fn trim_a_closed_bulged_polyline_cut_twice_keeps_the_picked_window() {
    // A closed 10x10 square whose right edge bulges out (bulge 0.5). Two separate
    // vertical boundaries cut the bottom edge (u = 0.5) and the top edge
    // (u = 2.5). The pick is the bulge apex (on segment 1, u = 1.5), so the picked
    // window survives as one open polyline that starts at (5,0) and runs forward
    // through the preserved bulge to (5,10).
    let mut app = app_of(vec![
        polyline(
            1,
            vec![
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(10.0, 10.0),
                point(0.0, 10.0),
            ],
            vec![0.0, 0.5, 0.0, 0.0],
            true,
            0,
        ),
        line(2, point(5.0, -1.0), point(5.0, 1.0), 1),
        line(3, point(5.0, 9.0), point(5.0, 11.0), 2),
    ]);
    let outcome = trim(&mut app, 1, &[2, 3], point(12.5, 5.0)).unwrap();
    assert_eq!(outcome.objects, vec![ObjectId(1)], "one survivor, same id");
    let (points, bulges, closed) = expect_polyline(&app, 1);
    assert!(!closed, "the twice-cut ring becomes one open polyline");
    assert_eq!(
        points,
        vec![
            point(5.0, 0.0),
            point(10.0, 0.0),
            point(10.0, 10.0),
            point(5.0, 10.0)
        ],
        "the original vertex order/direction is preserved"
    );
    assert_eq!(bulges.len(), 3);
    assert!(near(bulges[0], 0.0), "{bulges:?}");
    assert!(
        near(bulges[1], 0.5),
        "the bulge survives exactly: {bulges:?}"
    );
    assert!(near(bulges[2], 0.0), "{bulges:?}");
    let changes = outcome.changes.as_ref().expect("one committed transaction");
    assert_eq!(
        changes.changes,
        vec![cad_db::ObjectChange::Update(
            ObjectId(1),
            cad_db::ChangeMask::GEOMETRY
        )],
        "one geometry update: {:?}",
        changes.changes
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn trim_deletes_a_polyline_wholly_inside_a_closed_boundary() {
    // No cut and the polyline's midpoint is inside the closed outline: the whole
    // target is removed, exactly like a contained LINE.
    let mut app = app_of(vec![
        polyline(
            1,
            vec![point(2.0, 2.0), point(4.0, 2.0), point(4.0, 4.0)],
            vec![0.0, 0.0],
            false,
            0,
        ),
        polyline(
            2,
            vec![
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(10.0, 10.0),
                point(0.0, 10.0),
            ],
            vec![0.0; 4],
            true,
            1,
        ),
    ]);
    let outcome = trim(&mut app, 1, &[2], point(3.0, 2.0)).unwrap();
    assert_eq!(
        outcome.changes.as_ref().unwrap().changes,
        vec![cad_db::ObjectChange::Delete(ObjectId(1))],
        "one delete in one transaction"
    );
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .is_none(),
        "the contained polyline is removed"
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn trim_refuses_a_polyline_pick_outside_its_own_ends() {
    // A click beyond the right end projects to a parameter above the polyline's
    // own span (1.0 for one segment): refused by name, never clamped onto the end.
    let mut app = app_of(vec![
        polyline(
            1,
            vec![point(0.0, 0.0), point(10.0, 0.0)],
            vec![0.0],
            false,
            0,
        ),
        line(2, point(5.0, -1.0), point(5.0, 1.0), 1),
    ]);
    let before = snapshot(&app);
    let error = trim(&mut app, 1, &[2], point(30.0, 5.0)).unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(
        error.to_string().contains("trim.polyline_pick_outside"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before, "a refusal changes nothing");
    assert_eq!(undo_depth(&app), 0, "a refusal opens no transaction");
}

#[test]
fn trim_refuses_a_polyline_with_no_intersection() {
    let mut app = app_of(vec![
        polyline(
            1,
            vec![point(0.0, 0.0), point(10.0, 0.0)],
            vec![0.0],
            false,
            0,
        ),
        line(2, point(20.0, -5.0), point(20.0, 5.0), 1),
    ]);
    let before = snapshot(&app);
    let error = trim(&mut app, 1, &[2], point(2.0, 0.0)).unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(
        error.to_string().contains("trim.polyline_no_intersection"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn trim_refuses_degenerate_polyline_geometry_by_name() {
    // Two coincident vertices: no measurable length to cut.
    let mut app = app_of(vec![
        polyline(
            1,
            vec![point(1.0, 1.0), point(1.0, 1.0)],
            vec![0.0],
            false,
            0,
        ),
        line(2, point(0.0, -1.0), point(0.0, 3.0), 1),
    ]);
    let before = snapshot(&app);
    let error = trim(&mut app, 1, &[2], point(1.0, 1.0)).unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(
        error.to_string().contains("trim.polyline_degenerate"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn trim_refuses_a_nonplanar_bulged_polyline_by_name() {
    // A bulge is a circular arc in one plane; two vertices at different z have no
    // single in-plane centre, so the target is refused instead of guessing a plane.
    let mut app = app_of(vec![
        polyline(
            1,
            vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 3.0)],
            vec![1.0],
            false,
            0,
        ),
        line(2, point(2.0, -5.0), point(2.0, 5.0), 1),
    ]);
    let before = snapshot(&app);
    let error = trim(&mut app, 1, &[2], point(2.0, 0.0)).unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(
        error.to_string().contains("trim.polyline_nonplanar_bulge"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn trim_refuses_a_closed_polyline_with_a_single_cut() {
    // One cut cannot separate a piece from a ring (the same rule a CIRCLE keeps).
    let mut app = app_of(vec![
        polyline(
            1,
            vec![
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(10.0, 10.0),
                point(0.0, 10.0),
            ],
            vec![0.0; 4],
            true,
            0,
        ),
        line(2, point(5.0, -1.0), point(5.0, 5.0), 1),
    ]);
    let before = snapshot(&app);
    let error = trim(&mut app, 1, &[2], point(2.5, 0.0)).unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(error.to_string().contains("at least two"), "{error:?}");
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn extend_refuses_a_polyline_target_by_name() {
    let mut app = app_of(vec![
        ent(
            1,
            SemanticGeometry::Polyline {
                points: vec![point(0.0, 0.0), point(5.0, 0.0)],
                bulges: vec![0.0],
                closed: false,
            },
            0,
        ),
        line(2, point(9.0, -2.0), point(9.0, 2.0), 1),
    ]);
    let before = snapshot(&app);
    let error = extend(&mut app, 1, &[2], point(5.0, 0.0)).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("extend.target_polyline"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn trim_and_extend_refuse_a_hatch_target_by_name() {
    // A HATCH's display is derived from its loops and fill: there is no source
    // curve to cut or extend, so both commands refuse by name and write nothing.
    let mut app = app_of(vec![
        ent(1, solid_hatch(), 0),
        line(2, point(5.0, -2.0), point(5.0, 2.0), 1),
    ]);
    let before = snapshot(&app);
    for error in [
        trim(&mut app, 1, &[2], point(2.0, 1.0)).unwrap_err(),
        extend(&mut app, 1, &[2], point(4.0, 1.0)).unwrap_err(),
    ] {
        assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
        assert!(
            error.to_string().contains(".target_hatch"),
            "the named target refusal: {error:?}"
        );
    }
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn trim_refuses_an_opaque_target_by_name() {
    let mut app = app_of(vec![
        ent(
            1,
            SemanticGeometry::Opaque {
                type_key: "ACAD_PROXY_ENTITY".into(),
                version: 1,
                payload: vec![1, 2, 3],
            },
            0,
        ),
        line(2, point(5.0, -2.0), point(5.0, 2.0), 1),
    ]);
    let before = snapshot(&app);
    let error = trim(&mut app, 1, &[2], point(2.0, 1.0)).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("trim.target_opaque"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before);
}
