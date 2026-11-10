//! ARRAYPATH (2D path array v1) contracts.
//!
//! The first selected entity is the path and the remaining entities are the
//! items; every item is copied to `count` equal arc-length positions along the
//! path. Split out of `drawing/array.rs` so both suites stay inside the
//! 1000-line code gate.

use super::array::{all_lines, array_app, entity_count, one_line_app, undo_depth};
use super::*;

// ---------------------------------------------------------------------------
// ARRAYPATH (2D path array v1): the first selected entity is the path, the rest
// are items placed at equal arc-length positions.
// ---------------------------------------------------------------------------

fn z_axis() -> Point3 {
    Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    }
}

fn all_circles(app: &Application) -> Vec<(Point3, f64)> {
    let mut circles: Vec<(Point3, f64)> = app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Circle { center, radius, .. } => Some((*center, *radius)),
            _ => None,
        })
        .collect();
    circles.sort_by(|a, b| a.0.x.partial_cmp(&b.0.x).expect("finite centres"));
    circles
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn arraypath_straight_path_places_every_item_at_equal_arc_length_positions() {
    // Path: a straight LINE (0,0)-(9,0). Items: a LINE and a CIRCLE whose combined
    // bounding-box lower-left is the origin, so the placement anchor is exactly the
    // path start.
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(9.0, 0.0),
            },
        ),
        (
            2,
            SemanticGeometry::Line {
                start: point(0.0, 1.0),
                end: point(1.0, 1.0),
            },
        ),
        (
            3,
            SemanticGeometry::Circle {
                center: point(2.0, 1.0),
                normal: z_axis(),
                radius: 1.0,
            },
        ),
    ]);
    session
        .selection
        .replace(vec![ref_of(1), ref_of(2), ref_of(3)]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        CommandPayload::ArrayPath {
            count: 4,
            align: false,
        },
    )
    .unwrap();

    // 2 items x 4 positions = 8 independent copies; the path is not duplicated.
    assert_eq!(outcome.objects.len(), 8);
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");
    assert_eq!(entity_count(&app), 11, "path + 2 items + 8 copies");

    // Every item LINE copy starts at its path position plus its own (0,1) offset.
    let mut starts: Vec<f64> = all_lines(&app)
        .into_iter()
        .filter(|(start, _)| near(start.y, 1.0))
        .map(|(start, _)| start.x)
        .collect();
    starts.sort_by(|a, b| a.partial_cmp(b).expect("finite x"));
    // Five lines share y=1: the retained source item (x=0) plus the four copies at
    // 0/3/6/9 (the copy at the path start overlaps the source, as declared).
    let expected = [0.0, 0.0, 3.0, 6.0, 9.0];
    assert_eq!(starts.len(), expected.len());
    for (actual, expected) in starts.iter().zip(expected) {
        assert!(near(*actual, expected), "{actual} != {expected}");
    }
    // The CIRCLE copies sit at the same four positions plus their own (2,1) offset;
    // the retained source circle at (2,1) overlaps the copy at the path start.
    let circles = all_circles(&app);
    assert_eq!(circles.len(), 5, "source + four CIRCLE copies");
    for ((center, radius), expected) in circles.iter().zip([2.0, 2.0, 5.0, 8.0, 11.0]) {
        assert!(near(center.x, expected) && near(center.y, 1.0));
        assert!(near(*radius, 1.0));
    }

    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "arraypath.created")
        .expect("ARRAYPATH reports its placement summary");
    assert!(
        diagnostic.message.contains("4 equal-distance positions"),
        "{}",
        diagnostic.message
    );
    assert!(
        diagnostic.message.contains("align=false"),
        "{}",
        diagnostic.message
    );

    // UNDO removes every copy and restores the exact prior entity count.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 3, "undo removes every copy");
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(all_circles(&app).len(), 1, "only the source CIRCLE remains");

    // REDO reapplies the whole array.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 11);
    assert_eq!(
        all_lines(&app).len(),
        6,
        "path + source item + 4 item LINE copies"
    );
}

#[test]
fn arraypath_align_rotates_each_copy_by_the_local_tangent_delta() {
    // A quarter ARC path: at u = 1 the tangent is PI while the path start tangent
    // is PI/2, so the second copy is rotated by +PI/2 about its own position.
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Arc {
                center: point(0.0, 0.0),
                normal: z_axis(),
                radius: 10.0,
                start: 0.0,
                sweep: std::f64::consts::FRAC_PI_2,
            },
        ),
        (
            2,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(1.0, 0.0),
            },
        ),
    ]);
    session.selection.replace(vec![ref_of(1), ref_of(2)]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        CommandPayload::ArrayPath {
            count: 2,
            align: true,
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(entity_count(&app), 4, "arc + item + 2 copies");

    // The copy at the path end is anchored at (0,10) and its local direction is the
    // rotated original +X, i.e. +Y (angle PI/2).
    let end_copy = all_lines(&app)
        .into_iter()
        .find(|(start, _)| near(start.x, 0.0) && near(start.y, 10.0))
        .expect("a copy is placed at the quarter-arc end");
    let angle = (end_copy.1.y - end_copy.0.y).atan2(end_copy.1.x - end_copy.0.x);
    assert!(
        near(angle, std::f64::consts::FRAC_PI_2),
        "copy angle {angle} != PI/2"
    );

    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "arraypath.created")
        .expect("ARRAYPATH reports its placement summary");
    assert!(
        diagnostic.message.contains("align=true"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn arraypath_a_single_position_lands_at_the_path_start() {
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(9.0, 0.0),
            },
        ),
        (
            2,
            SemanticGeometry::Line {
                start: point(0.0, 1.0),
                end: point(1.0, 1.0),
            },
        ),
    ]);
    session.selection.replace(vec![ref_of(1), ref_of(2)]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        CommandPayload::ArrayPath {
            count: 1,
            align: false,
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(entity_count(&app), 3);
    // The item's bounding-box lower-left (0,1) is translated to the path start.
    let copy = all_lines(&app)
        .into_iter()
        .find(|(start, _)| near(start.y, 0.0))
        .expect("the single copy sits at the path start");
    assert!(near(copy.0.x, 0.0) && near(copy.0.y, 0.0));
    assert!(near(copy.1.x, 1.0) && near(copy.1.y, 0.0));
}

#[test]
fn arraypath_refusals_write_nothing() {
    let payload = |count, align| CommandPayload::ArrayPath { count, align };

    // No selection.
    let (mut app, mut session) = one_line_app();
    session.selection.clear();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        payload(2, false),
    )
    .expect_err("an empty selection is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);

    // Only the path (no item).
    let (mut app, mut session) = one_line_app();
    session.selection.replace(vec![ref_of(1)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        payload(2, false),
    )
    .expect_err("a path without an item is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(entity_count(&app), 1);

    // An unsupported path kind (OPAQUE) is refused as Unsupported.
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Opaque {
                type_key: "ACAD_PROXY_ENTITY".into(),
                version: 1,
                payload: vec![1, 2, 3],
            },
        ),
        (
            2,
            SemanticGeometry::Line {
                start: point(0.0, 1.0),
                end: point(1.0, 1.0),
            },
        ),
    ]);
    session.selection.replace(vec![ref_of(1), ref_of(2)]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        payload(2, false),
    )
    .expect_err("an unsupported path is refused");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);

    // A zero-length path is refused without a fabricated position.
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Line {
                start: point(5.0, 5.0),
                end: point(5.0, 5.0),
            },
        ),
        (
            2,
            SemanticGeometry::Line {
                start: point(0.0, 1.0),
                end: point(1.0, 1.0),
            },
        ),
    ]);
    session.selection.replace(vec![ref_of(1), ref_of(2)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        payload(2, false),
    )
    .expect_err("a zero-length path is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(entity_count(&app), 2);

    // Count 0.
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(9.0, 0.0),
            },
        ),
        (
            2,
            SemanticGeometry::Line {
                start: point(0.0, 1.0),
                end: point(1.0, 1.0),
            },
        ),
    ]);
    session.selection.replace(vec![ref_of(1), ref_of(2)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        payload(0, false),
    )
    .expect_err("count 0 is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(entity_count(&app), 2);

    // Cap overflow (1 item x (cap + 1) positions).
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        payload(crate::app_drawing::MAX_ARRAY_ENTITIES + 1, false),
    )
    .expect_err("above the array cap is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}
