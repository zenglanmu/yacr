//! HATCH boundary-arc reading contract tests (ADR 0018 §B.7, U-14).
//!
//! A boundary arc's `counter_clockwise` flag is not only a direction: a
//! clockwise edge stores its angles mirrored (the complementary angle), so the
//! edge runs from `-start` to `-end`. Read as literal angles, the measured
//! AC1018 file's flat 0.9° edges became 359° arcs, and one hatch's boundary
//! reached 3 500 units out of a 27-unit chord — the render collapsed to giant
//! disks and the whole drawing's extents were wrong.

use super::*;
use acadrust::entities::{BoundaryEdge, BoundaryPath, CircularArcEdge, EllipticArcEdge, Hatch};

fn line(path: &mut BoundaryPath, from: (f64, f64), to: (f64, f64)) {
    path.add_edge(BoundaryEdge::Line(acadrust::entities::LineEdge {
        start: acadrust::types::Vector2::new(from.0, from.1),
        end: acadrust::types::Vector2::new(to.0, to.1),
    }));
}

fn ellipse(
    center: (f64, f64),
    major: (f64, f64),
    start: f64,
    end: f64,
    counter_clockwise: bool,
) -> BoundaryEdge {
    BoundaryEdge::EllipticArc(EllipticArcEdge {
        center: acadrust::types::Vector2::new(center.0, center.1),
        major_axis_endpoint: acadrust::types::Vector2::new(major.0, major.1),
        minor_axis_ratio: 1.0,
        start_angle: start,
        end_angle: end,
        counter_clockwise,
    })
}

fn arc(
    center: (f64, f64),
    radius: f64,
    start: f64,
    end: f64,
    counter_clockwise: bool,
) -> BoundaryEdge {
    BoundaryEdge::CircularArc(CircularArcEdge {
        center: acadrust::types::Vector2::new(center.0, center.1),
        radius,
        start_angle: start,
        end_angle: end,
        counter_clockwise,
    })
}

fn sample(edge: &BoundaryEdge) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    append_boundary_edge(edge, &mut out, &TessellationParams::default());
    out
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// The measured AC1018 pair: a 45°-family hatch whose second boundary arc is
/// clockwise. `a1 > a0` with `ccw = false` is the case that used to become a
/// 359° sweep.
fn measured_path() -> BoundaryPath {
    let mut path = BoundaryPath::new();
    line(&mut path, (497.3, 156.7), (551.3, 161.0));
    line(&mut path, (551.3, 161.0), (551.3, 159.0));
    path.add_edge(ellipse(
        (612.861, -1603.878),
        (1763.953, 0.0),
        1.605_703,
        1.621_023,
        true,
    ));
    path.add_edge(ellipse(
        (435.739, 1919.578),
        (-1763.953, 0.0),
        -1.621_023,
        -1.605_703,
        false,
    ));
    path
}

#[test]
fn a_clockwise_flat_ellipse_edge_joins_the_chain_instead_of_circling_the_drawing() {
    let path = measured_path();
    let mut points = Vec::new();
    let report = sample_boundary_path(&path.edges, &mut points, &TessellationParams::default());
    assert!(!report.broken, "{report:?}");

    // The chain's own vertices span ~54 x 4 units. A literal reading of the
    // clockwise edge put a sample at y = 3681 (radius 1764 out of a 27-unit
    // chord), so every junction and every vertex must stay in this box.
    for point in &points {
        assert!(
            (-1.0..=554.0).contains(&point[0]) && (155.0..=162.0).contains(&point[1]),
            "point {point:?} left the boundary envelope; points: {points:?}"
        );
    }
    // Each edge must start where the previous one ended (quantised vertices).
    let mut previous: Option<[f64; 2]> = None;
    for edge in &path.edges {
        let samples = sample(edge);
        if let (Some(previous), Some(&start)) = (previous, samples.first()) {
            assert!(
                distance(previous, start) <= 1.0,
                "junction gap from {previous:?} to {start:?}"
            );
        }
        previous = samples.last().copied();
    }
}

#[test]
fn a_clockwise_arc_sweeps_the_mirrored_parameters() {
    // The convention, stated as an identity: a clockwise edge's stored angles
    // are the mirrored (complementary) ones, so it samples the same points as
    // the counter-clockwise edge over the negated angles.
    // A clockwise edge runs `-a0 → -a1` (decreasing), so it is the *reverse* of
    // the counter-clockwise edge over the negated angles (`-a1 → -a0`).
    let clockwise = arc((5.0, -2.0), 7.5, 0.4, 1.1, false);
    let reversed_ccw = arc((5.0, -2.0), 7.5, -1.1, -0.4, true);
    let a = sample(&clockwise);
    let b = sample(&reversed_ccw);
    assert_eq!(a.len(), b.len());
    for (index, p) in a.iter().enumerate() {
        let q = b[b.len() - 1 - index];
        assert!(
            (p[0] - q[0]).abs() < 1e-12 && (p[1] - q[1]).abs() < 1e-12,
            "{p:?} vs {q:?}"
        );
    }
    // ... and it is *mirrored*, not just reversed: the literal reading (same
    // angles, counter-clockwise) starts somewhere else entirely.
    let literal = sample(&arc((5.0, -2.0), 7.5, 0.4, 1.1, true));
    assert!(
        distance(a[0], literal[0]) > 1.0,
        "the clockwise edge must not sample the literal angles: {:?} vs {:?}",
        a[0],
        literal[0]
    );
}

#[test]
fn a_full_turn_boundary_arc_is_not_collapsed() {
    // `start = 0, end = 2π` normalises to zero but means a whole turn.
    for counter_clockwise in [true, false] {
        let points = sample(&arc(
            (0.0, 0.0),
            3.0,
            0.0,
            std::f64::consts::TAU,
            counter_clockwise,
        ));
        assert!(
            points.len() > 100,
            "a full turn must keep its samples, got {}",
            points.len()
        );
        assert!(distance(points[0], *points.last().unwrap()) < 1e-9);
    }
    // A genuinely identical angle pair is a degenerate zero-length edge.
    let collapsed = sample(&arc((0.0, 0.0), 3.0, 1.0, 1.0, true));
    assert!(collapsed.len() <= 3, "{collapsed:?}");
}

#[test]
fn a_clockwise_pattern_keeps_the_files_evaluated_spacing() {
    // Measured rows of a ZBM22 hatch: hatch angle 90°, scale 8, and rows that
    // already carry both (angle 45° + 90°, offset 1 × 8, dashes 0.2 × 8). The
    // importer un-evaluates them so `families()` reproduces exactly these
    // numbers; composing them twice drew every 64 units at 225°.
    let mut hatch = Hatch::new();
    hatch.pattern_angle = std::f64::consts::FRAC_PI_2;
    hatch.pattern_scale = 8.0;
    hatch
        .pattern
        .add_line(acadrust::entities::HatchPatternLine {
            angle: 135.0_f64.to_radians(),
            base_point: acadrust::types::Vector2::new(0.0, 5.656),
            offset: acadrust::types::Vector2::new(
                -(8.0_f64 / std::f64::consts::SQRT_2),
                -(8.0_f64 / std::f64::consts::SQRT_2),
            ),
            dash_lengths: vec![1.6, -1.6],
        });
    let pattern = import_pattern(&hatch);
    let family = &pattern.families()[0];
    assert!(
        (family.angle - 135.0_f64.to_radians()).abs() < 1e-9,
        "{family:?}"
    );
    assert!((family.base[0]).abs() < 1e-9, "{family:?}");
    assert!((family.base[1] - 5.656).abs() < 1e-9, "{family:?}");
    let component = 8.0_f64 / std::f64::consts::SQRT_2;
    assert!((family.offset[0] + component).abs() < 1e-9, "{family:?}");
    assert!((family.offset[1] + component).abs() < 1e-9, "{family:?}");
    let spacing =
        (family.offset[0] * family.offset[0] + family.offset[1] * family.offset[1]).sqrt();
    assert!((spacing - 8.0).abs() < 1e-9, "{spacing}");
    assert_eq!(family.dashes, vec![1.6, -1.6]);
}

#[test]
fn unmatched_boundary_parameters_are_reported_not_corrected() {
    // Two edges that truly do not meet: the sampled chain keeps them as read and
    // reports the measured gap, so a wrong-looking fill is explained.
    let mut path = BoundaryPath::new();
    line(&mut path, (0.0, 0.0), (10.0, 0.0));
    line(&mut path, (40.0, 0.0), (0.0, 10.0));
    let mut points = Vec::new();
    let report = sample_boundary_path(&path.edges, &mut points, &TessellationParams::default());
    assert!(report.broken, "{report:?}");
    assert!((report.max_gap - 30.0).abs() < 1e-9, "{report:?}");
    // Nothing was dropped or re-read.
    assert_eq!(points.len(), 4, "{points:?}");
}
