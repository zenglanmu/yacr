//! POLYLINE live-preview overlay contract test.
//!
//! A preview chain is drawn with `Lines` topology, i.e. a `LineList` (two
//! vertices per segment). The chain must therefore be expanded into explicit
//! consecutive pairs: a raw chain would only draw every other link and silently
//! drop the segment between captured points 2 and 3.

use super::*;

/// A draw preview over `points` with the live cursor as the next point.
fn preview(kind: DrawToolKind, points: Vec<Point3>, cursor: Option<Point3>) -> DrawPreview {
    DrawPreview {
        kind,
        points,
        picks: Vec::new(),
        cursor,
        candidate: None,
        remaining: 1,
        selection_count: 0,
        ready: false,
        polygon_sides: None,
        awaiting_text: false,
        text_prefill: None,
    }
}

#[test]
fn polyline_preview_pairs_every_consecutive_segment() {
    // Two captured points plus a third and the live cursor: every consecutive
    // link must appear as an explicit segment pair, including captured 2 -> 3.
    let preview = preview(
        DrawToolKind::Polyline,
        vec![p(0.0, 0.0), p(1.0, 0.0), p(2.0, 1.0)],
        Some(p(3.0, 3.0)),
    );
    let overlay = draw_preview_overlay(&preview, DocumentId(1), false, &PreviewOptions::default());
    assert_eq!(overlay.drawn(), 1, "the chain is one batch");
    let batch = &overlay.batches[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert_eq!(
        batch.vertices,
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
            [2.0, 1.0, 0.0],
            [3.0, 3.0, 0.0],
        ],
        "a 4-point chain is three explicit segments relative to the first point"
    );
}

#[test]
fn rectang_preview_pairs_the_four_outline_edges() {
    // The closed four-corner outline is a five-point chain, i.e. four segments and
    // eight vertices; a raw chain would draw only the top and bottom edges.
    let preview = preview(DrawToolKind::Rectang, vec![p(1.0, 1.0)], Some(p(5.0, 4.0)));
    let overlay = draw_preview_overlay(&preview, DocumentId(1), false, &PreviewOptions::default());
    assert_eq!(overlay.drawn(), 1);
    let batch = &overlay.batches[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert_eq!(batch.vertices.len(), 8, "four edges as explicit pairs");
    assert_eq!(batch.vertices.len() % 2, 0);
}
