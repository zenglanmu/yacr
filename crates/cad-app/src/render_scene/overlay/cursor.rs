//! The pointer cursor overlay: a viewport-spanning crosshair with a square
//! pickbox at the intersection (the AutoCAD "crosshair + pickbox" affordance).
//!
//! The CAD partition is a native surface no DOM overlay can draw on (ADR-0014
//! §5, ADR-0002), so the crosshair is drawn through the renderer's transient
//! overlay — the same pure, non-mutating pipeline the snap hints and tool
//! previews use. It never writes the database, the revision or the history, and
//! under ADR 0028 it does not even enter scene preparation/publication: a moved
//! cursor only re-feeds this geometry to the renderer's transient buffer.
//!
//! The crosshair spans the view through the cursor as two world-space lines
//! (derived from [`OverlayInputs::viewport_bounds`]): each arm is a configured
//! percentage of its cursor-to-edge span, so the default 100 % is the historical
//! full-viewport crosshair and a smaller value draws a shorter cross. The pickbox
//! is a square centred on the cursor whose **half-side** equals the configured
//! pickbox size (the effective `view.pickbox.sizePixels`, which is exactly
//! [`cad_domain::TolerancePolicy::pickbox_logical_pixels`], the AutoCAD `PICKBOX`
//! analogue this project's click predicate uses) converted to world units by the
//! current view scale, so the box circumscribes the region a click can actually
//! hit. The crosshair size never changes it.
//!
//! The whole geometry is bounded: two full-extent crosshair segments plus, when
//! the view scale is usable, four closed pickbox segments (at most
//! [`cad_scene::MAX_TRANSIENT_CURSOR_SEGMENTS`]). The geometry is consumed by the
//! renderer's transient overlay (ADR 0028); it is not a committed scene batch.

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

/// The crosshair percentage that reproduces the historical full-viewport
/// crosshair: every arm reaches its view rectangle edge.
pub const DEFAULT_CROSSHAIR_PERCENT: u32 = 100;

/// The renderer-owned transient cursor geometry for `cursor` (ADR 0028 D2).
///
/// A missing cursor, a missing view rectangle (a 3D view has none), a degenerate
/// rectangle or a non-finite cursor yields `None` (an explicit empty, never a
/// fabricated cursor). A degenerate view scale still draws the crosshair but
/// omits the pickbox.
///
/// The returned segments are **world-space** `[start, end]` pairs: two crosshair
/// arms plus up to four pickbox sides (at most
/// [`cad_scene::MAX_TRANSIENT_CURSOR_SEGMENTS`]). `color` carries the same
/// sanitised `CURSOR_COLOR` and opaque alpha the legacy batch used, and
/// `draw_order` is [`CURSOR_DRAW_ORDER`], so the transient merges into the
/// overlay sort between the tool preview and the snap hints.
pub fn cursor_geometry(
    cursor: Option<Point3>,
    viewport_bounds: Option<(Point3, Point3)>,
    view_scale: Option<f64>,
    pickbox_logical_pixels: f64,
    crosshair_percent: u32,
) -> Option<cad_scene::TransientCursor> {
    let cursor = cursor?;
    let (min, max) = viewport_bounds?;
    // A non-finite cursor, or a view rectangle with no extent, is not a valid
    // view: emit nothing rather than a zero-length or NaN batch.
    if !is_finite(cursor) || !is_finite(min) || !is_finite(max) || max.x <= min.x || max.y <= min.y
    {
        return None;
    }
    let mut points: Vec<Point3> = Vec::with_capacity(12);
    // Crosshair: two lines through the cursor whose arms are the configured
    // percentage of the cursor-to-edge span. `Lines` topology pairs consecutive
    // vertices, so each line is one explicit pair.
    let (x0, x1) = crosshair_arms(min.x, cursor.x, max.x, crosshair_percent);
    let (y0, y1) = crosshair_arms(min.y, cursor.y, max.y, crosshair_percent);
    points.push(Point3 {
        x: x0,
        y: cursor.y,
        z: cursor.z,
    });
    points.push(Point3 {
        x: x1,
        y: cursor.y,
        z: cursor.z,
    });
    points.push(Point3 {
        x: cursor.x,
        y: y0,
        z: cursor.z,
    });
    points.push(Point3 {
        x: cursor.x,
        y: y1,
        z: cursor.z,
    });
    if let Some(half) = pickbox_half_side(view_scale, pickbox_logical_pixels) {
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
    let mut segments: Vec<[Point3; 2]> = points
        .chunks_exact(2)
        .map(|pair| [pair[0], pair[1]])
        .collect();
    debug_assert!(segments.len() <= cad_scene::MAX_TRANSIENT_CURSOR_SEGMENTS);
    segments.truncate(cad_scene::MAX_TRANSIENT_CURSOR_SEGMENTS);
    let [r, g, b] = cad_scene::sanitize_color(CURSOR_COLOR);
    Some(cad_scene::TransientCursor {
        segments,
        color: [r, g, b, 1.0],
        draw_order: CURSOR_DRAW_ORDER,
    })
}

/// The two ends of the horizontal (or vertical) crosshair line through `cursor`.
///
/// Each arm is `percent` per cent of its own full cursor-to-edge span, so the
/// two arms are symmetric about the cursor only when the cursor is centred —
/// which is exactly what keeps `100` identical to the historical
/// full-viewport line (`min..max`, returned verbatim, not a recomputed
/// near-equal).
///
/// The percentage is authoritative on the configuration side: the schema
/// (`view.crosshair.sizePercent`) and the host's closed settings table reject
/// values outside `1..=100` instead of clamping. This pure geometry helper still
/// floors/ceils a direct caller so it can never emit a zero-length or inverted
/// line; that defence is not a configuration policy.
fn crosshair_arms(min: f64, cursor: f64, max: f64, percent: u32) -> (f64, f64) {
    let percent = percent.clamp(1, 100);
    if percent == 100 {
        return (min, max);
    }
    let factor = f64::from(percent) / 100.0;
    (
        cursor + (min - cursor) * factor,
        cursor + (max - cursor) * factor,
    )
}

/// The pickbox half-side in world units, or `None` when the scale is degenerate.
///
/// The half-side is the configured pickbox size in logical pixels (the effective
/// `view.pickbox.sizePixels`, which is also
/// [`cad_domain::TolerancePolicy::pickbox_logical_pixels`] — the value the click
/// predicate uses) times the view scale, so the box's inscribed circle is exactly
/// the region a click can hit. A missing or degenerate scale yields no pickbox
/// (never a zero/NaN one). An invalid (non-finite or non-positive) pixel size also
/// yields no box: the configuration layer refuses those, and this pure helper must
/// not fabricate a geometry from one.
fn pickbox_half_side(view_scale: Option<f64>, pickbox_logical_pixels: f64) -> Option<f64> {
    let scale = view_scale.filter(|scale| scale.is_finite() && *scale > 0.0)?;
    if !pickbox_logical_pixels.is_finite() || pickbox_logical_pixels <= 0.0 {
        return None;
    }
    let half = (pickbox_logical_pixels * scale).max(f64::MIN_POSITIVE);
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

    fn geometry(
        cursor: Option<Point3>,
        scale: Option<f64>,
        pickbox: f64,
        percent: u32,
    ) -> cad_scene::TransientCursor {
        cursor_geometry(cursor, view(), scale, pickbox, percent)
            .expect("a valid 2D cursor has transient geometry")
    }

    /// World-space `(x, y)` of the `vertex`-th flattened endpoint (the old batch
    /// vertex order: crosshair pair, pair, then the pickbox pairs).
    fn endpoint(geometry: &cad_scene::TransientCursor, vertex: usize) -> [f64; 2] {
        let point = geometry.segments[vertex / 2][vertex % 2];
        [point.x, point.y]
    }

    /// The world-space half-extent of the pickbox: half of its X span (segments
    /// 2..6 are the four sides).
    fn pickbox_half_extent(geometry: &cad_scene::TransientCursor) -> f64 {
        let xs: Vec<f64> = geometry
            .segments
            .iter()
            .skip(2)
            .flat_map(|segment| segment.iter().map(|point| point.x))
            .collect();
        (xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
            - xs.iter().cloned().fold(f64::INFINITY, f64::min))
            * 0.5
    }

    #[test]
    fn cursor_geometry_has_two_crosshair_and_four_pickbox_segments() {
        let cursor = geometry(Some(p(1.0, 2.0)), Some(0.5), 3.0, 100);
        assert_eq!(
            cursor.draw_order, CURSOR_DRAW_ORDER,
            "the transient sorts at the pointer order (preview < cursor < snap)"
        );
        let [r, g, b] = cad_scene::sanitize_color(CURSOR_COLOR);
        assert_eq!(cursor.color, [r, g, b, 1.0]);
        // Two crosshair segments + four pickbox sides, at most the bounded max.
        assert_eq!(cursor.segments.len(), 6);
        assert!(cursor.segments.len() <= cad_scene::MAX_TRANSIENT_CURSOR_SEGMENTS);
        assert!(cursor
            .segments
            .iter()
            .flatten()
            .all(|point| point.x.is_finite() && point.y.is_finite() && point.z == 0.0));
    }

    #[test]
    fn the_crosshair_spans_the_view_rectangle_through_the_cursor() {
        let cursor = geometry(Some(p(1.0, 2.0)), Some(0.5), 3.0, 100);
        // Horizontal line: x spans the view, y is the cursor's.
        assert!((endpoint(&cursor, 0)[0] - -50.0).abs() < 1e-6);
        assert!((endpoint(&cursor, 1)[0] - 50.0).abs() < 1e-6);
        assert!((endpoint(&cursor, 0)[1] - 2.0).abs() < 1e-6);
        assert!((endpoint(&cursor, 1)[1] - 2.0).abs() < 1e-6);
        // Vertical line: y spans the view, x is the cursor's.
        assert!((endpoint(&cursor, 2)[1] - -30.0).abs() < 1e-6);
        assert!((endpoint(&cursor, 3)[1] - 30.0).abs() < 1e-6);
        assert!((endpoint(&cursor, 2)[0] - 1.0).abs() < 1e-6);
        assert!((endpoint(&cursor, 3)[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_hundred_percent_crosshair_keeps_the_historical_full_view_extents() {
        // Pinned: the historical crosshair spanned `min..max` for both axes and
        // must stay bit-identical at 100 %, independent of the cursor position.
        for cursor in [p(1.0, 2.0), p(-49.0, 29.0), p(0.0, 0.0)] {
            let geometry = geometry(Some(cursor), Some(0.5), 3.0, 100);
            assert_eq!(endpoint(&geometry, 0), [-50.0, cursor.y]);
            assert_eq!(endpoint(&geometry, 1), [50.0, cursor.y]);
            assert_eq!(endpoint(&geometry, 2), [cursor.x, -30.0]);
            assert_eq!(endpoint(&geometry, 3), [cursor.x, 30.0]);
        }
    }

    #[test]
    fn a_half_crosshair_halves_each_arm_about_the_cursor() {
        let cursor = geometry(Some(p(1.0, 2.0)), Some(0.5), 3.0, 50);
        // Each arm is half of its cursor-to-edge span: -51..49 becomes
        // -25.5..24.5 around the cursor's x = 1.
        assert!((endpoint(&cursor, 0)[0] - -24.5).abs() < 1e-6);
        assert!((endpoint(&cursor, 1)[0] - 25.5).abs() < 1e-6);
        // -32..28 becomes -16..14 around the cursor's y = 2.
        assert!((endpoint(&cursor, 2)[1] - -14.0).abs() < 1e-6);
        assert!((endpoint(&cursor, 3)[1] - 16.0).abs() < 1e-6);
        // The centre is unchanged and the pickbox is untouched.
        assert!((endpoint(&cursor, 0)[1] - 2.0).abs() < 1e-6);
        assert!((endpoint(&cursor, 2)[0] - 1.0).abs() < 1e-6);
        assert_eq!(cursor.segments.len(), 6);
    }

    #[test]
    fn a_one_percent_crosshair_is_a_minimal_centred_cross() {
        let cursor = geometry(Some(p(1.0, 2.0)), Some(0.5), 3.0, 1);
        assert!((endpoint(&cursor, 0)[0] - 0.49).abs() < 1e-6);
        assert!((endpoint(&cursor, 1)[0] - 1.49).abs() < 1e-6);
        assert!((endpoint(&cursor, 2)[1] - 1.68).abs() < 1e-6);
        assert!((endpoint(&cursor, 3)[1] - 2.28).abs() < 1e-6);
        // Still a real cross through the cursor, with the unchanged pickbox.
        assert!((endpoint(&cursor, 0)[1] - 2.0).abs() < 1e-6);
        assert!((endpoint(&cursor, 2)[0] - 1.0).abs() < 1e-6);
        assert_eq!(cursor.segments.len(), 6);
    }

    #[test]
    fn an_out_of_range_percent_is_floored_to_a_visible_cross() {
        // The configuration/settings layer rejects these; this pure helper only
        // guarantees it never emits a zero-length or inverted line.
        for percent in [0, 101, u32::MAX] {
            let cursor = geometry(Some(p(1.0, 2.0)), Some(0.5), 3.0, percent);
            assert!(
                endpoint(&cursor, 0)[0] < 1.0 && endpoint(&cursor, 1)[0] > 1.0,
                "{percent}% is a real x arm"
            );
            assert!(
                endpoint(&cursor, 2)[1] < 2.0 && endpoint(&cursor, 3)[1] > 2.0,
                "{percent}% is a real y arm"
            );
        }
    }

    #[test]
    fn the_pickbox_half_side_is_the_pick_tolerance_times_the_scale() {
        let policy = cad_domain::TolerancePolicy::default();
        let scale = 0.5;
        let cursor = geometry(Some(p(1.0, 2.0)), Some(scale), 3.0, 100);
        let expected = policy.pickbox_logical_pixels * scale;
        let half = pickbox_half_extent(&cursor);
        assert!(
            (half - expected).abs() < 1e-6,
            "the pickbox half-side must be PICKBOX × scale: {half} vs {expected}"
        );
        // The box is centred on the cursor.
        let xs: Vec<f64> = cursor
            .segments
            .iter()
            .skip(2)
            .flat_map(|segment| segment.iter().map(|point| point.x))
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
    fn the_pickbox_size_is_the_configured_pixel_count_with_the_pinned_geometry() {
        let scale = 0.5;
        let three = geometry(Some(p(1.0, 2.0)), Some(scale), 3.0, 100);
        let half_three = pickbox_half_extent(&three);
        assert_eq!(
            half_three, 1.5,
            "the default pickbox half-side must stay bit-identical: {half_three}"
        );

        let six = geometry(Some(p(1.0, 2.0)), Some(scale), 6.0, 100);
        let half_six = pickbox_half_extent(&six);
        assert_eq!(half_six, 3.0, "6 px must be exactly double the 3 px box");
        assert_eq!(half_six / half_three, 2.0);

        let one = geometry(Some(p(1.0, 2.0)), Some(scale), 1.0, 100);
        let half_one = pickbox_half_extent(&one);
        assert_eq!(half_one, 0.5, "1 px at scale 0.5 is a 0.5-unit half-side");
        assert!(half_one > 0.0 && half_one < half_three);

        // The crosshair is untouched by the pickbox size: its four crosshair
        // endpoints span the view rectangle exactly at 100 %.
        for cursor in [&one, &three, &six] {
            assert_eq!(endpoint(cursor, 0), [-50.0, 2.0]);
            assert_eq!(endpoint(cursor, 1), [50.0, 2.0]);
            assert_eq!(endpoint(cursor, 2), [1.0, -30.0]);
            assert_eq!(endpoint(cursor, 3), [1.0, 30.0]);
        }
    }

    #[test]
    fn an_invalid_pickbox_size_omits_the_box_instead_of_fabricating_one() {
        for pixels in [0.0, -3.0, f64::NAN, f64::INFINITY] {
            let cursor = geometry(Some(p(1.0, 2.0)), Some(0.5), pixels, 100);
            assert_eq!(
                cursor.segments.len(),
                2,
                "no pickbox for an invalid size {pixels}"
            );
        }
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
            let cursor = geometry(Some(p(0.0, 0.0)), scale, 3.0, 100);
            assert_eq!(
                cursor.segments.len(),
                2,
                "no pickbox for a degenerate scale {scale:?}"
            );
        }
    }

    #[test]
    fn cursor_geometry_is_absent_without_a_cursor_or_a_valid_view() {
        assert!(cursor_geometry(None, view(), Some(0.5), 3.0, 100).is_none());
        // A 3D view has no planar view rectangle.
        assert!(cursor_geometry(Some(p(0.0, 0.0)), None, Some(0.5), 3.0, 100).is_none());
        // Degenerate rectangles are not a valid view.
        for bounds in [
            Some((p(1.0, 0.0), p(1.0, 0.0))),
            Some((p(5.0, 0.0), p(0.0, 1.0))),
            Some((p(0.0, 5.0), p(1.0, 0.0))),
        ] {
            assert!(cursor_geometry(Some(p(0.0, 0.0)), bounds, Some(0.5), 3.0, 100).is_none());
        }
        // A non-finite cursor or bound is refused.
        assert!(cursor_geometry(
            Some(Point3 {
                x: f64::NAN,
                y: 0.0,
                z: 0.0
            }),
            view(),
            Some(0.5),
            3.0,
            100
        )
        .is_none());
        assert!(cursor_geometry(
            Some(p(0.0, 0.0)),
            Some((p(f64::NEG_INFINITY, 0.0), p(1.0, 1.0))),
            Some(0.5),
            3.0,
            100
        )
        .is_none());
    }

    /// Deterministic LCG fuzz: whatever the inputs, the geometry is bounded by
    /// `MAX_TRANSIENT_CURSOR_SEGMENTS` and every emitted coordinate is finite.
    #[test]
    fn cursor_geometry_is_always_bounded_and_finite() {
        let mut state = 0x9E37_79B9u32;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state
        };
        for _ in 0..2000 {
            let coord = |next: &mut dyn FnMut() -> u32| (f64::from(next() % 2000) - 1000.0) / 10.0;
            let cursor = p(coord(&mut next), coord(&mut next));
            let (x0, y0) = (coord(&mut next), coord(&mut next));
            let (mut x1, mut y1) = (coord(&mut next), coord(&mut next));
            // Deliberately let some rectangles be degenerate.
            if next() % 4 != 0 {
                x1 = x1.max(x0 + 0.1);
                y1 = y1.max(y0 + 0.1);
            }
            let scale = match next() % 6 {
                0 => None,
                1 => Some(0.0),
                2 => Some(-1.0),
                3 => Some(f64::NAN),
                _ => Some(f64::from(next() % 100) / 10.0 + 0.01),
            };
            let pickbox = match next() % 6 {
                0 => 0.0,
                1 => -3.0,
                2 => f64::NAN,
                _ => f64::from(next() % 500) / 20.0,
            };
            let percent = match next() % 4 {
                0 => 0,
                1 => 101,
                2 => u32::MAX,
                _ => next() % 101,
            };
            if let Some(cursor) = cursor_geometry(
                Some(cursor),
                Some((p(x0, y0), p(x1, y1))),
                scale,
                pickbox,
                percent,
            ) {
                assert!(!cursor.segments.is_empty());
                assert!(
                    cursor.segments.len() <= cad_scene::MAX_TRANSIENT_CURSOR_SEGMENTS,
                    "bounded segment count"
                );
                assert!(
                    matches!(cursor.segments.len(), 2 | 6),
                    "crosshair (2) or crosshair + pickbox (6), got {}",
                    cursor.segments.len()
                );
                for [a, b] in &cursor.segments {
                    for point in [a, b] {
                        assert!(
                            point.x.is_finite() && point.y.is_finite() && point.z.is_finite(),
                            "finite geometry"
                        );
                    }
                }
                assert_eq!(cursor.draw_order, CURSOR_DRAW_ORDER);
            }
        }
    }
}
