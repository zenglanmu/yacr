//! Display-closure contract tests: HATCH boundary spline knots/weights and polyline bulges.

use super::*;

/// Build a HATCH spline boundary edge; the per-control-point weight rides in `z`.
fn spline_edge(
    degree: i32,
    rational: bool,
    periodic: bool,
    knots: Vec<f64>,
    control: &[(f64, f64, f64)],
) -> BoundaryEdge {
    BoundaryEdge::Spline(acadrust::entities::SplineEdge {
        degree,
        rational,
        periodic,
        knots,
        control_points: control
            .iter()
            .map(|(x, y, w)| acadrust::types::Vector3::new(*x, *y, *w))
            .collect(),
        fit_points: Vec::new(),
        start_tangent: acadrust::types::Vector2::new(0.0, 0.0),
        end_tangent: acadrust::types::Vector2::new(0.0, 0.0),
    })
}

/// Sample one boundary edge through the real `append_boundary_edge` path.
fn sample(edge: &BoundaryEdge) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    append_boundary_edge(edge, &mut out, &TessellationParams::default());
    out
}

fn differs(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    if a.len() != b.len() {
        return true;
    }
    a.iter()
        .zip(b)
        .any(|(p, q)| (p[0] - q[0]).abs() > 1e-9 || (p[1] - q[1]).abs() > 1e-9)
}

fn same_points(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(p, q)| (p[0] - q[0]).abs() <= 1e-9 && (p[1] - q[1]).abs() <= 1e-9)
}

const CONTROL: [(f64, f64, f64); 4] = [
    (0.0, 0.0, 1.0),
    (1.0, 3.0, 1.0),
    (3.0, 3.0, 1.0),
    (4.0, 0.0, 1.0),
];

// (a) Rational weights change the curve, and the source knots are honoured.
#[test]
fn rational_hatch_spline_weights_change_the_curve() {
    let knots = vec![0.0, 0.0, 0.0, 0.3, 1.0, 1.0, 1.0];
    let unit = spline_edge(2, true, false, knots.clone(), &CONTROL);
    let pulled: [(f64, f64, f64); 4] = [
        (0.0, 0.0, 1.0),
        (1.0, 3.0, 4.0),
        (3.0, 3.0, 1.0),
        (4.0, 0.0, 1.0),
    ];
    let weighted = spline_edge(2, true, false, knots, &pulled);
    let a = sample(&unit);
    let b = sample(&weighted);
    assert!(!a.is_empty() && !b.is_empty(), "splines produced no points");
    assert!(differs(&a, &b), "weights were ignored: {a:?} == {b:?}");
}

#[test]
fn hatch_spline_honours_non_uniform_knots() {
    // Same control polygon and unit weights; only the knot vector differs from
    // the clamped-uniform convention `tessellate_bspline` would use.
    let knots = vec![0.0, 0.0, 0.0, 0.3, 1.0, 1.0, 1.0];
    let edge = spline_edge(2, false, false, knots, &CONTROL);
    let honoured = sample(&edge);
    let control: Vec<Point3> = CONTROL
        .iter()
        .map(|(x, y, _)| Point3 {
            x: *x,
            y: *y,
            z: 0.0,
        })
        .collect();
    let uniform: Vec<[f64; 2]> = tessellate_bspline(&control, 2, TessellationParams::default())
        .iter()
        .map(|p| [p.x, p.y])
        .collect();
    assert!(!honoured.is_empty(), "spline produced no points");
    assert!(
        differs(&honoured, &uniform),
        "non-uniform knots were ignored: {honoured:?} == {uniform:?}"
    );
}

// (b) A wrong-length knot vector falls back without panicking.
#[test]
fn hatch_spline_wrong_length_knots_falls_back() {
    for knots in [vec![0.0, 1.0], Vec::new(), vec![0.0; 12]] {
        let edge = spline_edge(2, false, false, knots, &CONTROL);
        let got = sample(&edge);
        let control: Vec<Point3> = CONTROL
            .iter()
            .map(|(x, y, _)| Point3 {
                x: *x,
                y: *y,
                z: 0.0,
            })
            .collect();
        let expected: Vec<[f64; 2]> =
            tessellate_bspline(&control, 2, TessellationParams::default())
                .iter()
                .map(|p| [p.x, p.y])
                .collect();
        assert!(
            same_points(&got, &expected),
            "wrong-length knots did not fall back: {got:?} != {expected:?}"
        );
    }
}

// A periodic spline keeps its knot vector and never panics; a non-monotone
// vector is rejected internally and falls back.
#[test]
fn hatch_periodic_spline_uses_knots_without_panicking() {
    let periodic = spline_edge(
        2,
        true,
        true,
        vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        &CONTROL,
    );
    let pts = sample(&periodic);
    assert!(pts.len() > 2, "periodic spline produced {pts:?}");

    // The source knots must actually change the sampled curve versus the
    // clamped-uniform convention `tessellate_bspline` would use.
    let control: Vec<Point3> = CONTROL
        .iter()
        .map(|(x, y, _)| Point3 {
            x: *x,
            y: *y,
            z: 0.0,
        })
        .collect();
    let uniform: Vec<[f64; 2]> = tessellate_bspline(&control, 2, TessellationParams::default())
        .iter()
        .map(|p| [p.x, p.y])
        .collect();
    assert!(
        differs(&pts, &uniform),
        "the periodic knot vector was ignored: {pts:?} == {uniform:?}"
    );

    let malformed = spline_edge(
        2,
        false,
        true,
        vec![0.0, 0.0, 0.0, 2.0, 1.0, 1.0, 1.0],
        &CONTROL,
    );
    assert!(!sample(&malformed).is_empty());
}

// A fit-point-only spline (no control polygon) falls back to a polyline.
#[test]
fn hatch_fit_point_spline_falls_back_to_polyline() {
    let mut edge = spline_edge(3, false, false, Vec::new(), &[]);
    if let BoundaryEdge::Spline(s) = &mut edge {
        s.fit_points = vec![
            acadrust::types::Vector2::new(0.0, 0.0),
            acadrust::types::Vector2::new(1.0, 1.0),
            acadrust::types::Vector2::new(2.0, 0.0),
        ];
    }
    let pts = sample(&edge);
    assert!(
        same_points(&pts, &[[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]]),
        "fit points were not used: {pts:?}"
    );
}

// (c) A non-zero bulge expands into arc points; a zero bulge stays straight.
#[test]
fn hatch_polyline_bulge_expands_to_arc() {
    let edge = BoundaryEdge::Polyline(acadrust::entities::PolylineEdge {
        vertices: vec![
            acadrust::types::Vector3::new(0.0, 0.0, 1.0),
            acadrust::types::Vector3::new(2.0, 0.0, 0.0),
        ],
        is_closed: false,
    });
    let pts = sample(&edge);
    assert!(pts.len() > 2, "bulge produced no arc points: {pts:?}");
    // A bulge of 1 is a semicircle: every sample lies on the circle centred at
    // the chord midpoint (1, 0) with radius 1.
    for p in &pts {
        let r = ((p[0] - 1.0).powi(2) + p[1].powi(2)).sqrt();
        assert!((r - 1.0).abs() < 1e-6, "off-circle point {p:?}");
    }
    // Positive (counter-clockwise) bulge dips below a left-to-right chord.
    assert!(pts.iter().any(|p| p[1] < -0.5), "arc did not bow: {pts:?}");
}

#[test]
fn hatch_polyline_zero_bulge_stays_straight() {
    let edge = BoundaryEdge::Polyline(acadrust::entities::PolylineEdge {
        vertices: vec![
            acadrust::types::Vector3::new(0.0, 0.0, 0.0),
            acadrust::types::Vector3::new(2.0, 0.0, 0.0),
        ],
        is_closed: false,
    });
    assert!(same_points(&sample(&edge), &[[0.0, 0.0], [2.0, 0.0]]));
}

// (d) A closed polyline's wrap segment carries the last vertex's bulge and the
// loop closes on its first point.
#[test]
fn hatch_closed_polyline_wrap_uses_last_bulge() {
    let edge = BoundaryEdge::Polyline(acadrust::entities::PolylineEdge {
        vertices: vec![
            acadrust::types::Vector3::new(0.0, 0.0, 0.0),
            acadrust::types::Vector3::new(2.0, 0.0, 0.0),
            acadrust::types::Vector3::new(2.0, 2.0, 0.0),
            acadrust::types::Vector3::new(0.0, 2.0, 1.0),
        ],
        is_closed: true,
    });
    let pts = sample(&edge);
    assert!(pts.len() > 5, "wrap bulge produced no arc: {pts:?}");
    assert_eq!(pts[0], [0.0, 0.0], "loop must start at the first vertex");
    assert_eq!(
        pts[pts.len() - 1],
        [0.0, 0.0],
        "loop must close on its first point"
    );
    // The last vertex's bulge bows the wrap segment to the left of the square.
    assert!(
        pts.iter().any(|p| p[0] < -0.5),
        "wrap did not use the last vertex's bulge: {pts:?}"
    );
    // The wrap samples sit on the semicircle centred at (0, 1) with radius 1.
    for p in pts.iter().filter(|p| p[0] < 0.0) {
        let r = (p[0].powi(2) + (p[1] - 1.0).powi(2)).sqrt();
        assert!((r - 1.0).abs() < 1e-6, "off-circle wrap point {p:?}");
    }
}
