//! The two-level reference grid contracts: minor/major batches, the default
//! GRIDMAJOR interval of five, per-level bounds and the view-spanning geometry.
//!
//! Split out of `tests.rs` to stay under the 1000-line file gate.

use super::axes_grid::grid_positions;
use super::*;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn segment_pairs(batch: &RenderBatch) -> Vec<([f32; 3], [f32; 3])> {
    batch
        .vertices
        .as_chunks::<2>()
        .0
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

/// World-space X of every vertical segment in `batch` (the grid's vertical
/// lines); horizontal segments are ignored.
fn vertical_line_xs(batch: &RenderBatch) -> Vec<f64> {
    segment_pairs(batch)
        .into_iter()
        .filter_map(|(a, b)| {
            let ax = f64::from(a[0]) + batch.local_origin.x;
            let bx = f64::from(b[0]) + batch.local_origin.x;
            ((ax - bx).abs() < 1e-6).then_some(ax)
        })
        .collect()
}

#[test]
fn grid_overlay_is_two_bounded_batches_spanning_the_supplied_view_bounds() {
    let overlay = grid_overlay((p(-50.0, -50.0), p(50.0, 50.0)), DocumentId(1));
    assert_eq!(overlay.drawn(), 2, "minor and major grid batches");
    let minor = &overlay.batches[0];
    let major = &overlay.batches[1];
    assert!(!minor.vertices.is_empty() && !major.vertices.is_empty());
    assert_eq!(minor.vertices.len() % 2, 0);
    assert_eq!(major.vertices.len() % 2, 0);
    assert_eq!(minor.color, GRID_COLOR, "the minor level is the dimmer one");
    assert_eq!(major.color, GRID_MAJOR_COLOR, "majors are brighter");
    assert!(minor.draw_order < 900_000);
    assert!(major.draw_order < 900_000);
    assert!(
        minor.vertices.len() / 2 <= 2 * GRID_MAX_MINOR_PER_AXIS,
        "the minor level honours its per-axis cap"
    );
    assert!(
        major.vertices.len() / 2 <= 2 * GRID_MAX_MAJOR_PER_AXIS,
        "the major level honours its per-axis cap"
    );
    let total = (minor.vertices.len() + major.vertices.len()) / 2;
    assert!(total <= GRID_MAX_LINES, "both levels together stay bounded");
    // The two levels together span the supplied view rectangle; line batches are
    // clipped to this exact extent by the renderer for every live camera/resize.
    let world_points = overlay.batches.iter().flat_map(|batch| {
        batch.vertices.iter().map(move |v| {
            (
                f64::from(v[0]) + batch.local_origin.x,
                f64::from(v[1]) + batch.local_origin.y,
            )
        })
    });
    let (min_x, max_x, min_y, max_y) = world_points.fold(
        (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ),
        |(min_x, max_x, min_y, max_y), (x, y)| {
            (min_x.min(x), max_x.max(x), min_y.min(y), max_y.max(y))
        },
    );
    assert!((min_x + 50.0).abs() < 1e-4 && (max_x - 50.0).abs() < 1e-4);
    assert!((min_y + 50.0).abs() < 1e-4 && (max_y - 50.0).abs() < 1e-4);
}

#[test]
fn grid_has_one_major_line_every_five_minor_intervals() {
    // Span 100 → minor step 2, major step 10 (the default GRIDMAJOR of 5).
    let overlay = grid_overlay((p(0.0, 0.0), p(100.0, 100.0)), DocumentId(1));
    let minors = vertical_line_xs(&overlay.batches[0]);
    let majors = vertical_line_xs(&overlay.batches[1]);
    assert_eq!(majors.len(), 11, "0, 10, … 100");
    assert_eq!(minors.len(), 40, "the four minors between each major pair");
    for x in &majors {
        assert!(
            (x - (x / 10.0).round() * 10.0).abs() < 1e-6,
            "major lines sit on GRIDMAJOR multiples: {x}"
        );
    }
    for pair in majors.windows(2) {
        let gap = pair[1] - pair[0];
        assert!((gap - 10.0).abs() < 1e-6, "major gap {gap}");
        let between = minors
            .iter()
            .filter(|x| **x > pair[0] + 1e-6 && **x < pair[1] - 1e-6)
            .count();
        assert_eq!(between, 4, "four minors between two majors");
    }
    for x in &minors {
        assert!(
            (x - (x / 10.0).round() * 10.0).abs() > 1e-6,
            "a minor never coincides with a major: {x}"
        );
    }
}

#[test]
fn grid_positions_caps_a_dense_axis_without_shrinking_it() {
    let positions = grid_positions(0.0, 1000.0, 0.001, 10);
    assert_eq!(positions.len(), 10);
    assert_eq!(positions[0], 0.0);
    assert!(
        positions.last().copied().unwrap() > 900.0,
        "the cap samples across the whole range, not just the corner"
    );
}

#[test]
fn grid_overlay_remains_bounded_at_extreme_pan_and_zoom() {
    let overlay = grid_overlay(
        (p(1.0e12, -1.0e12), p(1.0e12 + 1.0e-2, -1.0e12 + 1.0e-2)),
        DocumentId(1),
    );
    let total: usize = overlay
        .batches
        .iter()
        .map(|batch| batch.vertices.len() / 2)
        .sum();
    assert!(total > 0);
    assert!(total <= GRID_MAX_LINES);
}
