//! ARRAYPATH periodic-path, per-item anchor, ALIGN rotation, ELLIPSE and cap
//! contracts. Split out of `drawing/arraypath.rs` so both suites stay inside the
//! 1000-line code gate.

use super::array::{all_lines, array_app, entity_count};
use super::arraypath::{all_inserts, arraypath_payload, insert_item_app, near, z_axis};
use super::*;

// ---------------------------------------------------------------------------
// Periodic paths, per-item anchors, ALIGN rotation and the copy-count cap.
// ---------------------------------------------------------------------------

/// A drawing with a CIRCLE path and a small horizontal LINE item at the origin.
fn circle_path_app(radius: f64) -> (Application, SessionState) {
    array_app(vec![
        (
            1,
            SemanticGeometry::Circle {
                center: point(0.0, 0.0),
                normal: z_axis(),
                radius,
            },
        ),
        (
            2,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(1.0, 0.0),
            },
        ),
    ])
}

/// The copy LINE starts (the source item is excluded by its exact (0,0) start).
fn copy_starts(app: &Application) -> Vec<Point3> {
    all_lines(app)
        .into_iter()
        .map(|(start, _)| start)
        .filter(|start| !(near(start.x, 0.0) && near(start.y, 0.0)))
        .collect()
}

#[test]
fn arraypath_a_periodic_path_places_count_distinct_copies_without_a_seam_duplicate() {
    // CIRCLE, count = 4: the old i/(count-1) fractions produced the seam point
    // twice (3 distinct positions). Periodic paths use i/count instead.
    let (mut app, mut session) = circle_path_app(10.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 4, false),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 4, "4 copies on a periodic path");
    let starts = copy_starts(&app);
    assert_eq!(starts.len(), 4);
    let expected = [(10.0, 0.0), (0.0, 10.0), (-10.0, 0.0), (0.0, -10.0)];
    for (x, y) in expected {
        let matches = starts
            .iter()
            .filter(|start| near(start.x, x) && near(start.y, y))
            .count();
        assert_eq!(matches, 1, "position ({x},{y}) must appear exactly once");
    }

    // count = 2 keeps the two halves distinct (the seam is never both).
    let (mut app, mut session) = circle_path_app(10.0);
    execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 2, false),
    )
    .unwrap();
    let starts = copy_starts(&app);
    assert_eq!(starts.len(), 2);
    assert!(
        starts
            .iter()
            .any(|start| near(start.x, 10.0) && near(start.y, 0.0))
            && starts
                .iter()
                .any(|start| near(start.x, -10.0) && near(start.y, 0.0)),
        "two distinct halves: {starts:?}"
    );
}

#[test]
fn arraypath_an_insert_item_uses_the_block_base_point_as_its_anchor() {
    // Path LINE (0,0)-(9,0), one INSERT item at (10,10) whose block base point is
    // (2,3): the world anchor is `transform.apply_point(base) == (10,10)`, so the
    // three copies' world insertion base points are the three path positions.
    let (mut app, mut session) = insert_item_app(
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(9.0, 0.0),
        },
        point(10.0, 10.0),
        point(2.0, 3.0),
    );
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 3, false),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 3);
    // The source INSERT (world base (10,10)) plus the three copies (0, 4.5, 9).
    let insertions = all_inserts(&app);
    assert_eq!(insertions.len(), 4, "source + three INSERT copies");
    let xs: Vec<f64> = insertions.iter().map(|(base, _)| base.x).collect();
    let expected = [0.0, 4.5, 9.0, 10.0];
    assert_eq!(xs.len(), expected.len());
    for (actual, expected) in xs.iter().zip(expected) {
        assert!(near(*actual, expected), "{actual} != {expected}");
    }
    for (base, _) in &insertions[..3] {
        assert!(
            near(base.y, 0.0) && near(base.z, 0.0),
            "copy base point {base:?}"
        );
    }
}

#[test]
fn arraypath_align_rotates_each_copy_about_its_own_translated_anchor() {
    // A quarter ARC path (tangent PI/2 at the start, PI at the end) and two items
    // 1000 units apart: each copy keeps the group translation but rotates about
    // its own anchor, so the far item never swings around the path point.
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
        (
            3,
            SemanticGeometry::Line {
                start: point(1000.0, 500.0),
                end: point(1001.0, 500.0),
            },
        ),
    ]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2), ref_of(3)], 2, true),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 4, "2 items x 2 copies");
    let lines = all_lines(&app);

    // Copy at the path start (10,0): delta is 0 there, so both items keep their
    // orientation and each anchor is translated to the group position.
    let at_start = |start: Point3, end: Point3| {
        lines.iter().any(|(a, b)| {
            near(a.x, start.x) && near(a.y, start.y) && near(b.x, end.x) && near(b.y, end.y)
        })
    };
    assert!(
        at_start(point(10.0, 0.0), point(11.0, 0.0)),
        "near item at the path start (no rotation there): {lines:?}"
    );
    assert!(
        at_start(point(1010.0, 500.0), point(1011.0, 500.0)),
        "far item keeps its own anchor, no swing to the path point: {lines:?}"
    );
    // Copy at the path end (0,10): the local tangent differs by +PI/2, so each
    // copy rotates in place about its own translated anchor.
    assert!(
        at_start(point(0.0, 10.0), point(0.0, 11.0)),
        "near item rotated about its own anchor: {lines:?}"
    );
    assert!(
        at_start(point(1000.0, 510.0), point(1000.0, 511.0)),
        "far item rotated about its own anchor: {lines:?}"
    );

    // No far-item copy was dragged to the path point: the nearest far-item copy
    // start is still over 500 units away from its placement point.
    let far_copies: Vec<Point3> = lines
        .iter()
        .map(|(start, _)| *start)
        .filter(|start| start.x >= 1000.0 && !(near(start.x, 1000.0) && near(start.y, 500.0)))
        .collect();
    assert_eq!(far_copies.len(), 2, "two far-item copies: {far_copies:?}");
    for placement in [point(1010.0, 500.0), point(1000.0, 510.0)] {
        assert!(
            far_copies
                .iter()
                .any(|copy| near(copy.x, placement.x) && near(copy.y, placement.y)),
            "anchor missing at {placement:?}: {far_copies:?}"
        );
    }
}

#[test]
fn arraypath_a_bulged_polyline_mid_point_and_tangent_are_exact() {
    // LWPOLYLINE (0,0)-(20,0) with bulge 1.0: a CCW semicircle (radius 10) below
    // the chord. count = 3 puts the middle copy at half arc length, the bottom of
    // the circle (10,-10), where the tangent is +X (angle 0). The start tangent is
    // -Y (-PI/2), so ALIGN rotates the middle copy by +PI/2.
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Polyline {
                points: vec![point(0.0, 0.0), point(20.0, 0.0)],
                bulges: vec![1.0],
                closed: false,
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
    execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 3, true),
    )
    .unwrap();
    let lines = all_lines(&app);
    assert!(
        lines.iter().any(|(start, end)| {
            near(start.x, 10.0) && near(start.y, -10.0) && near(end.x, 10.0) && near(end.y, -9.0)
        }),
        "middle copy at the semicircle bottom, rotated +PI/2: {lines:?}"
    );
}

#[test]
fn arraypath_align_refuses_a_path_without_an_in_plane_tangent() {
    // A purely vertical LINE path has no XY tangent at either end. ALIGN cannot
    // orient a copy there, so it refuses by name; a translate-only array is still
    // a real result and must keep working.
    let vertical = SemanticGeometry::Line {
        start: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        end: Point3 {
            x: 0.0,
            y: 0.0,
            z: 10.0,
        },
    };
    let item = SemanticGeometry::Line {
        start: point(0.0, 0.0),
        end: point(1.0, 0.0),
    };
    let (mut app, mut session) = array_app(vec![(1, vertical), (2, item)]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 2, true),
    )
    .expect_err("ALIGN on a vertical path cannot invent a tangent");
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.contains("arraypath.no_plane_tangent")),
        "{error:?}"
    );
    assert_eq!(revision(&app), before, "a refusal changes nothing");
    assert_eq!(entity_count(&app), 2);

    // The same path without ALIGN needs no tangent: two translated copies.
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 2, false),
    )
    .expect("a translate-only vertical array is a real result");
    assert_eq!(outcome.objects.len(), 2);
    let zs: Vec<f64> = all_lines(&app)
        .into_iter()
        .map(|(start, _)| start.z)
        .filter(|z| near(*z, 0.0) || near(*z, 10.0))
        .collect();
    assert!(
        zs.iter().filter(|z| near(**z, 0.0)).count() >= 2 && zs.iter().any(|z| near(*z, 10.0)),
        "copies at both path ends: {zs:?}"
    );
}

/// The ellipse speed `|d/dθ (a cosθ, b sinθ)|`.
fn ellipse_speed(angle: f64, major: f64, minor: f64) -> f64 {
    (major * major * angle.sin().powi(2) + minor * minor * angle.cos().powi(2)).sqrt()
}

/// The arc length of an ellipse from angle 0 to `angle` (composite Simpson).
fn ellipse_arc(angle: f64, major: f64, minor: f64) -> f64 {
    const N: usize = 2000;
    let h = angle / N as f64;
    let mut sum = ellipse_speed(0.0, major, minor) + ellipse_speed(angle, major, minor);
    for k in 1..N {
        let weight = if k % 2 == 0 { 2.0 } else { 4.0 };
        sum += weight * ellipse_speed(k as f64 * h, major, minor);
    }
    sum * h / 3.0
}

/// The parameter angle whose ellipse arc length from 0 equals `target`.
fn ellipse_angle_at_arc(target: f64, sweep: f64, major: f64, minor: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, sweep);
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if ellipse_arc(mid, major, minor) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

#[test]
fn arraypath_an_ellipse_path_uses_equal_arc_length_not_the_parameter_midpoint() {
    // ELLIPSE major 10, ratio 0.25, sweep 3PI/4. The middle copy sits at half the
    // arc length, which is NOT the parameter midpoint (3PI/8) on a non-circular
    // ellipse.
    let major = 10.0;
    let minor = 2.5;
    let sweep = 3.0 * std::f64::consts::FRAC_PI_4;
    let (mut app, mut session) = array_app(vec![
        (
            1,
            SemanticGeometry::Ellipse {
                center: point(0.0, 0.0),
                normal: z_axis(),
                major_axis: Point3 {
                    x: major,
                    y: 0.0,
                    z: 0.0,
                },
                ratio: 0.25,
                start: 0.0,
                sweep,
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
    execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(vec![ref_of(1), ref_of(2)], 3, false),
    )
    .unwrap();

    let total = ellipse_arc(sweep, major, minor);
    let angle = ellipse_angle_at_arc(total / 2.0, sweep, major, minor);
    let expected = point(major * angle.cos(), minor * angle.sin());
    let parameter_midpoint = point(major * (sweep / 2.0).cos(), minor * (sweep / 2.0).sin());

    let lines = all_lines(&app);
    let middle = lines
        .iter()
        .map(|(start, _)| *start)
        .filter(|start| !(near(start.x, 0.0) && near(start.y, 0.0)))
        .min_by(|a, b| {
            let da = (a.x - expected.x).hypot(a.y - expected.y);
            let db = (b.x - expected.x).hypot(b.y - expected.y);
            da.partial_cmp(&db).expect("finite distances")
        })
        .expect("three ELLIPSE copies");
    assert!(
        (middle.x - expected.x).hypot(middle.y - expected.y) < 1e-3,
        "middle copy {middle:?} is not the equal-arc point {expected:?}"
    );
    assert!(
        (middle.x - parameter_midpoint.x).hypot(middle.y - parameter_midpoint.y) > 0.5,
        "the equal-arc midpoint must differ from the parameter midpoint: {middle:?} vs \
         {parameter_midpoint:?}"
    );
}

#[test]
fn arraypath_accepts_exactly_the_max_array_entities_cap() {
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
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPath,
        arraypath_payload(
            vec![ref_of(1), ref_of(2)],
            crate::app_drawing::MAX_ARRAY_ENTITIES,
            false,
        ),
    )
    .expect("exactly the cap is accepted");
    assert_eq!(
        outcome.objects.len(),
        crate::app_drawing::MAX_ARRAY_ENTITIES,
        "one item x the cap copies is exactly the cap"
    );
}
