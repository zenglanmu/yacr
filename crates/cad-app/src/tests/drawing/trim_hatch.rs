//! TRIM of a **HATCH** target: the boundary loop through the shared polyline
//! planner, the re-closure along the straight cutting edge, and the named
//! refusals that write nothing.
//!
//! The module under test is `app_drawing::trim_hatch`; these contracts pin the
//! kept window (the same "the segment containing the pick projection is kept"
//! rule every other TRIM target uses), the rebuilt fill, the preserved entity
//! fields, the one-transaction/one-undo shape, and every refusal branch.

use super::hatch::filled_area;
use super::trim_extend_curves::{app_of, geometry_of, line, snapshot, trim, undo_depth};
use super::*;

/// A solid HATCH over the closed loop `points` (millimetre-free drawing units).
fn solid_hatch_geo(points: &[[f64; 2]]) -> SemanticGeometry {
    let hatch = HatchGeometry::new(
        vec![HatchLoop::new(
            points.to_vec(),
            vec![0.0; points.len()],
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

/// The 4x4 square loop `(0,0) (4,0) (4,4) (0,4)`.
fn square_loop() -> &'static [[f64; 2]] {
    &[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]
}

fn hatch_of(app: &Application, id: u128) -> HatchGeometry {
    match geometry_of(app, id) {
        SemanticGeometry::Hatch(hatch) => *hatch,
        other => panic!("expected a HATCH, got {other:?}"),
    }
}

/// The single loop's vertices in world space.
fn loop_world_points(hatch: &HatchGeometry) -> Vec<Point3> {
    hatch.loops()[0]
        .points
        .iter()
        .map(|point| hatch.plane().to_world(*point))
        .collect()
}

fn holds(points: &[Point3], expected: Point3) -> bool {
    points.iter().any(|point| {
        (point.x - expected.x).abs() <= 1e-9
            && (point.y - expected.y).abs() <= 1e-9
            && (point.z - expected.z).abs() <= 1e-9
    })
}

#[test]
fn trim_a_hatch_boundary_loop_keeps_the_picked_window_closed_by_the_cutting_line() {
    // The 4x4 solid hatch is cut by the vertical line x = 2 at (2,0) and (2,4).
    // The pick projects onto the left half, so that window is kept and closed by
    // the straight cutting edge between the two cut points: a 2x4 rectangle of
    // area 8, with the same id/layer/space/fill/plane.
    let mut app = app_of(vec![
        ent(1, solid_hatch_geo(square_loop()), 0),
        line(2, point(2.0, -2.0), point(2.0, 6.0), 1),
    ]);
    assert!((filled_area(&hatch_of(&app, 1)) - 16.0).abs() < 1e-9);
    let type_key_before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .expect("the seed hatch")
        .object
        .type_key
        .clone();

    let outcome = trim(&mut app, 1, &[2], point(1.0, 2.0)).expect("hatch trim succeeds");
    assert_eq!(outcome.diagnostics[0].code, "drawing.trim");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("re-closed along the straight")),
        "the re-closure is reported: {:?}",
        outcome.diagnostics
    );
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.entity_count(), 2, "the hatch was replaced in place");
    let entity = drawing.entity(EntityId(1)).expect("the hatch survives");
    assert_eq!(entity.object.type_key, type_key_before, "type key kept");
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.draw_order, 0);

    let hatch = hatch_of(&app, 1);
    assert_eq!(hatch.loops().len(), 1, "still one boundary loop");
    let points = loop_world_points(&hatch);
    assert_eq!(points.len(), 4, "the kept window is the four left corners");
    for expected in [
        point(2.0, 4.0),
        point(0.0, 4.0),
        point(0.0, 0.0),
        point(2.0, 0.0),
    ] {
        assert!(
            holds(&points, expected),
            "missing {expected:?} in {points:?}"
        );
    }
    assert!(
        !holds(&points, point(4.0, 0.0)) && !holds(&points, point(4.0, 4.0)),
        "the trimmed-away right half must be gone: {points:?}"
    );
    assert!(
        hatch.loops()[0]
            .bulges
            .iter()
            .all(|bulge| bulge.abs() <= 1e-12),
        "the closing chord is straight: {:?}",
        hatch.loops()[0].bulges
    );
    assert!(
        (filled_area(&hatch) - 8.0).abs() < 1e-9,
        "the fill was rebuilt from the kept loop: {}",
        filled_area(&hatch)
    );
    assert_eq!(
        hatch.plane(),
        HatchPlane::default(),
        "the plane is preserved"
    );
    assert_eq!(
        *hatch.fill(),
        HatchFill::Solid,
        "the fill definition is kept"
    );

    // One undo restores the source loop and its fill exactly.
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    let restored = hatch_of(&app, 1);
    let restored_points = loop_world_points(&restored);
    assert_eq!(restored_points.len(), 4);
    for expected in [
        point(4.0, 0.0),
        point(4.0, 4.0),
        point(0.0, 4.0),
        point(0.0, 0.0),
    ] {
        assert!(
            holds(&restored_points, expected),
            "undo must restore {expected:?}: {restored_points:?}"
        );
    }
    assert!((filled_area(&restored) - 16.0).abs() < 1e-9);
}

#[test]
fn trim_deletes_a_hatch_wholly_inside_a_closed_boundary() {
    // No cut at all and the loop lies inside the closed boundary: the shared
    // planner returns `Delete`, so the hatch goes away exactly like any other
    // contained TRIM target, in one transaction / one undo step.
    let closed_outline = SemanticGeometry::Polyline {
        points: vec![
            point(-2.0, -2.0),
            point(10.0, -2.0),
            point(10.0, 10.0),
            point(-2.0, 10.0),
        ],
        bulges: vec![0.0; 4],
        closed: true,
        width: 0.0,
    };
    let mut app = app_of(vec![
        ent(1, solid_hatch_geo(square_loop()), 0),
        ent(2, closed_outline, 1),
    ]);
    let outcome = trim(&mut app, 1, &[2], point(2.0, 2.0)).expect("a contained hatch is deleted");
    assert_eq!(outcome.diagnostics[0].code, "drawing.trim");
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .is_none(),
        "the wholly contained hatch is removed"
    );
    assert_eq!(undo_depth(&app), 1);

    // Undo restores the whole hatch (loop and fill).
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    let restored = hatch_of(&app, 1);
    assert_eq!(restored.loops().len(), 1);
    assert!((filled_area(&restored) - 16.0).abs() < 1e-9);
}

#[test]
fn trim_refuses_a_curved_cutting_edge_that_cannot_close_the_loop() {
    // The circle cuts the square twice — at (2,0) and (2,4) — but the piece of
    // boundary between those two points is not straight, and one circle through
    // two points has two candidate sub-arcs. No straight LINE boundary covers the
    // closing span, so the loop cannot be re-closed exactly: named refusal.
    let mut app = app_of(vec![
        ent(1, solid_hatch_geo(square_loop()), 0),
        ent(
            2,
            SemanticGeometry::Circle {
                center: point(0.5, 2.0),
                normal: p3(0.0, 0.0, 1.0),
                radius: 2.5,
            },
            1,
        ),
    ]);
    let before = snapshot(&app);
    let error = trim(&mut app, 1, &[2], point(3.5, 2.0)).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("trim.hatch_cut_not_closable"),
        "the stable refusal code: {error:?}"
    );
    assert_eq!(snapshot(&app), before, "nothing was written");
    assert_eq!(undo_depth(&app), 0, "no undo step was recorded");
}

#[test]
fn trim_refuses_a_multi_loop_hatch_by_name() {
    // Outer 6x6 loop + inner 2x2 island: cutting the outer loop would change which
    // island stays inside the region — a region boolean, not a loop trim.
    let hatch = HatchGeometry::new(
        vec![
            HatchLoop::new(
                vec![[0.0, 0.0], [6.0, 0.0], [6.0, 6.0], [0.0, 6.0]],
                vec![0.0; 4],
                HatchLoopFlags(HatchLoopFlags::EXTERNAL),
            ),
            HatchLoop::new(
                vec![[2.0, 2.0], [4.0, 2.0], [4.0, 4.0], [2.0, 4.0]],
                vec![0.0; 4],
                HatchLoopFlags::default(),
            ),
        ],
        HatchPlane::default(),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::UserDefined,
    )
    .expect("valid nested hatch");
    let mut app = app_of(vec![
        ent(1, SemanticGeometry::Hatch(Box::new(hatch)), 0),
        line(2, point(-1.0, 3.0), point(7.0, 3.0), 1),
    ]);
    let before = snapshot(&app);
    let error = trim(&mut app, 1, &[2], point(0.5, 3.0)).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("trim.hatch_nested_loops"),
        "the stable refusal code: {error:?}"
    );
    assert_eq!(snapshot(&app), before, "nothing was written");
    assert_eq!(undo_depth(&app), 0, "no undo step was recorded");
}

#[test]
fn trim_refuses_an_insert_target_with_the_shared_definition_reason() {
    // A block reference stores a placement; its members are rows of the shared
    // definition. Cutting one member would rewrite the definition for every
    // insertion (nested ones included) or need a per-instance geometry override
    // this build does not have, so the whole command refuses and writes nothing.
    let (mut app, mut session) = drawing_app();
    let before = snapshot(&app);
    let insert_before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(3))
        .cloned()
        .expect("the seed INSERT");
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TrimEntity,
        CommandPayload::Trim {
            target: ref_of(3),
            boundary: vec![ref_of(1)],
            pick_point: point(5.0, 5.0),
        },
    )
    .unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("trim.target_insert"),
        "the stable refusal code: {error:?}"
    );
    assert!(
        error.to_string().contains("shared block definition"),
        "the refusal states why a member cut is not expressible: {error:?}"
    );
    assert_eq!(snapshot(&app), before, "nothing was written");
    assert_eq!(
        format!(
            "{:?}",
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity(EntityId(3))
                .expect("still there")
        ),
        format!("{insert_before:?}"),
        "the reference itself is untouched"
    );
    assert_eq!(undo_depth(&app), 0, "no undo step was recorded");
}
