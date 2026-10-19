//! Screen-space marker sizing (defect #2) and the pointer-cursor overlay input
//! fingerprints. Split out of `overlay/tests.rs` so that file stays under the
//! 1000-line code limit.

use super::*;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

#[test]
fn preview_marker_size_tracks_the_view_scale() {
    let small = PreviewOptions::for_view_scale(Some(2.0));
    let large = PreviewOptions::for_view_scale(Some(8.0));
    assert_eq!(small.draw_order, PREVIEW_DRAW_ORDER);
    assert!(
        (small.marker_size - SNAP_MARKER_HALF_EXTENT_PIXELS * 2.0).abs() < 1e-12,
        "the half-extent is pixels × scale: {}",
        small.marker_size
    );
    assert!(
        (large.marker_size / small.marker_size - 4.0).abs() < 1e-12,
        "the same pixel size at 4× the scale is 4× the world size: {} vs {}",
        large.marker_size,
        small.marker_size
    );
}

#[test]
fn preview_marker_size_is_finite_and_positive_for_a_degenerate_scale() {
    for scale in [
        None,
        Some(0.0),
        Some(-3.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        let options = PreviewOptions::for_view_scale(scale);
        assert!(
            options.marker_size.is_finite() && options.marker_size > 0.0,
            "a degenerate scale {scale:?} must not yield a non-finite/non-positive size: {}",
            options.marker_size
        );
    }
    // No scale (a 3D view) falls back to the documented world default.
    assert_eq!(
        PreviewOptions::for_view_scale(None).marker_size,
        DEFAULT_PREVIEW_MARKER_SIZE
    );
}

#[test]
fn overlay_fingerprint_tracks_the_pointer_cursor_but_not_the_view_scale() {
    let baseline = OverlayInputs::default();
    let with_cursor = OverlayInputs {
        cursor: Some(p(1.0, 2.0)),
        ..OverlayInputs::default()
    };
    assert_ne!(
        overlay_fingerprint(&baseline),
        overlay_fingerprint(&with_cursor)
    );
    assert_ne!(
        overlay_fingerprint(&with_cursor),
        overlay_fingerprint(&OverlayInputs {
            cursor: Some(p(3.0, 4.0)),
            ..OverlayInputs::default()
        }),
        "a moved cursor invalidates the overlay"
    );
    // The view scale is camera-derived (the bounds already track it), so it adds
    // no second camera signal to the fingerprint.
    assert_eq!(
        overlay_fingerprint(&baseline),
        overlay_fingerprint(&OverlayInputs {
            view_scale: Some(0.5),
            ..OverlayInputs::default()
        })
    );
}
