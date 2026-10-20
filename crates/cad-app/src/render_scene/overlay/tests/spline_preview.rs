//! SPLINE live-preview overlay contract test (ADR 0015 decision F, P3d).
//!
//! Split out of `overlay/tests.rs` to keep that file under the code-size limit.
//! The preview reuses the existing `DrawPreview` points + cursor plumbing (no new
//! overlay channel): the captured fit points plus the live cursor are interpolated
//! by the same pure helper the commit uses and drawn as a bounded chord
//! approximation; below the four-point minimum (or when the interpolation refuses)
//! it falls back to the rubber-band chain.

use super::*;

#[test]
fn spline_preview_draws_a_bounded_chord_approximation_of_the_curve() {
    // Four fit points plus the live cursor interpolate to a degree-3 fit spline,
    // sampled as a bounded chord approximation in a single `Lines` batch.
    let preview = DrawPreview {
        kind: DrawToolKind::Spline,
        points: vec![p(0.0, 0.0), p(1.0, 2.0), p(2.5, 2.5), p(4.0, 1.0)],
        picks: Vec::new(),
        cursor: Some(p(5.5, -1.0)),
        candidate: None,
        remaining: 0,
        selection_count: 0,
        ready: true,
        polygon_sides: None,
        awaiting_text: false,
    };
    let overlay = draw_preview_overlay(&preview, DocumentId(1), false, &PreviewOptions::default());
    assert_eq!(overlay.drawn(), 1);
    let batch = &overlay.batches[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert!(
        batch.vertices.len() >= 2 && batch.vertices.len() <= 257,
        "bounded chord approximation (<= 256 segments), got {}",
        batch.vertices.len()
    );
    // The curve starts exactly at the first fit point (clamped interpolation), and
    // the batch is expressed relative to that origin.
    for axis in 0..3 {
        assert!(
            batch.vertices[0][axis].abs() < 1e-6,
            "the curve must start at the first fit point"
        );
    }

    // Fewer than four fit points (two captured + cursor = three) cannot define a
    // degree-3 fit spline: only the rubber-band chain is previewed, never a
    // fabricated curve.
    let few = DrawPreview {
        points: vec![p(0.0, 0.0), p(1.0, 1.0)],
        cursor: Some(p(2.0, 0.0)),
        ..preview.clone()
    };
    let overlay = draw_preview_overlay(&few, DocumentId(1), false, &PreviewOptions::default());
    assert_eq!(
        overlay.batches[0].vertices.len(),
        3,
        "two fit points + cursor chain, never a fabricated curve"
    );

    // With no live cursor the captured fit points alone still interpolate.
    let no_cursor = DrawPreview {
        cursor: None,
        ..preview.clone()
    };
    let overlay =
        draw_preview_overlay(&no_cursor, DocumentId(1), false, &PreviewOptions::default());
    assert!(
        overlay.batches[0].vertices.len() > 2,
        "the captured fit points interpolate to a curve"
    );
}
