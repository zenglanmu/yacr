//! Screen-space marker sizing (defect #2) and the overlay-fingerprint
//! exclusions. Split out of `overlay/tests.rs` so that file stays under the
//! 1000-line code limit.

use super::*;

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
fn overlay_fingerprint_ignores_the_view_scale() {
    // `view_scale` stays an overlay input for the screen-relative snap markers,
    // but it is camera-derived (the bounds already track it), so it adds no second
    // camera signal to the fingerprint. The pointer cursor is no longer an overlay
    // input at all (ADR 0028: renderer transient).
    let baseline = OverlayInputs::default();
    assert_eq!(
        overlay_fingerprint(&baseline),
        overlay_fingerprint(&OverlayInputs {
            view_scale: Some(0.5),
            ..OverlayInputs::default()
        })
    );
}
