//! POLYGON live-preview overlay contract tests (ADR 0015 decision C, P3b).
//!
//! Split out of `overlay/tests.rs` to keep that file under the code-size limit.
//! The preview reuses the existing `DrawPreview` points + cursor plumbing (no new
//! overlay channel): the captured centre plus the live cursor as the radius point
//! is previewed as the exact closed `n`-vertex ring the commit stores, where `n`
//! is the **current** session side count carried on the preview.

use super::*;

/// A POLYGON preview at `sides`, centred at the origin with the cursor as the
/// radius point on `+X`.
fn polygon_preview(sides: Option<u32>) -> DrawPreview {
    DrawPreview {
        kind: DrawToolKind::Polygon,
        points: vec![p(0.0, 0.0)],
        picks: Vec::new(),
        cursor: Some(p(2.0, 0.0)),
        candidate: None,
        remaining: 1,
        selection_count: 0,
        ready: false,
        polygon_sides: sides,
        awaiting_text: false,
    }
}

#[test]
fn polygon_preview_draws_a_closed_outline_with_n_segments() {
    // The current side count drives the segment count: `n` vertices plus the
    // closing repeat of the first vertex, i.e. `n` drawn segments.
    for sides in [3u32, 4, 6, 12] {
        let preview = polygon_preview(Some(sides));
        let overlay =
            draw_preview_overlay(&preview, DocumentId(1), false, &PreviewOptions::default());
        assert_eq!(overlay.drawn(), 1, "{sides} sides must be one batch");
        let batch = &overlay.batches[0];
        assert_eq!(batch.topology, RenderTopology::Lines);
        assert_eq!(
            batch.vertices.len(),
            sides as usize + 1,
            "{sides} sides: {sides} segments, closed back onto the first vertex"
        );
        // A closed outline: the last vertex repeats the first.
        let first = batch.vertices.first().unwrap();
        let last = batch.vertices.last().unwrap();
        for axis in 0..3 {
            assert!(
                (first[axis] - last[axis]).abs() < 1e-6,
                "{sides} sides: the outline must close onto its start"
            );
        }
    }
}

#[test]
fn polygon_preview_falls_back_to_the_chain_for_a_degenerate_or_unknown_capture() {
    // No side count on the preview (a non-POLYGON capture or a missing sysvar):
    // the plain rubber-band chain, never a fabricated ring.
    let unknown = polygon_preview(None);
    let overlay = draw_preview_overlay(&unknown, DocumentId(1), false, &PreviewOptions::default());
    assert_eq!(
        overlay.batches[0].vertices.len(),
        2,
        "centre + cursor chain"
    );

    // A zero-radius capture (the cursor on the centre) has no ring; the chain
    // collapses to a single point, so nothing is drawn — never a fabricated ring.
    let degenerate = DrawPreview {
        cursor: Some(p(0.0, 0.0)),
        ..polygon_preview(Some(6))
    };
    let overlay = draw_preview_overlay(
        &degenerate,
        DocumentId(1),
        false,
        &PreviewOptions::default(),
    );
    assert!(
        overlay.is_empty(),
        "a zero-radius capture draws nothing, never a fabricated ring"
    );
}
