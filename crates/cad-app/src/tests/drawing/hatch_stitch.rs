//! HATCH `LINE`/`ARC` boundary stitching contract tests.
//!
//! Pins the stitching algorithm end to end: nodes cluster at the **absolute**
//! `1e-6` world-unit tolerance, `ARC` edges become exact bulge segments
//! (`bulge = tan(sweep / 4)`, never sampled), one selection may yield several
//! loops, mixed edge orientations stitch, and the named refusals
//! (`hatch.boundary_open`, `hatch.boundary_branch`, `hatch.boundary_degenerate`)
//! change nothing.

use super::*;

use super::hatch::{
    entity_count, filled_area, hatch_app, pattern_lines, run, square, the_hatch, undo_depth,
    user_defined,
};

const FRAC_PI_2: f64 = std::f64::consts::FRAC_PI_2;
const FRAC_PI_4: f64 = std::f64::consts::FRAC_PI_4;
const FRAC_PI_8: f64 = std::f64::consts::FRAC_PI_8;
const PI: f64 = std::f64::consts::PI;
const TAU: f64 = std::f64::consts::TAU;

fn line(id: u128, start: (f64, f64), end: (f64, f64)) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Line {
            start: p3(start.0, start.1, 0.0),
            end: p3(end.0, end.1, 0.0),
        },
    )
}

/// An `ARC` on the `+Z` plane, sweeping counter-clockwise from `start` by `sweep`.
fn arc(
    id: u128,
    center: (f64, f64),
    radius: f64,
    start: f64,
    sweep: f64,
) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Arc {
            center: p3(center.0, center.1, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius,
            start,
            sweep,
        },
    )
}

/// The **exact** area of a closed bulged polyline.
///
/// The chord polygon area plus the signed circular-segment area of every bulge:
/// `bulge = tan(sweep / 4)` gives the included sweep, the chord is
/// `2 r sin(sweep / 2)` and the segment area is `(r^2 / 2)(sweep - sin sweep)`.
/// Signed contributions make the loop's winding irrelevant, and nothing here
/// samples the arcs.
fn exact_area(points: &[[f64; 2]], bulges: &[f64]) -> f64 {
    let count = points.len();
    let mut area = 0.0;
    for index in 0..count {
        let a = points[index];
        let b = points[(index + 1) % count];
        area += 0.5 * (a[0] * b[1] - b[0] * a[1]);
    }
    for index in 0..count {
        let bulge = bulges.get(index).copied().unwrap_or(0.0);
        if bulge == 0.0 {
            continue;
        }
        let sweep = 4.0 * bulge.atan();
        let a = points[index];
        let b = points[(index + 1) % count];
        let chord = (b[0] - a[0]).hypot(b[1] - a[1]);
        let radius = chord / (2.0 * (sweep / 2.0).sin().abs());
        area += 0.5 * radius * radius * (sweep - sweep.sin());
    }
    area.abs()
}

/// Run HATCH, assert every `needle` appears in the refusal, and assert nothing was
/// written (no revision bump, no transaction, no inserted entity).
fn refuses(app: &mut Application, session: &mut SessionState, ids: &[u128], needles: &[&str]) {
    let before = revision(app);
    let depth = undo_depth(app);
    let entities = entity_count(app);
    let error = run(app, session, ids).expect_err("the command must refuse");
    let text = format!("{error:?}");
    for needle in needles {
        assert!(text.contains(needle), "expected {needle:?} in {text}");
    }
    assert_eq!(revision(app), before, "a refusal must not write");
    assert_eq!(
        undo_depth(app),
        depth,
        "a refusal must not open a transaction"
    );
    assert_eq!(
        entities,
        entity_count(app),
        "a refusal must not insert an entity"
    );
}

fn assert_point(actual: [f64; 2], expected: [f64; 2]) {
    assert!(
        (actual[0] - expected[0]).abs() < 1e-12 && (actual[1] - expected[1]).abs() < 1e-12,
        "point {actual:?} vs {expected:?}"
    );
}

// ---------------------------------------------------------------------------
// Closed chains
// ---------------------------------------------------------------------------

#[test]
fn four_lines_close_into_one_loop_and_match_the_polyline_hatch() {
    let edges = vec![
        line(1, (0.5, 0.5), (10.5, 0.5)),
        line(2, (10.5, 0.5), (10.5, 10.5)),
        line(3, (10.5, 10.5), (0.5, 10.5)),
        line(4, (0.5, 10.5), (0.5, 0.5)),
    ];
    let (mut app, mut session) = hatch_app(edges);
    user_defined(&mut session, 2.0, 0.0, false);
    let out = run(&mut app, &mut session, &[1, 2, 3, 4]).expect("a closed LINE chain hatches");
    assert_eq!(out.objects.len(), 1, "one hatch entity");

    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1, "one loop per closed chain");
    let loop_ = &hatch.loops()[0];
    assert_eq!(
        loop_.points,
        vec![[0.5, 0.5], [10.5, 0.5], [10.5, 10.5], [0.5, 10.5]],
        "one vertex per consumed edge, in traversal order"
    );
    assert_eq!(
        loop_.bulges,
        vec![0.0; 4],
        "LINE edges are bulge-0 segments"
    );
    assert!((exact_area(&loop_.points, &loop_.bulges) - 100.0).abs() < 1e-12);

    // The same boundary selected as one closed polyline must produce the identical
    // loop, and therefore the identical analytic pattern geometry.
    let (mut poly_app, mut poly_session) = hatch_app(vec![square(9, 0.5, 10.5, 0.0)]);
    user_defined(&mut poly_session, 2.0, 0.0, false);
    run(&mut poly_app, &mut poly_session, &[9]).expect("the polyline square hatches");
    let poly = the_hatch(&poly_app);
    assert_eq!(poly.loops().len(), 1);
    assert_eq!(loop_.points, poly.loops()[0].points);
    assert_eq!(loop_.bulges, poly.loops()[0].bulges);

    // Spacing 2 / angle 0 over [0.5, 10.5]^2: five horizontal lines of length 10.
    let (count, total) = pattern_lines(hatch);
    assert_eq!(count, 5, "five horizontal pattern lines");
    assert!((total - 50.0).abs() < 1e-9, "total length {total}");
    let (poly_count, poly_total) = pattern_lines(poly);
    assert_eq!(count, poly_count);
    assert!((total - poly_total).abs() < 1e-12);

    // The four LINEs are retained (no associativity, no deletion).
    assert_eq!(entity_count(&app), 5);
}

#[test]
fn a_line_and_arc_capsule_stitches_with_exact_bulges_and_area() {
    // Capsule: y = ±2 from x = 0 to x = 10, closed by two half-circles of r = 2 that
    // bulge outwards (`+pi` sweeps counter-clockwise through (12, 0) and (-2, 0)).
    // Its area is the 10 x 4 rectangle plus the two half discs (4*pi).
    let edges = vec![
        line(1, (0.0, -2.0), (10.0, -2.0)),
        arc(2, (10.0, 0.0), 2.0, 3.0 * FRAC_PI_2, PI),
        line(3, (10.0, 2.0), (0.0, 2.0)),
        arc(4, (0.0, 0.0), 2.0, FRAC_PI_2, PI),
    ];
    let (mut app, mut session) = hatch_app(edges);
    let out = run(&mut app, &mut session, &[1, 2, 3, 4]).expect("the capsule hatches");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1, "one closed loop");
    let loop_ = &hatch.loops()[0];
    assert_eq!(loop_.points.len(), 4, "one vertex per consumed edge");

    // Two chord segments and two half-circle arcs: `tan(pi/4)` exactly, with the
    // same sign as the traversal (both arcs sweep counter-clockwise here).
    assert_eq!(
        loop_.bulges,
        vec![0.0, FRAC_PI_4.tan(), 0.0, FRAC_PI_4.tan()],
        "arcs stay exact bulges (tan(sweep/4)), never sampled chords"
    );
    for (point, expected) in
        loop_
            .points
            .iter()
            .zip([[0.0, -2.0], [10.0, -2.0], [10.0, 2.0], [0.0, 2.0]])
    {
        assert_point(*point, expected);
    }

    // Area from the polyline with bulges, not from a sampled outline.
    let expected = 40.0 + 4.0 * PI;
    let area = exact_area(&loop_.points, &loop_.bulges);
    assert!(
        (area - expected).abs() < 1e-9,
        "capsule area {area} vs {expected}"
    );

    // The derived solid fill agrees with the analytic area.
    let filled = filled_area(hatch);
    assert!(
        (filled - expected).abs() < 0.2,
        "filled area {filled} vs {expected}"
    );

    // The summary diagnostic declares the exact-bulge rule and the tolerance.
    let message = out
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "drawing.hatch")
        .map(|diagnostic| diagnostic.message.clone())
        .expect("a hatch summary diagnostic");
    assert!(
        message.contains("exact bulge segments"),
        "arcs must be declared exact: {message}"
    );
    assert!(
        message.contains("never sampled"),
        "the sampling loss must be refused: {message}"
    );
    assert!(
        message.contains("1e-6 world units"),
        "the stitching tolerance must be named: {message}"
    );
    assert!(
        message.contains("associative hatch NOT IMPLEMENTED"),
        "the existing deferral must stay: {message}"
    );
}

#[test]
fn a_quarter_arc_corner_carries_exactly_tan_pi_over_eight() {
    // A 10 x 8 rectangle whose two right-hand corners are rounded by quarter arcs
    // of r = 2. Unlike a semicircle (whose centre lies on the chord), a quarter arc
    // puts the centre on one definite side, so this pins both the `bulge =
    // tan(sweep / 4)` angle (`tan(pi/8)` here) and the bulge **sign** against the
    // loop's winding: the outward arcs add area (88 + 2*pi), an inverted bulge
    // would subtract it (96 - 2*pi).
    let edges = vec![
        line(1, (0.0, 0.0), (10.0, 0.0)),
        arc(2, (10.0, 2.0), 2.0, 3.0 * FRAC_PI_2, FRAC_PI_2),
        line(3, (12.0, 2.0), (12.0, 6.0)),
        arc(4, (10.0, 6.0), 2.0, 0.0, FRAC_PI_2),
        line(5, (10.0, 8.0), (0.0, 8.0)),
        line(6, (0.0, 8.0), (0.0, 0.0)),
    ];
    let (mut app, mut session) = hatch_app(edges);
    run(&mut app, &mut session, &[1, 2, 3, 4, 5, 6]).expect("the rounded rectangle hatches");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1);
    let loop_ = &hatch.loops()[0];
    assert_eq!(loop_.points.len(), 6, "one vertex per consumed edge");
    assert_eq!(
        loop_.bulges,
        vec![0.0, FRAC_PI_8.tan(), 0.0, FRAC_PI_8.tan(), 0.0, 0.0],
        "a quarter arc is tan(pi/8), the same value the circle loop uses"
    );
    for (point, expected) in loop_.points.iter().zip([
        [0.0, 0.0],
        [10.0, 0.0],
        [12.0, 2.0],
        [12.0, 6.0],
        [10.0, 8.0],
        [0.0, 8.0],
    ]) {
        assert_point(*point, expected);
    }
    let expected = 88.0 + 2.0 * PI;
    let area = exact_area(&loop_.points, &loop_.bulges);
    assert!((area - expected).abs() < 1e-9, "area {area} vs {expected}");
    // The derived fill mesh inscribes the arcs within its own fill tolerance, so it
    // is slightly smaller than the exact area.
    let filled = filled_area(hatch);
    assert!((filled - expected).abs() < 0.2, "filled area {filled}");
}

#[test]
fn reversed_edge_directions_still_stitch() {
    // The rule: the stitcher matches **endpoints**, not stored directions, and an
    // edge may be consumed either way round (an arc consumed backwards flips its
    // sweep sign). Here the two LINEs and both ARCs are stored in the direction
    // opposite to the loop traversal, and the loop must come out identical.
    let forward = vec![
        line(1, (0.0, -2.0), (10.0, -2.0)),
        arc(2, (10.0, 0.0), 2.0, 3.0 * FRAC_PI_2, PI),
        line(3, (10.0, 2.0), (0.0, 2.0)),
        arc(4, (0.0, 0.0), 2.0, FRAC_PI_2, PI),
    ];
    let reversed = vec![
        // The seed line still runs with the loop.
        line(11, (0.0, -2.0), (10.0, -2.0)),
        // (10, 2) -> (10, -2): the same outward half-circle, drawn the other way.
        arc(12, (10.0, 0.0), 2.0, FRAC_PI_2, -PI),
        // (0, 2) -> (10, 2): stored against the loop traversal.
        line(13, (0.0, 2.0), (10.0, 2.0)),
        // (0, -2) -> (0, 2): stored against the loop traversal.
        arc(14, (0.0, 0.0), 2.0, 3.0 * FRAC_PI_2, -PI),
    ];
    let (mut app, mut session) = hatch_app(forward);
    run(&mut app, &mut session, &[1, 2, 3, 4]).expect("the forward capsule hatches");
    let expected = the_hatch(&app).loops()[0].clone();

    let (mut rev_app, mut rev_session) = hatch_app(reversed);
    run(&mut rev_app, &mut rev_session, &[11, 12, 13, 14])
        .expect("the mixed-orientation capsule hatches");
    let rev_hatch = the_hatch(&rev_app);
    assert_eq!(rev_hatch.loops().len(), 1);
    let loop_ = &rev_hatch.loops()[0];
    assert_eq!(
        loop_.bulges, expected.bulges,
        "arc bulges follow the traversal"
    );
    for (actual, wanted) in loop_.points.iter().zip(expected.points.iter()) {
        assert_point(*actual, *wanted);
    }
    let area = exact_area(&loop_.points, &loop_.bulges);
    assert!(
        (area - (40.0 + 4.0 * PI)).abs() < 1e-9,
        "capsule area {area}"
    );
}

#[test]
fn two_disjoint_squares_stitch_into_two_loops_of_one_hatch() {
    let edges = vec![
        line(1, (0.0, 0.0), (10.0, 0.0)),
        line(2, (10.0, 0.0), (10.0, 10.0)),
        line(3, (10.0, 10.0), (0.0, 10.0)),
        line(4, (0.0, 10.0), (0.0, 0.0)),
        line(5, (20.0, 0.0), (30.0, 0.0)),
        line(6, (30.0, 0.0), (30.0, 10.0)),
        line(7, (30.0, 10.0), (20.0, 10.0)),
        line(8, (20.0, 10.0), (20.0, 0.0)),
    ];
    let (mut app, mut session) = hatch_app(edges);
    let out = run(&mut app, &mut session, &[1, 2, 3, 4, 5, 6, 7, 8]).expect("both squares hatch");
    assert_eq!(out.objects.len(), 1, "one entity, two boundary loops");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 2, "one loop per closed chain");
    for loop_ in hatch.loops() {
        assert_eq!(loop_.points.len(), 4);
        assert_eq!(loop_.bulges, vec![0.0; 4]);
        assert!((exact_area(&loop_.points, &loop_.bulges) - 100.0).abs() < 1e-12);
    }
    // Even-odd fill of two disjoint 100-area squares.
    let area = filled_area(hatch);
    assert!((area - 200.0).abs() < 1e-9, "filled area {area}");
    assert_eq!(entity_count(&app), 9, "eight edges plus the hatch");
}

#[test]
fn a_full_turn_arc_is_split_into_four_exact_quarter_edges() {
    // A full-circle ARC has coincident endpoints, which would collapse into a
    // single node; it is emitted as four quarter edges (same sweep sign), exactly
    // like a selected CIRCLE boundary.
    let (mut app, mut session) = hatch_app(vec![arc(1, (0.0, 0.0), 5.0, 0.0, TAU)]);
    run(&mut app, &mut session, &[1]).expect("a full ARC hatches");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1);
    let loop_ = &hatch.loops()[0];
    assert_eq!(loop_.points.len(), 4);
    assert_eq!(loop_.bulges, vec![FRAC_PI_8.tan(); 4]);
    for point in &loop_.points {
        assert!((point[0].hypot(point[1]) - 5.0).abs() < 1e-12, "{point:?}");
    }
    let area = exact_area(&loop_.points, &loop_.bulges);
    assert!((area - PI * 25.0).abs() < 1e-9, "disc area {area}");

    // An arc whose own endpoints are within the stitching tolerance is a loop by
    // the same endpoint rule, so it is split the same way (the hairline gap is
    // closed by the tolerance, not drawn).
    let (mut app, mut session) = hatch_app(vec![arc(2, (0.0, 0.0), 5.0, 0.0, TAU - 1e-8)]);
    run(&mut app, &mut session, &[2]).expect("a near-full ARC hatches");
    let hatch = the_hatch(&app);
    let loop_ = &hatch.loops()[0];
    assert_eq!(loop_.points.len(), 4);
    let area = exact_area(&loop_.points, &loop_.bulges);
    assert!((area - PI * 25.0).abs() < 1e-5, "near-disc area {area}");
}

// ---------------------------------------------------------------------------
// Tolerance and refusals
// ---------------------------------------------------------------------------

#[test]
fn a_gap_below_the_tolerance_closes_and_a_larger_gap_is_refused() {
    // The last corner stops `gap` short of the first vertex. `STITCH_TOLERANCE` is
    // 1e-6 world units, so 5e-7 closes and 2e-6 does not; both sides are pinned.
    let chain = |gap: f64| {
        vec![
            line(1, (0.0, 0.0), (10.0, 0.0)),
            line(2, (10.0, 0.0), (10.0, 10.0)),
            line(3, (10.0, 10.0), (0.0, 10.0)),
            line(4, (0.0, 10.0), (0.0, gap)),
        ]
    };

    let (mut app, mut session) = hatch_app(chain(5e-7));
    run(&mut app, &mut session, &[1, 2, 3, 4]).expect("a sub-tolerance gap closes");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1);
    assert_eq!(hatch.loops()[0].points.len(), 4);
    assert_point(hatch.loops()[0].points[0], [0.0, 0.0]);

    let (mut app, mut session) = hatch_app(chain(2e-6));
    refuses(
        &mut app,
        &mut session,
        &[1, 2, 3, 4],
        &["hatch.boundary_open", "4 unconsumed", "1e-6"],
    );
}

#[test]
fn an_open_chain_is_refused_by_name_without_writing() {
    // Three sides of a square: every edge is real, but the chain never closes.
    let edges = vec![
        line(1, (0.0, 0.0), (10.0, 0.0)),
        line(2, (10.0, 0.0), (10.0, 10.0)),
        line(3, (10.0, 10.0), (0.0, 10.0)),
    ];
    let (mut app, mut session) = hatch_app(edges);
    refuses(
        &mut app,
        &mut session,
        &[1, 2, 3],
        &["hatch.boundary_open", "3 unconsumed", "1e-6"],
    );
    assert_eq!(entity_count(&app), 3, "the edges are untouched");
}

#[test]
fn a_branch_node_is_refused_by_name_and_never_paired() {
    // Three edges meet at the origin: there is no unique pairing, so the command
    // names the node and the count instead of guessing.
    let edges = vec![
        line(1, (0.0, 0.0), (10.0, 0.0)),
        line(2, (0.0, 0.0), (0.0, 10.0)),
        line(3, (0.0, 0.0), (-10.0, 0.0)),
    ];
    let (mut app, mut session) = hatch_app(edges);
    refuses(
        &mut app,
        &mut session,
        &[1, 2, 3],
        &["hatch.boundary_branch", "3 edge ends", "(0, 0, 0)"],
    );
}

#[test]
fn a_degenerate_line_or_arc_is_refused_before_stitching() {
    // Zero-length LINE: it has no direction to chain.
    let (mut app, mut session) = hatch_app(vec![line(1, (3.0, 3.0), (3.0, 3.0))]);
    refuses(
        &mut app,
        &mut session,
        &[1],
        &["hatch.boundary_degenerate", "zero length"],
    );

    // Zero-sweep ARC: the "arc" is a point.
    let (mut app, mut session) = hatch_app(vec![arc(2, (0.0, 0.0), 5.0, 0.0, 0.0)]);
    refuses(
        &mut app,
        &mut session,
        &[2],
        &["hatch.boundary_degenerate", "zero sweep"],
    );
}
