//! Per-kind object-snap marker glyphs, pinned to **AutoCAD's** object-snap
//! convention so a future swap of two arms fails here instead of silently
//! changing what a user reads from a marker.
//!
//! Split out of `overlay/tests.rs` to keep that file under the code-size limit.

use std::collections::BTreeSet;

use super::*;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn hint(kind: SnapHintKind) -> SnapHint {
    cad_measure::SnapCandidate {
        kind,
        point: p(0.0, 0.0),
        space: SpaceId::Model,
        source: SelectionRef {
            document: DocumentId(1),
            entity: EntityId(1),
            instance: InstancePath::default(),
            sub_element: None,
        },
        secondary: None,
        precision: Precision::Analytic,
        logical_pixel_distance: 0.0,
    }
}

/// The marker's segments in **world** coordinates, quantised to 1e-4 so the
/// comparison is exact, as an unordered set of (sorted) endpoint pairs.
fn glyph(kind: SnapHintKind) -> BTreeSet<((i64, i64), (i64, i64))> {
    let overlay = snap_hint_overlay(
        &[hint(kind)],
        DocumentId(1),
        &PreviewOptions {
            marker_size: 1.0,
            ..PreviewOptions::default()
        },
    );
    assert_eq!(overlay.drawn(), 1, "{kind:?} must draw exactly one batch");
    let batch = &overlay.batches[0];
    let q = |v: [f32; 3]| {
        (
            ((batch.local_origin.x + f64::from(v[0])) * 1e4).round() as i64,
            ((batch.local_origin.y + f64::from(v[1])) * 1e4).round() as i64,
        )
    };
    batch
        .vertices
        .chunks_exact(2)
        .map(|pair| {
            let a = q(pair[0]);
            let b = q(pair[1]);
            if a <= b {
                (a, b)
            } else {
                (b, a)
            }
        })
        .collect()
}

/// Build the expected segment set from `(x1,y1)-(x2,y2)` f64 endpoints.
fn segs(list: &[(f64, f64, f64, f64)]) -> BTreeSet<((i64, i64), (i64, i64))> {
    list.iter()
        .map(|(x1, y1, x2, y2)| {
            let a = ((x1 * 1e4).round() as i64, (y1 * 1e4).round() as i64);
            let b = ((x2 * 1e4).round() as i64, (y2 * 1e4).round() as i64);
            if a <= b {
                (a, b)
            } else {
                (b, a)
            }
        })
        .collect()
}

#[test]
fn snap_marker_glyphs_match_autocad_per_kind() {
    // Endpoint = square.
    assert_eq!(
        glyph(SnapKind::Endpoint),
        segs(&[
            (-1.0, -1.0, 1.0, -1.0),
            (1.0, -1.0, 1.0, 1.0),
            (1.0, 1.0, -1.0, 1.0),
            (-1.0, 1.0, -1.0, -1.0),
        ]),
        "ENDPOINT must be AutoCAD's square"
    );
    // Midpoint = triangle (AutoCAD MID).
    assert_eq!(
        glyph(SnapKind::Midpoint),
        segs(&[
            (0.0, 1.0, 1.0, -0.8),
            (1.0, -0.8, -1.0, -0.8),
            (-1.0, -0.8, 0.0, 1.0),
        ]),
        "MIDPOINT must be AutoCAD's triangle (not a diamond)"
    );
    // Quadrant = diamond (AutoCAD QUA).
    assert_eq!(
        glyph(SnapKind::Quadrant),
        segs(&[
            (0.0, 1.0, 1.0, 0.0),
            (1.0, 0.0, 0.0, -1.0),
            (0.0, -1.0, -1.0, 0.0),
            (-1.0, 0.0, 0.0, 1.0),
        ]),
        "QUADRANT must be AutoCAD's diamond (not a triangle)"
    );
    // Perpendicular = AutoCAD's ⊥ (stem on a base bar).
    assert_eq!(
        glyph(SnapKind::Perpendicular),
        segs(&[(0.0, -1.0, 0.0, 1.0), (-1.0, -1.0, 1.0, -1.0)]),
        "PERPENDICULAR must be AutoCAD's ⊥ symbol"
    );
    // Local intersection = X.
    assert_eq!(
        glyph(SnapKind::LocalIntersection),
        segs(&[(-1.0, -1.0, 1.0, 1.0), (-1.0, 1.0, 1.0, -1.0)]),
        "LOCAL INTERSECTION must be AutoCAD's X"
    );
    // Centre = circle of radius 1.
    let circle = glyph(SnapKind::Center);
    assert_eq!(circle.len(), SNAP_HINT_CIRCLE_SEGMENTS);
    for ((x1, y1), (x2, y2)) in &circle {
        for (x, y) in [(x1, y1), (x2, y2)] {
            let radius = (*x as f64 / 1e4).hypot(*y as f64 / 1e4);
            assert!(
                (radius - 1.0).abs() < 2e-3,
                "CENTRE marker vertex off the unit circle: r={radius}"
            );
        }
    }
}

/// The snap marker must be the **dominant** shape at the snap point: clearly
/// larger than the pickbox it shares a centre with when the pointer is on the
/// snap target. Both are logical-pixel sizes.
#[test]
fn the_snap_marker_is_clearly_larger_than_the_pickbox() {
    let policy = cad_domain::TolerancePolicy::default();
    let marker = SNAP_MARKER_HALF_EXTENT_PIXELS * 2.0;
    let pickbox = policy.pickbox_logical_pixels * 2.0;
    assert!(
        marker >= pickbox * 2.5,
        "the marker must dominate the pickbox: marker {marker} px across vs pickbox {pickbox} px"
    );
}

/// The paint order that makes the snap type readable: the snap marker is the top
/// shape (the pickbox can never paint over it), the crosshair is still above the
/// rubber band it anchors, and the rest of the table is unchanged.
///
/// Asserted as a runtime sequence so the invariant is checked as a whole and a
/// future reorder fails here.
#[test]
fn the_paint_order_puts_the_snap_marker_above_the_cursor() {
    let table: [(&str, i64); 7] = [
        ("world axes/grid", AXES_GRID_DRAW_ORDER),
        ("POINT markers", POINT_MARKER_DRAW_ORDER),
        ("selection highlight", cad_scene::HIGHLIGHT_DRAW_ORDER),
        ("box-selection band", BOX_SELECT_DRAW_ORDER),
        ("tool preview", PREVIEW_DRAW_ORDER),
        ("pointer cursor crosshair + pickbox", CURSOR_DRAW_ORDER),
        ("object-snap hints", SNAP_HINT_DRAW_ORDER),
    ];
    for pair in table.windows(2) {
        assert!(
            pair[0].1 < pair[1].1,
            "{} ({}) must draw below {} ({})",
            pair[0].0,
            pair[0].1,
            pair[1].0,
            pair[1].1
        );
    }
}
