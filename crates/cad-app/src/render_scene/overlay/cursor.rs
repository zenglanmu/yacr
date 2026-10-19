//! The pointer cursor overlay: a viewport-spanning crosshair with a square
//! pickbox at the intersection (the AutoCAD "crosshair + pickbox" affordance).
//!
//! The CAD partition is a native surface no DOM overlay can draw on (ADR-0014
//! §5, ADR-0002), so the crosshair is drawn through the renderer's transient
//! overlay — the same pure, non-mutating pipeline the snap hints and tool
//! previews use. It never writes the database, the revision or the history: a
//! moved cursor simply republishes the overlay inputs.
//!
//! The crosshair spans the whole viewport through the cursor as two world-space
//! lines (derived from [`OverlayInputs::viewport_bounds`]). The pickbox is a
//! square centred on the cursor whose **half-side** equals the pick tolerance
//! ([`cad_domain::TolerancePolicy::pickbox_logical_pixels`] logical pixels, the
//! AutoCAD `PICKBOX` analogue this project's click predicate uses) converted to
//! world units by the current view scale, so the box circumscribes the region a
//! click can actually hit.
//!
//! The whole overlay is one bounded `Lines` batch: two full-extent crosshair
//! segments (four vertices) plus, when the view scale is usable, four closed
//! pickbox segments (eight vertices).

use super::preview::line_batch;
use super::*;

/// `draw_order` of the pointer cursor overlay.
///
/// Above the geometry and above every transient overlay **except the object-snap
/// hints** ([`SNAP_HINT_DRAW_ORDER`], `3_000_000`): the crosshair **is** the
/// pointer, so AutoCAD paints it over the geometry and over the transient
/// graphics, and sitting above the tool preview ([`PREVIEW_DRAW_ORDER`],
/// `2_000_000`) guarantees the live rubber band the user is dragging can never
/// hide the pointer it is anchored to. The snap marker is the one thing drawn
/// above it, because a marker painted over by the pickbox would stop being
/// readable exactly when the pointer sits on the snap target.
pub const CURSOR_DRAW_ORDER: i64 = 2_500_000;

/// Colour of the crosshair and pickbox: a cool mid-grey.
///
/// Chosen to read against **both** the pure-black CAD background
/// (`cad_render_wgpu::CAD_CLEAR_COLOR`) and the default pure-white geometry
/// colour (`cad_scene`'s documented `[1, 1, 1]` default), which a white crosshair
/// (AutoCAD's own colour) or a black one would not. It is also distinct from
/// every other overlay colour: amber snap hints, magenta draw preview, cyan
/// measurement preview, the pale-blue POINT markers and the blue/green
/// box-selection band.
pub const CURSOR_COLOR: [f32; 3] = [0.72, 0.75, 0.80];

/// Build the pointer cursor overlay for `cursor`.
///
/// `viewport_bounds` is the exact world-space view rectangle (the same input the
/// reference grid uses) and `view_scale` is world units per logical pixel. A
/// missing cursor, a missing view rectangle (a 3D view has none), a degenerate
/// rectangle or a non-finite cursor yields **no** batch — an explicit empty
/// overlay, never a fabricated one. A degenerate view scale still draws the
/// crosshair but omits the pickbox, since a screen-relative pickbox size has no
/// domain.
pub fn cursor_overlay(
    cursor: Option<Point3>,
    viewport_bounds: Option<(Point3, Point3)>,
    view_scale: Option<f64>,
    document: DocumentId,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    let Some(cursor) = cursor else {
        return out;
    };
    let Some((min, max)) = viewport_bounds else {
        return out;
    };
    // A non-finite cursor, or a view rectangle with no extent, is not a valid
    // view: emit nothing rather than a zero-length or NaN batch.
    if !is_finite(cursor) || !is_finite(min) || !is_finite(max) || max.x <= min.x || max.y <= min.y
    {
        return out;
    }
    let mut points: Vec<Point3> = Vec::with_capacity(12);
    // Crosshair: two full-extent lines through the cursor. `Lines` topology pairs
    // consecutive vertices, so each line is one explicit pair.
    points.push(Point3 {
        x: min.x,
        y: cursor.y,
        z: cursor.z,
    });
    points.push(Point3 {
        x: max.x,
        y: cursor.y,
        z: cursor.z,
    });
    points.push(Point3 {
        x: cursor.x,
        y: min.y,
        z: cursor.z,
    });
    points.push(Point3 {
        x: cursor.x,
        y: max.y,
        z: cursor.z,
    });
    if let Some(half) = pickbox_half_side(view_scale) {
        let corner = |dx: f64, dy: f64| Point3 {
            x: cursor.x + dx,
            y: cursor.y + dy,
            z: cursor.z,
        };
        let a = corner(-half, -half);
        let b = corner(half, -half);
        let c = corner(half, half);
        let d = corner(-half, half);
        for (start, end) in [(a, b), (b, c), (c, d), (d, a)] {
            points.push(start);
            points.push(end);
        }
    }
    // One bounded batch; `line_batch` rejects fewer than two vertices.
    if let Some(batch) = line_batch(
        &points,
        CURSOR_COLOR,
        document,
        CURSOR_DRAW_ORDER,
        "overlay.cursor",
    ) {
        out.batches.push(batch);
    }
    out
}

/// The pickbox half-side in world units, or `None` when the scale is degenerate.
///
/// The half-side is the pick tolerance in logical pixels
/// ([`cad_domain::TolerancePolicy::pickbox_logical_pixels`], the same value the
/// click predicate uses) times the view scale, so the box's inscribed circle is
/// exactly the region a click can hit. A missing or degenerate scale yields no
/// pickbox (never a zero/NaN one).
fn pickbox_half_side(view_scale: Option<f64>) -> Option<f64> {
    let scale = view_scale.filter(|scale| scale.is_finite() && *scale > 0.0)?;
    let half = (cad_domain::TolerancePolicy::default().pickbox_logical_pixels * scale)
        .max(f64::MIN_POSITIVE);
    (half.is_finite() && half > 0.0).then_some(half)
}

fn is_finite(point: Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn view() -> Option<(Point3, Point3)> {
        Some((p(-50.0, -30.0), p(50.0, 30.0)))
    }

    /// The world-space half-extent of the pickbox: half of the largest X/Y span
    /// of the batch (the crosshair always spans the whole view rectangle, so the
    /// pickbox extent is measured on the cursor-centred box by scanning only the
    /// last eight vertices).
    fn pickbox_half_extent(batch: &RenderBatch) -> f64 {
        let tail = &batch.vertices[batch.vertices.len() - 8..];
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        for vertex in tail {
            let x = batch.local_origin.x + f64::from(vertex[0]);
            min_x = min_x.min(x);
            max_x = max_x.max(x);
        }
        (max_x - min_x) * 0.5
    }

    #[test]
    fn cursor_overlay_draws_one_crosshair_and_pickbox_batch() {
        let overlay = cursor_overlay(Some(p(1.0, 2.0)), view(), Some(0.5), DocumentId(1));
        assert_eq!(overlay.drawn(), 1, "one bounded cursor batch");
        let batch = &overlay.batches[0];
        assert_eq!(batch.topology, RenderTopology::Lines);
        assert_eq!(batch.draw_order, CURSOR_DRAW_ORDER);
        assert_eq!(batch.color, CURSOR_COLOR);
        assert!(!batch.color_unresolved);
        // Two crosshair segments (4 vertices) + four pickbox segments (8).
        assert_eq!(batch.vertices.len(), 12);
    }

    #[test]
    fn the_crosshair_spans_the_view_rectangle_through_the_cursor() {
        let overlay = cursor_overlay(Some(p(1.0, 2.0)), view(), Some(0.5), DocumentId(1));
        let batch = &overlay.batches[0];
        let v = |i: usize| {
            [
                batch.local_origin.x + f64::from(batch.vertices[i][0]),
                batch.local_origin.y + f64::from(batch.vertices[i][1]),
            ]
        };
        // Horizontal line: x spans the view, y is the cursor's.
        assert!((v(0)[0] - -50.0).abs() < 1e-6 && (v(1)[0] - 50.0).abs() < 1e-6);
        assert!((v(0)[1] - 2.0).abs() < 1e-6 && (v(1)[1] - 2.0).abs() < 1e-6);
        // Vertical line: y spans the view, x is the cursor's.
        assert!((v(2)[1] - -30.0).abs() < 1e-6 && (v(3)[1] - 30.0).abs() < 1e-6);
        assert!((v(2)[0] - 1.0).abs() < 1e-6 && (v(3)[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_pickbox_half_side_is_the_pick_tolerance_times_the_scale() {
        let policy = cad_domain::TolerancePolicy::default();
        let scale = 0.5;
        let overlay = cursor_overlay(Some(p(1.0, 2.0)), view(), Some(scale), DocumentId(1));
        let half = pickbox_half_extent(&overlay.batches[0]);
        let expected = policy.pickbox_logical_pixels * scale;
        assert!(
            (half - expected).abs() < 1e-6,
            "the pickbox half-side must be PICKBOX × scale: {half} vs {expected}"
        );
        // The box is centred on the cursor.
        let batch = &overlay.batches[0];
        let tail = &batch.vertices[batch.vertices.len() - 8..];
        let xs: Vec<f64> = tail
            .iter()
            .map(|v| batch.local_origin.x + f64::from(v[0]))
            .collect();
        let center = (xs.iter().cloned().fold(f64::INFINITY, f64::min)
            + xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max))
            * 0.5;
        assert!(
            (center - 1.0).abs() < 1e-6,
            "the box is centred on the cursor"
        );
    }

    #[test]
    fn a_degenerate_scale_omits_the_pickbox_but_keeps_the_crosshair() {
        for scale in [
            None,
            Some(0.0),
            Some(-1.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            let overlay = cursor_overlay(Some(p(0.0, 0.0)), view(), scale, DocumentId(1));
            assert_eq!(
                overlay.drawn(),
                1,
                "the crosshair still draws for {scale:?}"
            );
            assert_eq!(
                overlay.batches[0].vertices.len(),
                4,
                "no pickbox for a degenerate scale {scale:?}"
            );
        }
    }

    #[test]
    fn cursor_overlay_is_absent_without_a_cursor_or_a_view() {
        // No cursor position: an explicit empty overlay, never a fabricated one.
        assert!(cursor_overlay(None, view(), Some(0.5), DocumentId(1)).is_empty());
        // A 3D view has no planar view rectangle.
        assert!(cursor_overlay(Some(p(0.0, 0.0)), None, Some(0.5), DocumentId(1)).is_empty());
        // A degenerate view rectangle is not a valid view.
        assert!(cursor_overlay(
            Some(p(0.0, 0.0)),
            Some((p(1.0, 0.0), p(1.0, 0.0))),
            Some(0.5),
            DocumentId(1)
        )
        .is_empty());
        // A non-finite cursor is refused.
        assert!(cursor_overlay(
            Some(Point3 {
                x: f64::NAN,
                y: 0.0,
                z: 0.0
            }),
            view(),
            Some(0.5),
            DocumentId(1)
        )
        .is_empty());
    }
}
