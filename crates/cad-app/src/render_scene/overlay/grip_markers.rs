//! Grip handles drawn through the transient overlay.
//!
//! A selected entity exposes draggable handles (`crate::grips`). The model of
//! *what* a handle is and *how* a drag is committed lives there; this module is
//! only the visible marker: a screen-relative hollow square per handle, drawn in
//! overlay order so it is never lost under the selection tint.
//!
//! ## The list is host-fed
//!
//! [`grip_marker_overlay`] takes a plain `&[Point3]`: **one entry per grip**, in
//! whatever order the host enumerated the selected entities. It never scans the
//! database, so a host that has not resolved grips yet (nothing selected, a
//! build without grip editing) simply passes an empty slice and gets no batch —
//! an explicit empty overlay, never a fabricated one.
//!
//! Because the list is positional, the host also owns the drag preview: while a
//! grip is being dragged it replaces that entry with the current drag target, so
//! the **moved marker is the preview**. No second "ghost" marker and no extra
//! preview input are needed.
//!
//! **Grip interaction is not part of this module.** Hit testing, dragging,
//! snapping and the commit/transaction path belong to `crate::grips` and the
//! host; nothing here reads input, mutates the database or opens a transaction.
//!
//! ## Shape
//!
//! The marker is a **hollow square** (four short segments), deliberately *not*
//! the axis-aligned plus of the POINT marker
//! ([`point_marker_overlay`](super::point_marker_overlay)): a grip must never
//! read as a POINT entity marker, and the square silhouette is the AutoCAD grip
//! affordance.

use super::preview::line_batch;
use super::*;

/// `draw_order` of the grip marker batch.
///
/// Grips only exist on **selected** entities, so they draw above the selection
/// highlight (`cad_scene::HIGHLIGHT_DRAW_ORDER`, `900_000`): the marker is the
/// affordance the user aims at, and the highlight tint must never cover it.
///
/// It stays below the live box-selection rubber band
/// ([`BOX_SELECT_DRAW_ORDER`], `1_750_000`) and therefore below the tool preview
/// (`2_000_000`), the pointer cursor (`2_500_000`) and the object-snap hints
/// (`3_000_000`): those are the live feedback of the gesture in progress, which
/// must never be occluded by a static handle marker.
pub const GRIP_MARKER_DRAW_ORDER: i64 = 950_000;

/// Grip marker colour: a saturated AutoCAD-like grip blue, distinct from the
/// pale blue-white POINT marker ([`POINT_MARKER_COLOR`]), the amber selection
/// highlight, the amber object-snap hints and both preview colours.
pub const GRIP_MARKER_COLOR: [f32; 3] = [0.15, 0.65, 1.0];

/// Half-extent of a grip square as a fraction of the viewport height.
///
/// The full square therefore spans **1%** of the viewport height — one fifth of
/// the POINT marker's 5% cross. Grips sit on entities the user has just
/// selected, often in clusters (both ends of a LINE), so they must stay small
/// enough to read as separate handles rather than blobs. Like the POINT marker
/// this is a **screen-relative** size (derived from the viewport height, not
/// from world units), so a grip keeps a constant on-screen size under any zoom.
pub const GRIP_MARKER_HALF_EXTENT_FRACTION: f64 = 0.005;

/// Maximum number of grip markers drawn in one overlay batch.
///
/// The overlay is rebuilt on every navigation step, so the marker geometry is a
/// per-frame, screen-relative cost that must stay bounded. Each marker is four
/// segments (eight vertices), so this cap keeps the worst case at **16 000**
/// vertices — the same vertex budget as the POINT marker cap
/// ([`POINT_MARKER_LIMIT`] `4000` × four vertices).
///
/// Above the cap the first [`GRIP_MARKER_LIMIT`] handles are drawn and **exactly
/// one** explicit diagnostic names the cap and the real count — never a silent
/// truncation, never a diagnostic per handle.
pub const GRIP_MARKER_LIMIT: usize = 2000;

/// Build the grip marker overlay as **one** bounded `Lines` batch.
///
/// `points` is one entry per grip, host-fed (see the module doc). The marker
/// size is derived from `viewport_bounds` (1% of the viewport height as a full
/// square), so a grip stays a constant screen size under any zoom instead of a
/// fixed world size that distorts at extreme scales.
///
/// In a 3D view `viewport_bounds` is `None`, so a screen-relative size has no
/// domain and **no** marker is emitted — an explicit branch, not an accidental
/// `unwrap_or(0.0)`. The same holds for an empty list and a degenerate
/// (non-finite or non-positive) viewport height.
///
/// Cost is `O(visible grips)`: a single capacity-reserved vertex buffer and a
/// single batch, with no per-marker allocation.
pub fn grip_marker_overlay(
    points: &[Point3],
    viewport_bounds: Option<(Point3, Point3)>,
    document: DocumentId,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    if points.is_empty() {
        return out;
    }
    // 3D view: no screen-relative size domain -> no markers.
    let Some((min, max)) = viewport_bounds else {
        return out;
    };
    let half = (max.y - min.y).abs() * GRIP_MARKER_HALF_EXTENT_FRACTION;
    if !half.is_finite() || half <= 0.0 {
        return out;
    }
    let drawn = if points.len() > GRIP_MARKER_LIMIT {
        &points[..GRIP_MARKER_LIMIT]
    } else {
        points
    };
    // One hollow square per grip: four corner-to-corner segments, i.e. eight
    // `LineList` vertices. Corners are emitted per segment (no shared-vertex
    // index buffer), exactly like the other overlay line batches.
    let mut vertices: Vec<Point3> = Vec::with_capacity(drawn.len() * 8);
    for point in drawn {
        let (left, right) = (point.x - half, point.x + half);
        let (bottom, top) = (point.y - half, point.y + half);
        let corner = |x: f64, y: f64| Point3 { x, y, z: point.z };
        let square = [
            corner(left, bottom),
            corner(right, bottom),
            corner(right, bottom),
            corner(right, top),
            corner(right, top),
            corner(left, top),
            corner(left, top),
            corner(left, bottom),
        ];
        vertices.extend_from_slice(&square);
    }
    if let Some(batch) = line_batch(
        &vertices,
        GRIP_MARKER_COLOR,
        document,
        GRIP_MARKER_DRAW_ORDER,
        "overlay.grip-marker",
    ) {
        out.batches.push(batch);
    }
    if points.len() > GRIP_MARKER_LIMIT {
        let message = format!(
            "{} grips exceed the {GRIP_MARKER_LIMIT}-marker cap; \
             drawing the first {GRIP_MARKER_LIMIT}",
            points.len()
        );
        out.diagnostics.push(Diagnostic {
            object: None,
            code: "overlay.grip-marker-limit".into(),
            message: message.clone(),
        });
        out.completeness = Completeness::Partial(vec![message]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_scene::CadSceneController;
    use crate::viewer_config::OverlayVisibility;
    use cad_db::{DrawingDatabase, DrawingDatabaseBuilder, Layer};
    use cad_domain::{DatabaseId, LayerId};
    use std::sync::Arc;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    /// The half-extent of one square, independent of where it sits: half of the
    /// larger world-space X/Y span of the batch.
    fn batch_extent(batch: &RenderBatch) -> f64 {
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for vertex in &batch.vertices {
            let x = batch.local_origin.x + f64::from(vertex[0]);
            let y = batch.local_origin.y + f64::from(vertex[1]);
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        (max_x - min_x).max(max_y - min_y) * 0.5
    }

    fn viewport(height: f64) -> Option<(Point3, Point3)> {
        Some((p(0.0, 0.0), p(100.0, height)))
    }

    #[test]
    fn grip_marker_overlay_draws_one_batch_and_scales_with_the_viewport_height() {
        let grips = [p(0.0, 0.0)];
        let small = grip_marker_overlay(&grips, viewport(100.0), DocumentId(1));
        let large = grip_marker_overlay(&grips, viewport(400.0), DocumentId(1));
        assert_eq!(small.drawn(), 1, "all grips share one bounded batch");
        assert_eq!(large.drawn(), 1);
        assert_eq!(small.batches[0].topology, RenderTopology::Lines);
        assert_eq!(small.batches[0].draw_order, GRIP_MARKER_DRAW_ORDER);
        assert_eq!(small.batches[0].color, GRIP_MARKER_COLOR);
        assert!(!small.batches[0].color_unresolved);
        // One marker => four closed square segments => eight vertices.
        assert_eq!(small.batches[0].vertices.len(), 8);
        // 1% of the viewport height as a full square: half-extent = 0.005 * h.
        let small_extent = batch_extent(&small.batches[0]);
        let large_extent = batch_extent(&large.batches[0]);
        assert!((small_extent - 0.5).abs() < 1e-6, "{small_extent}");
        assert!(
            (large_extent / small_extent - 4.0).abs() < 1e-3,
            "a 4x taller viewport must give a 4x larger marker: {large_extent} vs {small_extent}"
        );
        // The square is smaller than the POINT cross at the same viewport, so a
        // grip can never be mistaken for a POINT marker by size.
        assert!(
            small_extent < 100.0 * POINT_MARKER_HALF_EXTENT_FRACTION,
            "a grip square stays below the POINT marker cross size"
        );
    }

    #[test]
    fn a_grip_is_a_hollow_square_not_a_cross() {
        let overlay = grip_marker_overlay(&[p(0.0, 0.0)], viewport(100.0), DocumentId(1));
        let batch = &overlay.batches[0];
        // Map every vertex back to world space.
        let world: Vec<(f64, f64)> = batch
            .vertices
            .iter()
            .map(|v| {
                (
                    batch.local_origin.x + f64::from(v[0]),
                    batch.local_origin.y + f64::from(v[1]),
                )
            })
            .collect();
        assert_eq!(world.len(), 8);
        let half = 100.0 * GRIP_MARKER_HALF_EXTENT_FRACTION;
        let (l, r, b, t) = (-half, half, -half, half);
        // Four closed corner-to-corner segments, in emission order.
        let expected = [
            (l, b),
            (r, b),
            (r, b),
            (r, t),
            (r, t),
            (l, t),
            (l, t),
            (l, b),
        ];
        for (index, (actual, want)) in world.iter().zip(expected).enumerate() {
            assert!(
                (actual.0 - want.0).abs() < 1e-9 && (actual.1 - want.1).abs() < 1e-9,
                "vertex {index}: {actual:?} != {want:?}"
            );
        }
        // A cross would have an axis-aligned vertex exactly on the grip point.
        assert!(
            !world.iter().any(|(x, y)| x.abs() < 1e-9 && y.abs() < 1e-9),
            "no vertex sits on the grip point: this is a hollow square, not a plus"
        );
    }

    #[test]
    fn grip_marker_overlay_stops_at_the_cap_with_exactly_one_diagnostic() {
        let at_cap: Vec<Point3> = (0..GRIP_MARKER_LIMIT as u64)
            .map(|i| p(i as f64, 0.0))
            .collect();
        let overlay = grip_marker_overlay(&at_cap, viewport(100.0), DocumentId(1));
        assert_eq!(overlay.drawn(), 1);
        assert_eq!(overlay.batches[0].vertices.len(), GRIP_MARKER_LIMIT * 8);
        assert!(
            overlay.diagnostics.is_empty(),
            "no diagnostic at exactly the cap"
        );
        assert_eq!(overlay.completeness, Completeness::Complete);

        let over_cap: Vec<Point3> = (0..(GRIP_MARKER_LIMIT as u64 + 1))
            .map(|i| p(i as f64, 0.0))
            .collect();
        let overlay = grip_marker_overlay(&over_cap, viewport(100.0), DocumentId(1));
        assert_eq!(overlay.drawn(), 1);
        assert_eq!(
            overlay.batches[0].vertices.len(),
            GRIP_MARKER_LIMIT * 8,
            "the first {GRIP_MARKER_LIMIT} markers are drawn, not fewer"
        );
        assert_eq!(
            overlay.diagnostics.len(),
            1,
            "exactly one diagnostic, never one per grip"
        );
        assert_eq!(overlay.diagnostics[0].code, "overlay.grip-marker-limit");
        assert!(
            overlay.diagnostics[0].message.contains("2001"),
            "the diagnostic names the real count: {}",
            overlay.diagnostics[0].message
        );
        assert!(overlay.diagnostics[0].message.contains("2000"));
        assert!(matches!(overlay.completeness, Completeness::Partial(_)));
    }

    #[test]
    fn grip_marker_overlay_is_not_drawn_in_a_3d_view() {
        // `viewport_bounds == None` is the 3D case: no size domain, no markers.
        let overlay = grip_marker_overlay(&[p(1.0, 2.0)], None, DocumentId(1));
        assert!(overlay.is_empty());
        assert!(overlay.diagnostics.is_empty());
        assert_eq!(overlay.completeness, Completeness::Complete);
        // No grips at all, and a degenerate viewport, are equally explicit empties.
        assert!(grip_marker_overlay(&[], viewport(100.0), DocumentId(1)).is_empty());
        assert!(grip_marker_overlay(&[p(0.0, 0.0)], viewport(0.0), DocumentId(1)).is_empty());
    }

    #[test]
    fn overlay_fingerprint_tracks_a_moved_grip() {
        let baseline = overlay_fingerprint(&OverlayInputs::default());
        let with_grip = OverlayInputs {
            grip_markers: vec![p(1.0, 2.0)],
            ..OverlayInputs::default()
        };
        let hashed = overlay_fingerprint(&with_grip);
        assert_ne!(baseline, hashed, "a grip marker is part of the overlay");

        // A grip dragged to a new position invalidates the transient overlay, so
        // the moved marker really is the live drag preview.
        let moved = OverlayInputs {
            grip_markers: vec![p(3.0, 2.0)],
            ..OverlayInputs::default()
        };
        assert_ne!(
            hashed,
            overlay_fingerprint(&moved),
            "moving a grip must rebuild the overlay"
        );

        // A changed grip count invalidates it too.
        let more = OverlayInputs {
            grip_markers: vec![p(1.0, 2.0), p(3.0, 2.0)],
            ..OverlayInputs::default()
        };
        assert_ne!(hashed, overlay_fingerprint(&more));

        // An unchanged list is stable (the overlay delta is not thrashed).
        assert_eq!(hashed, overlay_fingerprint(&with_grip));

        // The list participates in `OverlayInputs::is_empty`.
        assert!(!with_grip.is_empty());
    }

    fn empty_drawing() -> DrawingDatabase {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        builder
            .insert_layer(Layer {
                id: LayerId(0),
                name: "0".into(),
                visible: true,
            })
            .unwrap();
        builder.finish().unwrap()
    }

    fn grip_batches(scene: &crate::render_scene::PreparedScene) -> Vec<&RenderBatch> {
        scene
            .highlight
            .added
            .iter()
            .filter(|batch| batch.draw_order == GRIP_MARKER_DRAW_ORDER)
            .collect()
    }

    fn prepare(overlays: &OverlayInputs) -> CadSceneController {
        let mut controller = CadSceneController::default();
        controller
            .prepare_shared_with_overlays(
                Some(Arc::new(empty_drawing())),
                DocumentId(1),
                None,
                &crate::layers::LayerOverrideSet::new(),
                cad_representation::SpaceSelection::Model,
                overlays,
            )
            .unwrap();
        controller
    }

    fn overlays_with(highlight: bool) -> OverlayInputs {
        OverlayInputs {
            visibility: OverlayVisibility {
                axes: false,
                grid: false,
                selection_highlight: highlight,
                ..OverlayVisibility::default()
            },
            viewport_bounds: viewport(100.0),
            grip_markers: vec![p(1.0, 1.0), p(2.0, 1.0)],
            ..OverlayInputs::default()
        }
    }

    #[test]
    fn the_scene_assembly_draws_grips_only_with_the_selection_highlight_flag() {
        let on = prepare(&overlays_with(true));
        let batches = grip_batches(on.ready.as_ref().unwrap());
        assert_eq!(batches.len(), 1, "one grip batch for the two handles");
        assert_eq!(batches[0].vertices.len(), 2 * 8);
        assert_eq!(batches[0].color, GRIP_MARKER_COLOR);

        // Grips are chrome on a selection, so the selection-highlight flag gates
        // them: with it off, a host that still feeds grips gets no batch.
        let off = prepare(&overlays_with(false));
        assert!(
            grip_batches(off.ready.as_ref().unwrap()).is_empty(),
            "no grip batch when the selection highlight is disabled"
        );
    }

    #[test]
    fn grip_markers_are_their_own_batch_above_the_highlight() {
        let scene = prepare(&overlays_with(true));
        let batch = grip_batches(scene.ready.as_ref().unwrap())[0];
        assert!(
            batch.draw_order > cad_scene::HIGHLIGHT_DRAW_ORDER,
            "a grip draws above the selection tint on the same entity"
        );
        assert!(
            batch.draw_order < BOX_SELECT_DRAW_ORDER,
            "a grip draws below the live box-selection band"
        );
        // A POINT marker batch is a separate draw order, not merged into this one.
        assert_ne!(batch.draw_order, POINT_MARKER_DRAW_ORDER);
    }
}
