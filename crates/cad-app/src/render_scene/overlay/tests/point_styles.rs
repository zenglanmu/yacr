//! Contract tests for the AutoCAD point display style (`$PDMODE` / `$PDSIZE`)
//! as the overlay consumes it.
//!
//! Split out of `overlay/point_markers.rs` so both files stay under the
//! 1000-line code limit. The assertions are geometric (vertex counts and offsets,
//! never pixels): the overlay is `Lines`-only, so a glyph is fully described by
//! its segment list.

use super::*;
use crate::layers::LayerOverrideSet;
use crate::render_scene::CadSceneController;
use crate::viewer_config::OverlayVisibility;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::{DatabaseId, EntityId, LayerId, ObjectId, Revision, SemanticGeometry, SpaceId};
use cad_representation::SpaceSelection;
use std::sync::Arc;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// A `$PDMODE`/`$PDSIZE` pair. The DWG defaults are `0` (dot) and `0.0`
/// (viewport-relative).
fn style(mode: i16, size: f64) -> PointDisplayStyle {
    PointDisplayStyle { mode, size }
}

/// A 100 x 100 viewport: with `$PDSIZE = 0` the marker half-extent is
/// `0.025 * 100 = 2.5`, so every expected offset below is that quarter of the
/// viewport spanned by the arms.
fn viewport() -> Option<(Point3, Point3)> {
    Some((p(0.0, 0.0), p(100.0, 100.0)))
}

/// One marker for the point `(10, 20)` in [`viewport`].
fn overlay_at(style: PointDisplayStyle) -> VisualOverlay {
    point_marker_overlay(&[p(10.0, 20.0)], viewport(), style, DocumentId(1))
}

/// Marker vertices in world coordinates, in emission order.
fn vertices(batch: &RenderBatch) -> Vec<(f64, f64)> {
    batch
        .vertices
        .iter()
        .map(|vertex| {
            (
                batch.local_origin.x + f64::from(vertex[0]),
                batch.local_origin.y + f64::from(vertex[1]),
            )
        })
        .collect()
}

/// The marker's vertices as offsets from the marked point `(10, 20)`, in
/// emission order.
fn offsets(overlay: &VisualOverlay) -> Vec<(f64, f64)> {
    offsets_at(overlay, 10.0, 20.0)
}

/// The vertices of `batch` as offsets from `(x, y)`, in emission order.
fn offsets_of_batch(batch: &RenderBatch, x: f64, y: f64) -> Vec<(f64, f64)> {
    vertices(batch)
        .into_iter()
        .map(|(vx, vy)| (vx - x, vy - y))
        .collect()
}

fn assert_offsets(actual: &[(f64, f64)], expected: &[(f64, f64)]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "segment list length: {actual:?} vs {expected:?}"
    );
    for ((ax, ay), (ex, ey)) in actual.iter().zip(expected) {
        assert!(
            (ax - ex).abs() < 1e-5 && (ay - ey).abs() < 1e-5,
            "vertex ({ax}, {ay}) is not ({ex}, {ey}); full list {actual:?}"
        );
    }
}

fn codes(overlay: &VisualOverlay) -> Vec<&str> {
    overlay
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn assert_partial(overlay: &VisualOverlay) {
    assert!(
        matches!(overlay.completeness, Completeness::Partial(_)),
        "an unsupported/approximated aspect must not look complete: {:?}",
        overlay.completeness
    );
}

/// The fixed chord count of a `$PDMODE` `32` circle surround.
const RING_SEGMENTS: usize = POINT_SURROUND_CIRCLE_SEGMENTS;

/// One surround square's four sides, in emission order, for a `±half` square:
/// bottom-left → bottom-right → top-right → top-left, back to bottom-left.
fn square_offsets(half: f64) -> [(f64, f64); 8] {
    let (left, right) = (-half, half);
    let (bottom, top) = (-half, half);
    [
        (left, bottom),
        (right, bottom),
        (right, bottom),
        (right, top),
        (right, top),
        (left, top),
        (left, top),
        (left, bottom),
    ]
}

/// A closed [`RING_SEGMENTS`]-chord ring of `radius`, as the surround emits it:
/// every vertex on the circumscribed radius, every consecutive pair a full chord,
/// and the last vertex bit-identical to the first (a closed ring, not an arc).
fn assert_closed_ring(ring: &[(f64, f64)], radius: f64) {
    assert_eq!(
        ring.len(),
        RING_SEGMENTS * 2,
        "one closed ring of {RING_SEGMENTS} chords: {ring:?}"
    );
    let step = std::f64::consts::TAU / RING_SEGMENTS as f64;
    let chord = 2.0 * radius * (step * 0.5).sin();
    for (index, pair) in ring.chunks(2).enumerate() {
        let (sx, sy) = pair[0];
        let (ex, ey) = pair[1];
        assert!(
            (sx.hypot(sy) - radius).abs() < 1e-5,
            "chord {index} start ({sx}, {sy}) is not on the radius {radius}"
        );
        assert!(
            (ex.hypot(ey) - radius).abs() < 1e-5,
            "chord {index} end ({ex}, {ey}) is not on the radius {radius}"
        );
        assert!(
            ((ex - sx).hypot(ey - sy) - chord).abs() < 1e-5,
            "chord {index} is not {chord} long: ({sx}, {sy}) -> ({ex}, {ey})"
        );
    }
    assert_eq!(
        ring[ring.len() - 1],
        ring[0],
        "the ring must close on its first vertex exactly"
    );
}

#[test]
fn each_implemented_base_mode_draws_its_documented_glyph() {
    // Mode 2: `+` (two axis-aligned segments).
    let plus = overlay_at(style(2, 0.0));
    assert_offsets(
        &offsets(&plus),
        &[(-2.5, 0.0), (2.5, 0.0), (0.0, -2.5), (0.0, 2.5)],
    );
    assert!(plus.diagnostics.is_empty(), "mode 2 is fully implemented");
    assert_eq!(plus.completeness, Completeness::Complete);

    // Mode 3: `x` (two diagonal segments) — the same vertex count as the plus,
    // so the assertion has to look at the geometry, not the count.
    let cross = overlay_at(style(3, 0.0));
    assert_offsets(
        &offsets(&cross),
        &[(-2.5, -2.5), (2.5, 2.5), (-2.5, 2.5), (2.5, -2.5)],
    );
    assert!(cross.diagnostics.is_empty());
    assert_eq!(cross.completeness, Completeness::Complete);

    // Mode 4: a single vertical tick (one segment, two vertices).
    let tick = overlay_at(style(4, 0.0));
    assert_offsets(&offsets(&tick), &[(0.0, -2.5), (0.0, 2.5)]);
    assert_eq!(tick.batches[0].topology, RenderTopology::Lines);
    assert!(tick.diagnostics.is_empty());
    assert_eq!(tick.completeness, Completeness::Complete);

    // Mode 1: nothing at all — an explicit empty marker, not an error and not a
    // fabricated glyph.
    let none = overlay_at(style(1, 0.0));
    assert!(none.batches.is_empty(), "mode 1 draws no marker");
    assert!(none.diagnostics.is_empty(), "mode 1 needs no diagnostic");
    assert_eq!(none.completeness, Completeness::Complete);
}

/// `$PDMODE = 0` is a dot. A `Lines`-only topology cannot rasterise a dot, so the
/// overlay draws the smallest plus glyph it can express and says so exactly once.
#[test]
fn mode_zero_is_an_honest_plus_approximation_not_a_dot() {
    let dot = overlay_at(style(0, 0.0));
    assert_offsets(
        &offsets(&dot),
        &[(-2.5, 0.0), (2.5, 0.0), (0.0, -2.5), (0.0, 2.5)],
    );
    assert_eq!(
        codes(&dot),
        vec!["overlay.point-mode-dot"],
        "exactly one diagnostic, never one per point"
    );
    let message = &dot.diagnostics[0].message;
    assert!(message.contains("not a dot"), "{message}");
    assert!(message.contains("approximation"), "{message}");
    assert_partial(&dot);

    // The approximation is reported once per overlay even for many points.
    let many: Vec<Point3> = (0..50).map(|i| p(i as f64, 0.0)).collect();
    let overlay = point_marker_overlay(&many, viewport(), style(0, 0.0), DocumentId(1));
    assert_eq!(overlay.drawn(), 1);
    assert_eq!(codes(&overlay), vec!["overlay.point-mode-dot"]);
    assert_eq!(
        overlay.batches[0].vertices.len(),
        200,
        "four vertices per approximated dot marker"
    );
}

/// `$PDSIZE > 0` is an absolute drawing-unit size: the marker spans exactly that
/// many drawing units and therefore no longer tracks the viewport.
#[test]
fn a_positive_pdsize_is_an_absolute_drawing_unit_size() {
    let small_view =
        point_marker_overlay(&[p(0.0, 0.0)], viewport(), style(2, 10.0), DocumentId(1));
    let tall_view = point_marker_overlay(
        &[p(0.0, 0.0)],
        Some((p(0.0, 0.0), p(100.0, 400.0))),
        style(2, 10.0),
        DocumentId(1),
    );
    // Half-extent = size / 2 = 5 both times: an absolute size has no viewport
    // domain, so a taller viewport must NOT change it.
    for overlay in [&small_view, &tall_view] {
        assert_offsets(
            &offsets_at(overlay, 0.0, 0.0),
            &[(-5.0, 0.0), (5.0, 0.0), (0.0, -5.0), (0.0, 5.0)],
        );
        assert!(overlay.diagnostics.is_empty());
        assert_eq!(overlay.completeness, Completeness::Complete);
    }

    // A degenerate viewport height does not erase an absolute size: it needs no
    // viewport-height domain.
    let flat = point_marker_overlay(
        &[p(0.0, 0.0)],
        Some((p(0.0, 0.0), p(100.0, 0.0))),
        style(2, 10.0),
        DocumentId(1),
    );
    assert_eq!(
        flat.drawn(),
        1,
        "an absolute PDSIZE is definable without a viewport height"
    );
}

/// Offsets from an arbitrary marked point.
fn offsets_at(overlay: &VisualOverlay, x: f64, y: f64) -> Vec<(f64, f64)> {
    assert_eq!(overlay.drawn(), 1, "one batch for one marker");
    offsets_of_batch(&overlay.batches[0], x, y)
}

/// `$PDSIZE = 0` keeps the historical viewport-relative half-extent (2.5% of the
/// viewport height) and a camera-only change rescales it.
#[test]
fn a_zero_pdsize_keeps_the_viewport_relative_fraction() {
    let short = point_marker_overlay(&[p(0.0, 0.0)], viewport(), style(2, 0.0), DocumentId(1));
    let tall = point_marker_overlay(
        &[p(0.0, 0.0)],
        Some((p(0.0, 0.0), p(100.0, 400.0))),
        style(2, 0.0),
        DocumentId(1),
    );
    assert_offsets(
        &offsets_at(&short, 0.0, 0.0),
        &[(-2.5, 0.0), (2.5, 0.0), (0.0, -2.5), (0.0, 2.5)],
    );
    assert_offsets(
        &offsets_at(&tall, 0.0, 0.0),
        &[(-10.0, 0.0), (10.0, 0.0), (0.0, -10.0), (0.0, 10.0)],
    );
    assert_eq!(
        POINT_MARKER_HALF_EXTENT_FRACTION, 0.025,
        "5% of the viewport height as a full marker: AutoCAD's PDSIZE = 0 meaning"
    );
}

/// `$PDSIZE < 0`: the magnitude is a percentage of the viewport height.
#[test]
fn a_negative_pdsize_is_a_percentage_of_the_viewport_height() {
    let short = point_marker_overlay(&[p(0.0, 0.0)], viewport(), style(2, -10.0), DocumentId(1));
    let tall = point_marker_overlay(
        &[p(0.0, 0.0)],
        Some((p(0.0, 0.0), p(100.0, 400.0))),
        style(2, -10.0),
        DocumentId(1),
    );
    // 10% of a 100-high viewport = a 10-unit half-extent, i.e. 20 units across.
    assert_offsets(
        &offsets_at(&short, 0.0, 0.0),
        &[(-10.0, 0.0), (10.0, 0.0), (0.0, -10.0), (0.0, 10.0)],
    );
    // 10% of a 400-high viewport = 40.
    assert_offsets(
        &offsets_at(&tall, 0.0, 0.0),
        &[(-40.0, 0.0), (40.0, 0.0), (0.0, -40.0), (0.0, 40.0)],
    );
    assert!(short.diagnostics.is_empty());
    assert_eq!(short.completeness, Completeness::Complete);
}

/// The single `$PDSIZE` sanitisation rule: non-finite or absurd magnitudes fall
/// back to the `PDSIZE = 0` behaviour, and the fallback is reported once.
#[test]
fn an_unusable_pdsize_falls_back_to_the_viewport_relative_size_once() {
    for bad in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        POINT_DISPLAY_SIZE_LIMIT * 10.0,
        -POINT_DISPLAY_SIZE_LIMIT * 10.0,
    ] {
        assert_eq!(
            sanitize_point_display_size(bad),
            0.0,
            "an unusable {bad} is the PDSIZE 0 behaviour, never a marker the GPU cannot draw"
        );
        let overlay = overlay_at(style(2, bad));
        assert_offsets(
            &offsets(&overlay),
            &[(-2.5, 0.0), (2.5, 0.0), (0.0, -2.5), (0.0, 2.5)],
        );
        assert_eq!(
            codes(&overlay),
            vec!["overlay.point-size-sanitized"],
            "the substitution is reported, never silent ({bad})"
        );
        assert_partial(&overlay);
    }
    // Usable values are kept verbatim, including the sign and the limit itself.
    for good in [0.0, -0.0, 2.5, -50.0, POINT_DISPLAY_SIZE_LIMIT] {
        assert_eq!(sanitize_point_display_size(good).to_bits(), good.to_bits());
    }
}

/// `$PDMODE` bits `32`/`64` add a circle/square around the base glyph, drawn as
/// explicit segments in the same `Lines` batch: `35` is a cross inside a circle.
#[test]
fn mode_35_is_a_cross_inside_one_closed_sixteen_segment_ring() {
    let circle_cross = overlay_at(style(35, 0.0));
    let all = offsets(&circle_cross);
    assert_eq!(
        all.len(),
        4 + RING_SEGMENTS * 2,
        "the cross's two segments plus one closed ring"
    );
    assert_offsets(
        &all[..4],
        &[(-2.5, -2.5), (2.5, 2.5), (-2.5, 2.5), (2.5, -2.5)],
    );
    // Radius = the marker half extent, i.e. the ring circumscribes the cross.
    assert_closed_ring(&all[4..], 2.5);
    assert!(
        circle_cross.diagnostics.is_empty(),
        "cross + circle is fully implemented: {:?}",
        circle_cross.diagnostics
    );
    assert_eq!(circle_cross.completeness, Completeness::Complete);
    // Still one batch: the ring shares colour, order and topology with the glyph.
    assert_eq!(circle_cross.drawn(), 1);
    assert_eq!(circle_cross.batches[0].topology, RenderTopology::Lines);
    assert_eq!(circle_cross.batches[0].color, POINT_MARKER_COLOR);
    assert_eq!(circle_cross.batches[0].draw_order, POINT_MARKER_DRAW_ORDER);
    assert!(!circle_cross.batches[0].color_unresolved);
}

/// `66` (`64 | 2`) is a plus inside a square: exactly four extra segments at
/// `±half`, i.e. the same square the grip marker draws.
#[test]
fn mode_66_is_a_plus_inside_a_four_segment_square_at_half_extent() {
    let square_plus = overlay_at(style(66, 0.0));
    let all = offsets(&square_plus);
    assert_eq!(
        all.len(),
        4 + POINT_SURROUND_SQUARE_SEGMENTS * 2,
        "the plus's two segments plus the square's four sides"
    );
    assert_offsets(
        &all[..4],
        &[(-2.5, 0.0), (2.5, 0.0), (0.0, -2.5), (0.0, 2.5)],
    );
    assert_offsets(&all[4..], &square_offsets(2.5));
    assert!(square_plus.diagnostics.is_empty());
    assert_eq!(square_plus.completeness, Completeness::Complete);
    assert_eq!(square_plus.drawn(), 1);
}

/// `96 | 4` (tick + circle + square) draws both surrounds and invents no base
/// glyph beyond the tick the base bits ask for.
#[test]
fn mode_100_is_a_tick_with_both_surrounds_and_no_invented_base_glyph() {
    let both = overlay_at(style(96 | 4, 0.0));
    let all = offsets(&both);
    assert_eq!(
        all.len(),
        2 + POINT_SURROUND_SQUARE_SEGMENTS * 2 + RING_SEGMENTS * 2,
        "tick + square + ring, nothing else"
    );
    assert_offsets(&all[..2], &[(0.0, -2.5), (0.0, 2.5)]);
    assert_offsets(
        &all[2..2 + POINT_SURROUND_SQUARE_SEGMENTS * 2],
        &square_offsets(2.5),
    );
    assert_closed_ring(&all[2 + POINT_SURROUND_SQUARE_SEGMENTS * 2..], 2.5);
    assert!(
        both.diagnostics.is_empty(),
        "tick + circle + square is fully implemented: {:?}",
        both.diagnostics
    );
    assert_eq!(both.completeness, Completeness::Complete);
    assert_eq!(both.drawn(), 1);
}

/// The surrounding shapes leave the base-bit decoding exactly as it was: bare
/// `32` decodes as base `0` (the dot approximation, hence its diagnostic) and
/// bare `33` as base `1` (no base glyph, ring only). AutoCAD's own rendering of a
/// bare `32` is **not verified**, so no per-shape exception is invented; there is
/// one decoding rule.
#[test]
fn bare_32_and_33_keep_the_base_bit_decoding_and_only_add_the_ring() {
    assert_eq!(32 & 0x1F, 0, "the surround bit leaves the base bits alone");
    assert_eq!(33 & 0x1F, 1);

    // 32 = circle + base 0, i.e. the honest plus approximation inside the ring.
    let dot_ring = overlay_at(style(32, 0.0));
    let all = offsets(&dot_ring);
    assert_eq!(all.len(), 4 + RING_SEGMENTS * 2);
    assert_offsets(
        &all[..4],
        &[(-2.5, 0.0), (2.5, 0.0), (0.0, -2.5), (0.0, 2.5)],
    );
    assert_closed_ring(&all[4..], 2.5);
    assert_eq!(
        codes(&dot_ring),
        vec!["overlay.point-mode-dot"],
        "base 0 stays an honest approximation and gets no shape diagnostic"
    );
    assert_partial(&dot_ring);

    // 33 = circle + base 1: no base glyph at all, the ring only.
    let ring_only = overlay_at(style(33, 0.0));
    let all = offsets(&ring_only);
    assert_eq!(
        all.len(),
        RING_SEGMENTS * 2,
        "base 1 draws no glyph; the ring is the whole marker"
    );
    assert_closed_ring(&all, 2.5);
    assert!(
        ring_only.diagnostics.is_empty(),
        "base 1 + circle is fully implemented: {:?}",
        ring_only.diagnostics
    );
    assert_eq!(ring_only.completeness, Completeness::Complete);
}

/// An unimplemented base mode (5..=31) still reports
/// `overlay.point-mode-unsupported`, and a requested surround is drawn without
/// inventing the missing base glyph.
#[test]
fn an_unsupported_base_mode_with_a_surround_draws_the_ring_and_no_base_glyph() {
    for base in 5..=31 {
        let overlay = overlay_at(style(32 | base, 0.0));
        let all = offsets(&overlay);
        assert_eq!(
            all.len(),
            RING_SEGMENTS * 2,
            "base {base}: the requested ring only, no invented glyph"
        );
        assert_closed_ring(&all, 2.5);
        assert_eq!(
            codes(&overlay),
            vec!["overlay.point-mode-unsupported"],
            "base {base} must be refused by name"
        );
        assert!(
            overlay.diagnostics[0]
                .message
                .contains(&format!("base mode {base}")),
            "{}",
            overlay.diagnostics[0].message
        );
        assert_partial(&overlay);
    }
}

/// The 4000 cap counts **markers**, not the vertices the surrounds add, and the
/// worst case stays inside the documented structural vertex bound.
#[test]
fn the_cap_counts_markers_not_surround_vertices() {
    // `96 | 4`: tick (1 segment) + square (4) + ring (16) = 21 segments = 42
    // vertices per marker. Fixed, never zoom-derived.
    const VERTICES_PER_TICK_SURROUND_MARKER: usize = 42;
    let worst = style(96 | 4, 0.0);
    let at_cap: Vec<Point3> = (0..POINT_MARKER_LIMIT as u64)
        .map(|i| p(i as f64, 0.0))
        .collect();
    let overlay = point_marker_overlay(&at_cap, viewport(), worst, DocumentId(1));
    assert_eq!(overlay.drawn(), 1, "still one bounded batch");
    assert_eq!(
        overlay.batches[0].vertices.len(),
        POINT_MARKER_LIMIT * VERTICES_PER_TICK_SURROUND_MARKER
    );
    assert!(
        overlay.diagnostics.is_empty(),
        "no diagnostic at exactly the cap"
    );
    assert_eq!(overlay.completeness, Completeness::Complete);

    let over_cap: Vec<Point3> = (0..(POINT_MARKER_LIMIT as u64 + 1))
        .map(|i| p(i as f64, 0.0))
        .collect();
    let overlay = point_marker_overlay(&over_cap, viewport(), worst, DocumentId(1));
    assert_eq!(overlay.drawn(), 1);
    assert_eq!(
        overlay.batches[0].vertices.len(),
        POINT_MARKER_LIMIT * VERTICES_PER_TICK_SURROUND_MARKER,
        "the first {POINT_MARKER_LIMIT} markers are drawn, even though one more \
         marker's vertices would fit in any vertex-based cap"
    );
    assert_eq!(
        codes(&overlay),
        vec!["overlay.point-marker-limit"],
        "exactly one diagnostic, never one per point"
    );
    assert!(overlay.diagnostics[0].message.contains("4001"));
    assert!(overlay.diagnostics[0].message.contains("4000"));
    assert_partial(&overlay);

    // The widest marker this path can emit (cross + square + ring = 22 segments):
    // the exact capped vertex count is the documented structural bound.
    let widest = style(96 | 3, 0.0);
    let overlay = point_marker_overlay(&over_cap, viewport(), widest, DocumentId(1));
    assert_eq!(
        overlay.batches[0].vertices.len(),
        POINT_MARKER_LIMIT * POINT_MARKER_MAX_SEGMENTS_PER_MARKER * 2
    );
    assert_eq!(
        overlay.batches[0].vertices.len(),
        POINT_MARKER_MAX_VERTICES,
        "the cap bound is exact for the widest marker, not an estimate"
    );
}

/// Anything that is not expressible at all is refused by name: whatever part is
/// expressible is drawn, and no shape is substituted for the rest.
#[test]
fn unexpressible_modes_are_refused_by_name_instead_of_substituted() {
    // A base mode this project does not implement (5..=31): no marker at all.
    let unimplemented_base = overlay_at(style(6, 0.0));
    assert!(
        unimplemented_base.batches.is_empty(),
        "no glyph is invented for base mode 6"
    );
    assert_eq!(
        codes(&unimplemented_base),
        vec!["overlay.point-mode-unsupported"]
    );
    assert!(unimplemented_base.diagnostics[0].message.contains('6'));
    assert_partial(&unimplemented_base);

    // Unknown bits above 32/64: the expressible base glyph still draws.
    let unknown_bit = overlay_at(style(4 | 128, 0.0));
    assert_offsets(&offsets(&unknown_bit), &[(0.0, -2.5), (0.0, 2.5)]);
    assert_eq!(codes(&unknown_bit), vec!["overlay.point-mode-unsupported"]);
    assert!(unknown_bit.diagnostics[0].message.contains("128"));

    // An unknown bit neither hides nor invents the implemented geometry: the
    // valid bits (tick 4, circle 32, square 64) still draw, the 128 bit is named.
    let unknown_with_surrounds = overlay_at(style(96 | 4 | 128, 0.0));
    let all = offsets(&unknown_with_surrounds);
    assert_eq!(
        all.len(),
        2 + POINT_SURROUND_SQUARE_SEGMENTS * 2 + RING_SEGMENTS * 2
    );
    assert_eq!(
        codes(&unknown_with_surrounds),
        vec!["overlay.point-mode-unsupported"]
    );
    assert!(unknown_with_surrounds.diagnostics[0]
        .message
        .contains("128"));
    assert_partial(&unknown_with_surrounds);

    // A negative mode is not a point display mode at all.
    let negative = overlay_at(style(-3, 0.0));
    assert!(negative.batches.is_empty());
    assert_eq!(codes(&negative), vec!["overlay.point-mode-unsupported"]);
    assert_partial(&negative);
}

/// The 3D restriction is deliberate and independent of the style: even a
/// `$PDSIZE > 0` that would need no viewport height draws nothing in a 3D view.
#[test]
fn a_3d_view_stays_markerless_even_with_an_absolute_pdsize() {
    for style in [style(2, 10.0), style(0, 0.0), style(3, -20.0)] {
        let overlay = point_marker_overlay(&[p(1.0, 2.0)], None, style, DocumentId(1));
        assert!(
            overlay.is_empty(),
            "3D point markers are NOT IMPLEMENTED whatever the style ({style:?})"
        );
        assert!(
            overlay.diagnostics.is_empty(),
            "nothing is dropped in 3D because nothing was going to be drawn"
        );
        assert_eq!(overlay.completeness, Completeness::Complete);
    }
}

fn point_entity(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbPoint".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Point(p(10.0, 20.0)),
        draw_order: id as i64,
    }
}

fn drawing_with_point(mode: i16, size: f64) -> Arc<cad_db::DrawingDatabase> {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.insert_entity(point_entity(1)).unwrap();
    builder.set_point_display_mode(mode);
    builder.set_point_display_size(size);
    Arc::new(builder.finish().unwrap())
}

/// The style travels from the database through the scene assembly into the
/// marker geometry, so an imported `$PDMODE`/`$PDSIZE` reaches the screen.
#[test]
fn the_scene_assembly_draws_the_drawings_point_style() {
    let db = drawing_with_point(3, -10.0);
    let layers = LayerOverrideSet::new();
    let mut overlays = OverlayInputs {
        visibility: OverlayVisibility {
            axes: false,
            grid: false,
            ..OverlayVisibility::default()
        },
        viewport_bounds: viewport(),
        ..OverlayInputs::default()
    };
    let mut controller = CadSceneController::default();
    controller
        .prepare_shared_with_overlays(
            Some(db.clone()),
            DocumentId(1),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays,
        )
        .unwrap();
    let scene = controller.ready.as_ref().unwrap();
    let batches: Vec<&RenderBatch> = scene
        .highlight
        .added
        .iter()
        .filter(|batch| batch.draw_order == POINT_MARKER_DRAW_ORDER)
        .collect();
    assert_eq!(batches.len(), 1);
    // Mode 3 (diagonal cross) at 10% of a 100-high viewport: half-extent 10.
    assert_offsets(
        &offsets_of_batch(batches[0], 10.0, 20.0),
        &[(-10.0, -10.0), (10.0, 10.0), (-10.0, 10.0), (10.0, -10.0)],
    );
    assert!(
        scene.highlight_diagnostics.is_empty(),
        "mode 3 at a 10% size is fully implemented: {:?}",
        scene.highlight_diagnostics
    );

    // A host that feeds its own style is overridden by the drawing's style: the
    // drawing is the single authority, exactly like the marker positions.
    overlays.point_display_style = style(4, 0.0);
    controller
        .prepare_shared_with_overlays(
            Some(db),
            DocumentId(1),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays,
        )
        .unwrap();
    let scene = controller.ready.as_ref().unwrap();
    let batch = scene
        .highlight
        .added
        .iter()
        .find(|batch| batch.draw_order == POINT_MARKER_DRAW_ORDER)
        .expect("marker batch");
    assert_eq!(
        batch.vertices.len(),
        4,
        "the drawing's cross still wins over the host-supplied tick"
    );
}

#[test]
fn the_point_style_participates_in_the_overlay_fingerprint() {
    let base = OverlayInputs {
        point_markers: vec![p(1.0, 2.0)],
        ..OverlayInputs::default()
    };
    let hashed = overlay_fingerprint(&base);
    assert_eq!(
        base.point_display_style,
        PointDisplayStyle::default(),
        "a host that never sets it gets the DWG defaults"
    );

    for changed in [
        PointDisplayStyle { mode: 3, size: 0.0 },
        PointDisplayStyle { mode: 0, size: 5.0 },
        PointDisplayStyle {
            mode: 0,
            size: -5.0,
        },
    ] {
        assert_ne!(
            hashed,
            overlay_fingerprint(&OverlayInputs {
                point_display_style: changed,
                ..base.clone()
            }),
            "a differently styled drawing must not reuse the cached marker glyph ({changed:?})"
        );
    }

    // Bitwise identity keeps a corrupt non-finite size a stable key instead of
    // making every preparation look like a change.
    let nan_style = PointDisplayStyle {
        mode: 0,
        size: f64::NAN,
    };
    assert_eq!(nan_style, nan_style);
    assert_eq!(
        overlay_fingerprint(&OverlayInputs {
            point_display_style: nan_style,
            ..base.clone()
        }),
        overlay_fingerprint(&OverlayInputs {
            point_display_style: nan_style,
            ..base
        })
    );
}
