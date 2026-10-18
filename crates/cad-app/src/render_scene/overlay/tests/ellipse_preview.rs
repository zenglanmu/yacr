//! ELLIPSE live-preview overlay contract tests (ADR 0015 decision A/D).
//!
//! Split out of `overlay/tests.rs` to keep that file under the code-size limit.
//! The preview reuses the existing `DrawPreview` points + cursor plumbing (no new
//! overlay channel) and draws a bounded closed chord tessellation of the exact
//! ellipse the commit stores; the preview itself is an approximation.

use super::*;

#[test]
fn ellipse_preview_draws_a_closed_bounded_tessellated_curve() {
    // The captured axis-endpoint picks plus the live cursor are previewed as one
    // bounded closed chord tessellation.
    let preview = DrawPreview {
        kind: DrawToolKind::Ellipse,
        points: vec![p(-2.0, 0.0), p(2.0, 0.0)],
        picks: Vec::new(),
        // The third pick: the perpendicular half-axis distance (b = 4), longer
        // than the first-axis semi-axis (a = 2), so the preview reads as the same
        // ellipse the command stores (major = 4, ratio = 0.5).
        cursor: Some(p(0.0, 4.0)),
        candidate: None,
        remaining: 1,
        selection_count: 0,
        ready: false,
    };
    let overlay = draw_preview_overlay(&preview, DocumentId(1), false, &PreviewOptions::default());
    assert_eq!(overlay.drawn(), 1);
    let batch = &overlay.batches[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert!(
        batch.vertices.len() >= 2 && batch.vertices.len() <= 129,
        "bounded tessellation (<= 128 segments), got {}",
        batch.vertices.len()
    );
    // A closed curve: the last vertex tessellates t = 2π, i.e. the first vertex.
    let first = batch.vertices.first().unwrap();
    let last = batch.vertices.last().unwrap();
    for axis in 0..3 {
        assert!(
            (first[axis] - last[axis]).abs() < 1e-6,
            "the tessellation must close back onto its start"
        );
    }

    // A degenerate capture (the third pick on the first-axis line) has no ellipse,
    // so only the rubber-band chain is previewed — never a fabricated ellipse.
    let degenerate = DrawPreview {
        cursor: Some(p(2.0, 0.0)),
        ..preview.clone()
    };
    let overlay = draw_preview_overlay(
        &degenerate,
        DocumentId(1),
        false,
        &PreviewOptions::default(),
    );
    assert_eq!(overlay.batches[0].vertices.len(), 2, "picks + cursor chain");

    // Fewer than two picks cannot define an ellipse either: the chain again.
    let single = DrawPreview {
        points: vec![p(1.0, 1.0)],
        cursor: Some(p(5.0, 4.0)),
        ..preview.clone()
    };
    let overlay = draw_preview_overlay(&single, DocumentId(1), false, &PreviewOptions::default());
    assert_eq!(
        overlay.batches[0].vertices.len(),
        2,
        "one pick + cursor chain, never a fabricated ellipse"
    );
}
