//! Visible POINT markers drawn through the transient overlay
//! (ADR 0015 decision E).
//!
//! A `SemanticGeometry::Point` has no base-scene geometry: the chord tessellator
//! would degenerate it to a zero-length `[p, p]` segment, which is invisible.
//! The base-scene provider therefore skips it (`representation.point_overlay`)
//! and it is drawn here instead, as a screen-relative cross in overlay order —
//! the same transient pipeline the snap hints and tool previews use.

use super::preview::line_batch;
use super::*;
use crate::layers::LayerOverrideSet;
use cad_db::DrawingDatabase;
use cad_domain::SemanticGeometry;
use cad_representation::SpaceSelection;

/// `draw_order` of the POINT marker batch.
///
/// Real drawing content, so it draws above the reference axes/grid
/// ([`AXES_GRID_DRAW_ORDER`], `800_000`) and above the base drawing (`0`), but
/// below every transient chrome overlay — the selection highlight (`900_000`),
/// the object-snap hints ([`SNAP_HINT_DRAW_ORDER`], `1_500_000`), the
/// box-selection band ([`BOX_SELECT_DRAW_ORDER`], `1_750_000`) and the tool
/// preview ([`PREVIEW_DRAW_ORDER`], `2_000_000`) — so the live feedback the user
/// is acting on is never occluded by a marker.
///
/// **Deliberate deviation (ADR 0015 decision E):** markers draw in overlay order,
/// i.e. always on top of base-scene entities, so a POINT is never hidden behind
/// other geometry the way a base-scene entity could be.
pub const POINT_MARKER_DRAW_ORDER: i64 = 850_000;

/// Marker colour: a pale blue-white, distinct from the amber object-snap hints
/// and both preview colours.
pub const POINT_MARKER_COLOR: [f32; 3] = [0.80, 0.90, 1.0];

/// Half-extent of a POINT marker cross as a fraction of the viewport height.
///
/// The full cross therefore spans 5% of the viewport height, aligning with the
/// `PDSIZE = 0` screen-relative size. `PDMODE`/`PDSIZE` and point styles are
/// **NOT IMPLEMENTED** (ADR 0015 decision E); the shape is a fixed axis-aligned
/// plus and this size formula is a deliberate deviation.
pub const POINT_MARKER_HALF_EXTENT_FRACTION: f64 = 0.025;

/// Maximum number of POINT markers drawn in one overlay batch.
///
/// The overlay is rebuilt on every navigation step, so the marker geometry is a
/// per-frame, screen-relative cost that must stay bounded and of the same order
/// as the existing performance baseline (ADR 0015 decision E). Above this cap
/// the first [`POINT_MARKER_LIMIT`] points are drawn and **exactly one**
/// explicit diagnostic names the cap and the real count — never a silent
/// truncation, never a diagnostic per point.
pub const POINT_MARKER_LIMIT: usize = 4000;

/// The visible POINT entity positions of `space`, in draw order.
///
/// Reads the drawing being displayed and applies the same layer-visibility
/// filtering the rest of the overlay uses ([`LayerOverrideSet::is_entity_visible`]),
/// so a hidden layer's points are excluded exactly like every other entity. This
/// covers imported POINTs, not just newly created ones.
pub fn visible_point_marker_positions(
    database: &DrawingDatabase,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
) -> Vec<Point3> {
    let entities = match space {
        SpaceSelection::Model => database.model_space(),
        SpaceSelection::Paper(layout) => database.layout_entities(layout),
    };
    entities
        .into_iter()
        .filter(|entity| overrides.is_entity_visible(database, entity))
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Point(point) => Some(*point),
            _ => None,
        })
        .collect()
}

/// Build the POINT marker overlay as **one** bounded `Lines` batch
/// (ADR 0015 decision E).
///
/// The marker size is derived from `viewport_bounds` (5% of the viewport height
/// as a full cross), so it stays a constant screen size under any zoom instead
/// of a fixed world size that distorts at extreme scales.
///
/// In a 3D view `viewport_bounds` is `None`, so a screen-relative size has no
/// domain and **no** marker is emitted (ADR 0015 decision E: 3D point markers
/// are NOT IMPLEMENTED). This is an explicit branch, not an accidental
/// `unwrap_or(0.0)`.
///
/// Cost is `O(visible points)`: a single vertex buffer and a single batch, with
/// no per-point allocation.
pub fn point_marker_overlay(
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
    let half = (max.y - min.y).abs() * POINT_MARKER_HALF_EXTENT_FRACTION;
    if !half.is_finite() || half <= 0.0 {
        return out;
    }
    let drawn = if points.len() > POINT_MARKER_LIMIT {
        &points[..POINT_MARKER_LIMIT]
    } else {
        points
    };
    // One axis-aligned plus per point, emitted as two explicit `LineList`
    // segment pairs (shape fixed; point styles are NOT IMPLEMENTED).
    let mut vertices: Vec<Point3> = Vec::with_capacity(drawn.len() * 4);
    for point in drawn {
        vertices.push(Point3 {
            x: point.x - half,
            y: point.y,
            z: point.z,
        });
        vertices.push(Point3 {
            x: point.x + half,
            y: point.y,
            z: point.z,
        });
        vertices.push(Point3 {
            x: point.x,
            y: point.y - half,
            z: point.z,
        });
        vertices.push(Point3 {
            x: point.x,
            y: point.y + half,
            z: point.z,
        });
    }
    if let Some(batch) = line_batch(
        &vertices,
        POINT_MARKER_COLOR,
        document,
        POINT_MARKER_DRAW_ORDER,
        "overlay.point-marker",
    ) {
        out.batches.push(batch);
    }
    if points.len() > POINT_MARKER_LIMIT {
        let message = format!(
            "{} visible POINT entities exceed the {POINT_MARKER_LIMIT}-marker cap; \
             drawing the first {POINT_MARKER_LIMIT}",
            points.len()
        );
        out.diagnostics.push(Diagnostic {
            object: None,
            code: "overlay.point-marker-limit".into(),
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
    use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
    use cad_domain::{DatabaseId, EntityId, LayerId, ObjectId, Revision, SpaceId};
    use std::sync::Arc;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    /// The half-extent of the marker cross in world units, independent of where
    /// the point sits: half of the larger world-space X/Y span of the batch.
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
    fn point_marker_overlay_draws_one_batch_and_scales_with_the_viewport_height() {
        let points = [p(0.0, 0.0)];
        let small = point_marker_overlay(&points, viewport(100.0), DocumentId(1));
        let large = point_marker_overlay(&points, viewport(400.0), DocumentId(1));
        assert_eq!(small.drawn(), 1, "all points share one bounded batch");
        assert_eq!(large.drawn(), 1);
        assert_eq!(small.batches[0].topology, RenderTopology::Lines);
        assert_eq!(small.batches[0].draw_order, POINT_MARKER_DRAW_ORDER);
        assert_eq!(small.batches[0].color, POINT_MARKER_COLOR);
        assert!(!small.batches[0].color_unresolved);
        // One marker => two segments => four vertices.
        assert_eq!(small.batches[0].vertices.len(), 4);
        // 5% of the viewport height as a full cross: half-extent = 0.025 * height.
        let small_extent = batch_extent(&small.batches[0]);
        let large_extent = batch_extent(&large.batches[0]);
        assert!((small_extent - 2.5).abs() < 1e-4, "{small_extent}");
        assert!(
            (large_extent / small_extent - 4.0).abs() < 1e-3,
            "a 4x taller viewport must give a 4x larger marker: {large_extent} vs {small_extent}"
        );
    }

    #[test]
    fn point_marker_overlay_stops_at_the_cap_with_exactly_one_diagnostic() {
        let at_cap: Vec<Point3> = (0..POINT_MARKER_LIMIT as u64)
            .map(|i| p(i as f64, 0.0))
            .collect();
        let overlay = point_marker_overlay(&at_cap, viewport(100.0), DocumentId(1));
        assert_eq!(overlay.drawn(), 1);
        assert_eq!(overlay.batches[0].vertices.len(), POINT_MARKER_LIMIT * 4);
        assert!(
            overlay.diagnostics.is_empty(),
            "no diagnostic at exactly the cap"
        );
        assert_eq!(overlay.completeness, Completeness::Complete);

        let over_cap: Vec<Point3> = (0..(POINT_MARKER_LIMIT as u64 + 1))
            .map(|i| p(i as f64, 0.0))
            .collect();
        let overlay = point_marker_overlay(&over_cap, viewport(100.0), DocumentId(1));
        assert_eq!(overlay.drawn(), 1);
        assert_eq!(
            overlay.batches[0].vertices.len(),
            POINT_MARKER_LIMIT * 4,
            "the first {POINT_MARKER_LIMIT} markers are drawn, not fewer"
        );
        assert_eq!(
            overlay.diagnostics.len(),
            1,
            "exactly one diagnostic, never one per point"
        );
        assert_eq!(overlay.diagnostics[0].code, "overlay.point-marker-limit");
        assert!(
            overlay.diagnostics[0].message.contains("4001"),
            "the diagnostic names the real count: {}",
            overlay.diagnostics[0].message
        );
        assert!(overlay.diagnostics[0].message.contains("4000"));
        assert!(matches!(overlay.completeness, Completeness::Partial(_)));
    }

    #[test]
    fn point_marker_overlay_is_not_drawn_in_a_3d_view() {
        let points = [p(1.0, 2.0)];
        // `viewport_bounds == None` is the 3D case: no size domain, no markers.
        let overlay = point_marker_overlay(&points, None, DocumentId(1));
        assert!(overlay.is_empty());
        assert!(overlay.diagnostics.is_empty());
        assert_eq!(overlay.completeness, Completeness::Complete);
    }

    #[test]
    fn point_marker_overlay_emits_nothing_for_an_empty_set_or_a_degenerate_viewport() {
        assert!(point_marker_overlay(&[], viewport(100.0), DocumentId(1)).is_empty());
        // A zero-height viewport has no screen-relative size domain.
        assert!(point_marker_overlay(&[p(0.0, 0.0)], viewport(0.0), DocumentId(1)).is_empty());
    }

    fn point_entity(id: u128, layer: u128, point: Point3) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbPoint".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(layer),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Point(point),
            draw_order: id as i64,
        }
    }

    /// A drawing with a visible point on layer 0 and a point on hidden layer 1.
    fn drawing_with_points() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_layer(Layer {
            id: LayerId(1),
            name: "HIDDEN".into(),
            visible: false,
        })
        .unwrap();
        b.insert_entity(point_entity(1, 0, p(1.0, 1.0))).unwrap();
        b.insert_entity(point_entity(2, 1, p(2.0, 2.0))).unwrap();
        b.finish().unwrap()
    }

    #[test]
    fn hidden_layer_points_are_excluded_like_every_other_entity() {
        let db = drawing_with_points();
        let visible =
            visible_point_marker_positions(&db, &LayerOverrideSet::new(), SpaceSelection::Model);
        assert_eq!(visible, vec![p(1.0, 1.0)], "hidden layer 1 is excluded");

        // A temporary override shows the hidden layer without touching the DB.
        let mut shown = LayerOverrideSet::new();
        shown.set(LayerId(1), true);
        let mut both = visible_point_marker_positions(&db, &shown, SpaceSelection::Model);
        both.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
        assert_eq!(both, vec![p(1.0, 1.0), p(2.0, 2.0)]);

        // Hiding the visible layer while showing the hidden one leaves only the
        // hidden-layer point.
        let mut hidden = LayerOverrideSet::new();
        hidden.set(LayerId(0), false);
        hidden.set(LayerId(1), true);
        assert_eq!(
            visible_point_marker_positions(&db, &hidden, SpaceSelection::Model),
            vec![p(2.0, 2.0)]
        );
    }

    fn overlays_without_reference() -> OverlayInputs {
        OverlayInputs {
            visibility: OverlayVisibility {
                axes: false,
                grid: false,
                ..OverlayVisibility::default()
            },
            ..OverlayInputs::default()
        }
    }

    fn marker_batches(scene: &crate::render_scene::PreparedScene) -> Vec<&RenderBatch> {
        scene
            .highlight
            .added
            .iter()
            .filter(|batch| batch.draw_order == POINT_MARKER_DRAW_ORDER)
            .collect()
    }

    #[test]
    fn the_scene_assembly_marks_imported_points_from_the_displayed_drawing() {
        let db = Arc::new(drawing_with_points());
        let layers = LayerOverrideSet::new();
        let mut overlays = overlays_without_reference();
        overlays.viewport_bounds = viewport(100.0);

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
        let batches = marker_batches(scene);
        assert_eq!(batches.len(), 1, "one marker batch for the visible point");
        // Only the visible-layer point is marked (the hidden-layer point is not).
        assert_eq!(batches[0].vertices.len(), 4);
        assert_eq!(batches[0].color, POINT_MARKER_COLOR);
        // The base scene still draws no zero-length line for the POINT.
        assert!(
            scene
                .base
                .added
                .iter()
                .all(|batch| batch.vertices.len() != 2 || batch.vertices[0] != batch.vertices[1]),
            "the base scene must not carry a zero-length POINT segment"
        );
    }

    #[test]
    fn a_camera_only_change_rescales_the_markers_without_rebuilding_the_base() {
        let db = Arc::new(drawing_with_points());
        let layers = LayerOverrideSet::new();
        let mut overlays = overlays_without_reference();
        overlays.viewport_bounds = viewport(100.0);
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
        let base = controller.ready.as_ref().unwrap().base.clone();
        let base_revision = controller.ready.as_ref().unwrap().base_revision;
        let extent_before = batch_extent(marker_batches(controller.ready.as_ref().unwrap())[0]);

        // Zoom out: the viewport is twice as tall. The base must be reused and
        // only the overlay rescaled.
        overlays.viewport_bounds = viewport(200.0);
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
        assert!(Arc::ptr_eq(&base, &scene.base), "base must be reused");
        assert_eq!(scene.base_revision, base_revision);
        let extent_after = batch_extent(marker_batches(scene)[0]);
        assert!(
            (extent_after / extent_before - 2.0).abs() < 1e-3,
            "a 2x taller viewport must double the marker size: {extent_after} vs {extent_before}"
        );
    }

    #[test]
    fn overlay_fingerprint_tracks_viewport_and_point_markers() {
        let baseline = overlay_fingerprint(&OverlayInputs::default());

        let with_point = OverlayInputs {
            point_markers: vec![p(0.0, 0.0)],
            ..OverlayInputs::default()
        };
        let hashed = overlay_fingerprint(&with_point);
        assert_ne!(baseline, hashed, "a point marker is part of the overlay");

        // A changed point set invalidates the overlay.
        let more_points = OverlayInputs {
            point_markers: vec![p(0.0, 0.0), p(1.0, 0.0)],
            ..OverlayInputs::default()
        };
        assert_ne!(hashed, overlay_fingerprint(&more_points));

        // The camera/viewport bounds are part of the fingerprint.
        let with_view = OverlayInputs {
            viewport_bounds: viewport(100.0),
            ..OverlayInputs::default()
        };
        let moved_view = OverlayInputs {
            viewport_bounds: viewport(200.0),
            ..OverlayInputs::default()
        };
        assert_ne!(
            overlay_fingerprint(&with_view),
            overlay_fingerprint(&moved_view)
        );

        // Rebuilding with identical inputs is stable.
        assert_eq!(hashed, overlay_fingerprint(&with_point));
    }
}
