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

/// `$PDMODE` bits 32/64 add a circle/square around the base glyph. Those shapes
/// are NOT IMPLEMENTED: the base glyph is drawn, the requested shapes are named
/// once, and no substitute geometry is invented.
#[test]
fn surrounding_shape_bits_are_reported_and_only_the_base_glyph_is_drawn() {
    // 32 | 3 -> cross, circle requested.
    let circle_cross = overlay_at(style(35, 0.0));
    assert_offsets(
        &offsets(&circle_cross),
        &[(-2.5, -2.5), (2.5, 2.5), (-2.5, 2.5), (2.5, -2.5)],
    );
    assert_eq!(codes(&circle_cross), vec!["overlay.point-mode-shape-bits"]);
    let message = &circle_cross.diagnostics[0].message;
    assert!(message.contains("32"), "{message}");
    assert!(message.contains("circle"), "{message}");
    assert!(message.contains("NOT IMPLEMENTED"), "{message}");
    assert_partial(&circle_cross);

    // 64 | 2 -> plus, square requested.
    let square_plus = overlay_at(style(66, 0.0));
    assert_offsets(
        &offsets(&square_plus),
        &[(-2.5, 0.0), (2.5, 0.0), (0.0, -2.5), (0.0, 2.5)],
    );
    assert_eq!(codes(&square_plus), vec!["overlay.point-mode-shape-bits"]);
    assert!(square_plus.diagnostics[0].message.contains("64"));
    assert!(square_plus.diagnostics[0].message.contains("square"));

    // 96 = 32 | 64 -> both shapes named in the same single diagnostic.
    let both = overlay_at(style(96 | 4, 0.0));
    assert_offsets(&offsets(&both), &[(0.0, -2.5), (0.0, 2.5)]);
    assert_eq!(codes(&both), vec!["overlay.point-mode-shape-bits"]);
    let message = &both.diagnostics[0].message;
    assert!(
        message.contains("32") && message.contains("64"),
        "{message}"
    );
    assert_eq!(
        both.batches[0].vertices.len(),
        2,
        "no circle/square geometry is invented around the tick"
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
