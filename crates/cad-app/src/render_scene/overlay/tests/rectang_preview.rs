//! RECTANG live-preview overlay contract test (ADR 0015 decision A, P3a).
//!
//! Split out of `overlay/tests.rs` to keep that file under the code-size limit
//! (and to make room for the POLYGON preview module). RECTANG reuses the existing
//! `DrawPreview` points + cursor plumbing (no new overlay channel).

use super::*;

#[test]
fn rectang_preview_draws_the_closed_outline_between_the_two_corners() {
    // RECTANG reuses the existing `DrawPreview` points + cursor plumbing (no new
    // overlay channel): one captured corner plus the live cursor is previewed as
    // the closed four-corner outline, as a single `Lines` batch.
    let preview = DrawPreview {
        kind: DrawToolKind::Rectang,
        points: vec![p(1.0, 1.0)],
        picks: Vec::new(),
        cursor: Some(p(5.0, 4.0)),
        candidate: None,
        remaining: 1,
        selection_count: 0,
        ready: false,
        polygon_sides: None,
    };
    let overlay = draw_preview_overlay(&preview, DocumentId(1), false, &PreviewOptions::default());
    assert_eq!(overlay.drawn(), 1);
    let batch = &overlay.batches[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert_eq!(
        batch.vertices.len(),
        5,
        "four corners with the last closing back onto the first"
    );
    // A degenerate span (the cursor on the corner's vertical line) has no
    // rectangle, so only the rubber-band chain is previewed.
    let degenerate = DrawPreview {
        cursor: Some(p(1.0, 9.0)),
        ..preview
    };
    let overlay = draw_preview_overlay(
        &degenerate,
        DocumentId(1),
        false,
        &PreviewOptions::default(),
    );
    assert_eq!(
        overlay.batches[0].vertices.len(),
        2,
        "corner + cursor chain, never a fabricated rectangle"
    );
}
