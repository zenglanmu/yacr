//! Visible POINT markers drawn through the transient overlay
//! (ADR 0015 decision E).
//!
//! A `SemanticGeometry::Point` has no base-scene geometry: the chord tessellator
//! would degenerate it to a zero-length `[p, p]` segment, which is invisible.
//! The base-scene provider therefore skips it (`representation.point_overlay`)
//! and it is drawn here instead, as a screen-relative glyph in overlay order —
//! the same transient pipeline the snap hints and tool previews use.
//!
//! The glyph is AutoCAD's drawing-global **point display style** (`$PDMODE` /
//! `$PDSIZE`, carried on [`cad_db::DrawingDatabase`] and surfaced as
//! [`PointDisplayStyle`]): `$PDMODE` chooses the base shape and `$PDSIZE` the
//! size. This is **display only** — it never touches
//! `SemanticGeometry::Point`, whose DXF group 50 axis angle stays unmodelled
//! because adding a field to the domain geometry for a display nicety would
//! ripple through every crate — and there is no `SETVAR` equivalent, so the
//! style can only arrive with the imported drawing.
//!
//! The topology is `Lines` (`cad_scene::RenderTopology` has no `Points`
//! variant), which is why a dot is an approximation. The `$PDMODE` surrounding
//! shapes (`32` circle, `64` square, `96` both) *are* drawn — as explicit
//! segments in the same batch, see [`super::point_surrounds`] — and a circle is
//! therefore a closed chord ring, not a conic. The dot approximation is reported,
//! never dropped silently.

use super::point_surrounds::push_point_surrounds;
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
/// the box-selection band ([`BOX_SELECT_DRAW_ORDER`], `1_750_000`), the tool
/// preview ([`PREVIEW_DRAW_ORDER`], `2_000_000`), the pointer cursor
/// ([`CURSOR_DRAW_ORDER`], `2_500_000`) and the object-snap hints
/// ([`SNAP_HINT_DRAW_ORDER`], `3_000_000`) — so the live feedback the user is
/// acting on is never occluded by a marker.
///
/// **Deliberate deviation (ADR 0015 decision E):** markers draw in overlay order,
/// i.e. always on top of base-scene entities, so a POINT is never hidden behind
/// other geometry the way a base-scene entity could be.
pub const POINT_MARKER_DRAW_ORDER: i64 = 850_000;

/// Marker colour: a pale blue-white, distinct from the amber object-snap hints
/// and both preview colours.
pub const POINT_MARKER_COLOR: [f32; 3] = [0.80, 0.90, 1.0];

/// Half-extent of a POINT marker as a fraction of the viewport height, used when
/// `$PDSIZE` is `0` (the DWG default).
///
/// The full marker therefore spans 5% of the viewport height, which is AutoCAD's
/// own meaning of `PDSIZE = 0` and the historical behaviour of this overlay.
/// `$PDSIZE > 0` is an absolute drawing-unit size and `$PDSIZE < 0` a percentage
/// of the viewport height; see [`point_marker_overlay`].
pub const POINT_MARKER_HALF_EXTENT_FRACTION: f64 = 0.025;

/// Largest `$PDSIZE` magnitude the overlay will use, in drawing units (positive)
/// or percent of viewport height (negative).
///
/// A larger magnitude cannot be a display size — a marker a million units (or a
/// million percent of the screen) across is unrenderable — so it is treated as
/// "unset", i.e. the `$PDSIZE = 0` behaviour, exactly like
/// [`cad_db::sanitize_linetype_scale`] degrades a broken scale to its identity.
/// The sanitisation is applied by [`sanitize_point_display_size`] at the consume
/// point and reported once, never silently.
pub const POINT_DISPLAY_SIZE_LIMIT: f64 = 1.0e6;

/// `$PDMODE` bits that request a surrounding shape around the base marker
/// (`32` circle, `64` square, `96` both).
///
/// The shapes are implemented ([`super::point_surrounds`]); this mask is what
/// decides which bits are **valid** mode bits. Anything outside
/// `0x1F | this mask` is not part of the AutoCAD point display mode and is
/// reported as `overlay.point-mode-unsupported`.
const PDMODE_SURROUNDING_SHAPE_BITS: i16 = 32 | 64;

/// Largest base arm table ([`PLUS_ARMS`] / [`CROSS_ARMS`]: two segments).
///
/// Used only for the structural vertex bound of a capped batch; the emitted
/// count still comes from the actual plan.
const POINT_MARKER_MAX_BASE_ARMS: usize = 2;

/// Structural upper bound on the `LineList` segments one marker can emit: the
/// widest base glyph plus both surrounding shapes.
///
/// The circle is a fixed [`super::POINT_SURROUND_CIRCLE_SEGMENTS`]-chord ring and
/// the square [`super::POINT_SURROUND_SQUARE_SEGMENTS`] sides, so this is a real
/// bound, not an estimate; nothing here scales with zoom.
pub const POINT_MARKER_MAX_SEGMENTS_PER_MARKER: usize =
    POINT_MARKER_MAX_BASE_ARMS + POINT_SURROUND_SQUARE_SEGMENTS + POINT_SURROUND_CIRCLE_SEGMENTS;

/// Structural upper bound on the vertices of one capped marker batch: two
/// vertices per segment of the widest marker, [`POINT_MARKER_LIMIT`] times over.
pub const POINT_MARKER_MAX_VERTICES: usize =
    POINT_MARKER_LIMIT * POINT_MARKER_MAX_SEGMENTS_PER_MARKER * 2;

/// One marker segment, in units of the marker half-extent: `(dx0, dy0, dx1, dy1)`.
type MarkerArm = (f64, f64, f64, f64);

/// `$PDMODE` base `2`: an axis-aligned plus (`+`).
const PLUS_ARMS: [MarkerArm; 2] = [(-1.0, 0.0, 1.0, 0.0), (0.0, -1.0, 0.0, 1.0)];
/// `$PDMODE` base `3`: a diagonal cross (`x`).
const CROSS_ARMS: [MarkerArm; 2] = [(-1.0, -1.0, 1.0, 1.0), (-1.0, 1.0, 1.0, -1.0)];
/// `$PDMODE` base `4`: a vertical tick (`|`).
const TICK_ARMS: [MarkerArm; 1] = [(0.0, -1.0, 0.0, 1.0)];
/// `$PDMODE` base `1` and every unexpressible mode: no marker at all.
const NO_ARMS: [MarkerArm; 0] = [];

/// The marker geometry decision for one `$PDMODE` value.
struct MarkerPlan {
    /// Segments to emit per point, in units of the half-extent. Empty = no marker.
    arms: &'static [MarkerArm],
    /// Surrounding shapes requested by bits `32`/`64`, drawn around the arms.
    surrounds: PointSurrounds,
    /// `(code, message)` per unsupported/approximated aspect, in report order.
    findings: Vec<(&'static str, String)>,
}

/// Decide what one `$PDMODE` value draws, and what has to be reported about it.
///
/// Supported base glyphs are `0` dot, `1` none, `2` plus, `3` cross and `4`
/// vertical tick. One of those is **not literal**: `0` (dot) cannot be rasterised
/// by a `Lines`-only topology (there is no `Points` topology, and adding one is
/// out of scope), so it is drawn as the smallest plus glyph this path can express
/// and reported as `Partial` — it is **not** a dot.
///
/// Bits `32` (circle) and `64` (square) add the surrounding shapes as explicit
/// segments in the same batch ([`super::point_surrounds`]); `96` is both. They
/// are fully implemented, so they raise no diagnostic of their own.
///
/// Any value the project cannot express as a whole (a negative mode, a base mode
/// above `4`, a bit outside `0..=4 | 32 | 64`) is reported as
/// `overlay.point-mode-unsupported`; whatever part *is* expressible is drawn and
/// nothing is invented for the rest.
///
/// The base decoding is `mode & 0x1F` and is deliberately **unchanged** by the
/// surrounding shapes, so the additive rule AutoCAD documents for `35`/`66`
/// holds: `32 | 3` is a circle around the cross. Note the arithmetic
/// consequence, which is pinned by tests rather than special-cased: bare `32`
/// decodes as base `0` (the dot approximation, plus a circle) and bare `33` as
/// base `1` (nothing, plus a circle). Whether AutoCAD itself shows a centre dot
/// for a bare `32` is **not verified**; inventing an exception would make the
/// decode depend on which shape bit is set, which is worse than one rule.
fn marker_plan(mode: i16) -> MarkerPlan {
    let mut findings: Vec<(&'static str, String)> = Vec::new();
    if mode < 0 {
        findings.push((
            "overlay.point-mode-unsupported",
            format!(
                "PDMODE {mode} is negative, which is not a point display mode; \
                 no POINT marker is drawn for it"
            ),
        ));
        return MarkerPlan {
            arms: &NO_ARMS,
            surrounds: PointSurrounds::NONE,
            findings,
        };
    }
    // The low five bits select the base glyph; `32`/`64` add surrounding shapes;
    // anything else is not part of the AutoCAD point display mode.
    let base = mode & 0x1F;
    let unknown_bits = mode & !(0x1F | PDMODE_SURROUNDING_SHAPE_BITS);
    let arms: &'static [MarkerArm] = match base {
        // A dot cannot be a real dot in a Lines-only topology. Two
        // approximations were possible: a plus at the configured size, or a
        // shrunken plus. The configured size is chosen deliberately — a dot is
        // the DWG default `$PDMODE`, so shrinking it would silently change how
        // every unstyled drawing's points look at `$PDSIZE = 0`, while a plus at
        // the configured size is exactly the marker this overlay drew before the
        // point style existed. The glyph is still reported as an approximation.
        0 => &PLUS_ARMS,
        1 => &NO_ARMS,
        2 => &PLUS_ARMS,
        3 => &CROSS_ARMS,
        4 => &TICK_ARMS,
        _ => &NO_ARMS,
    };
    let surrounds = PointSurrounds {
        circle: mode & 32 != 0,
        square: mode & 64 != 0,
    };
    if base == 0 {
        findings.push((
            "overlay.point-mode-dot",
            format!(
                "PDMODE {mode} selects the dot base shape (0), which the Lines-only \
                 marker topology cannot draw; the point is marked with the smallest plus \
                 glyph this topology can draw instead (an approximation, not a dot), \
                 sized by PDSIZE"
            ),
        ));
    }
    let mut refused: Vec<String> = Vec::new();
    if base > 4 {
        refused.push(format!(
            "base mode {base} is not one of the implemented point styles \
             (0 dot, 1 none, 2 plus, 3 cross, 4 vertical tick)"
        ));
    }
    if unknown_bits != 0 {
        refused.push(format!(
            "bit(s) {unknown_bits} are not part of the AutoCAD point display mode"
        ));
    }
    if !refused.is_empty() {
        let tail = if arms.is_empty() && surrounds.is_empty() {
            "no POINT marker is drawn for this mode"
        } else {
            "only the expressible marker parts are drawn and no substitute shape is invented"
        };
        findings.push((
            "overlay.point-mode-unsupported",
            format!("PDMODE {mode}: {}; {tail}", refused.join("; ")),
        ));
    }
    MarkerPlan {
        arms,
        surrounds,
        findings,
    }
}

/// The single sanitisation rule for `$PDSIZE`.
///
/// A non-finite or absurd magnitude ([`POINT_DISPLAY_SIZE_LIMIT`]) is meaningless
/// as a display size, so it degrades to `0.0` — the "viewport-relative" default —
/// exactly like [`cad_db::sanitize_linetype_scale`] degrades a broken linetype
/// scale to its identity. Everything else is kept as stored, sign included, so
/// the sign (`> 0` absolute, `< 0` percent) is interpreted by
/// [`marker_half_extent`] and nowhere else.
///
/// The database stores the raw value; this function is the **consume point**, and
/// [`point_marker_overlay`] reports a fallback once so it is never silent.
pub fn sanitize_point_display_size(size: f64) -> f64 {
    if size.is_finite() && size.abs() <= POINT_DISPLAY_SIZE_LIMIT {
        size
    } else {
        0.0
    }
}

/// The marker half-extent in drawing units for a sanitised `$PDSIZE` and a 2D
/// viewport height. `None` when no marker can be sized from it.
///
/// * `size > 0` — the absolute drawing-unit size of the marker, so `half = size / 2`.
/// * `size == 0` — viewport-relative: [`POINT_MARKER_HALF_EXTENT_FRACTION`] of the
///   viewport height (the historical behaviour).
/// * `size < 0` — the magnitude is a percentage of the viewport height.
fn marker_half_extent(size: f64, viewport_height: f64) -> Option<f64> {
    let half = if size > 0.0 {
        size / 2.0
    } else if size < 0.0 {
        viewport_height * (size.abs() / 100.0)
    } else {
        viewport_height * POINT_MARKER_HALF_EXTENT_FRACTION
    };
    (half.is_finite() && half > 0.0).then_some(half)
}

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
///
/// **Scope limitation (by design):** the scan walks top-level model/layout
/// entities only, so a POINT nested inside a block definition is **not** marked.
/// This is acceptable for now because such a point was equally invisible before
/// this change: insert expansion went through the same chord fallback, which
/// degenerated the point to a zero-length `[p, p]` segment. Expanding block
/// members into the marker scan is a separate, larger change and is deliberately
/// out of scope here. Pinned by
/// [`tests::a_block_resident_point_is_not_marked_by_design`].
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
/// The glyph is `$PDMODE`'s base shape and the size is `$PDSIZE`
/// ([`marker_plan`], [`marker_half_extent`]): `$PDSIZE = 0` keeps the historical
/// viewport-relative size (2.5% of the viewport height as a half-extent), a
/// positive size is an absolute drawing-unit size and a negative one a percentage
/// of the viewport height. A camera-only change therefore rescales the markers
/// without rebuilding the base scene.
///
/// In a 3D view `viewport_bounds` is `None`, so **no** marker is emitted (ADR 0015
/// decision E: 3D point markers are NOT IMPLEMENTED). This stays a deliberate
/// restriction even where `$PDSIZE > 0` would be definable without a viewport
/// height: the 3D restriction is about the view, not about the style, and a marker
/// that only appears for some drawings would read as a bug. A 3D view emits no
/// style diagnostic either, for the same reason — nothing is dropped, because
/// nothing was going to be drawn.
///
/// Every unsupported or approximated aspect of the style is reported **once** per
/// call (`overlay.point-mode-dot`, `overlay.point-mode-unsupported`,
/// `overlay.point-size-sanitized`) together with the cap diagnostic, and the
/// overlay is `Partial` when any of them applies — never a silent drop, never one
/// diagnostic per point. The implemented `$PDMODE` surrounding shapes (`32`
/// circle, `64` square) are geometry, not diagnostics.
///
/// Cost is `O(visible points)`: a single vertex buffer and a single batch, with
/// no per-point allocation. The per-marker segment count is fixed (base arms plus
/// [`POINT_SURROUND_SQUARE_SEGMENTS`]/[`POINT_SURROUND_CIRCLE_SEGMENTS`], see
/// [`PointSurrounds::segment_count`]), bounded by
/// [`POINT_MARKER_MAX_SEGMENTS_PER_MARKER`], and never derived from zoom.
pub fn point_marker_overlay(
    points: &[Point3],
    viewport_bounds: Option<(Point3, Point3)>,
    style: PointDisplayStyle,
    document: DocumentId,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    if points.is_empty() {
        return out;
    }
    // 3D view: no screen-relative size domain -> no markers (deliberate).
    let Some((min, max)) = viewport_bounds else {
        return out;
    };
    let mut findings: Vec<(&'static str, String)> = Vec::new();
    let size = sanitize_point_display_size(style.size);
    if size.to_bits() != style.size.to_bits() {
        findings.push((
            "overlay.point-size-sanitized",
            format!(
                "PDSIZE {} is not a usable marker size (non-finite or beyond the \
                 {POINT_DISPLAY_SIZE_LIMIT} limit); the PDSIZE 0 viewport-relative size is used instead",
                style.size
            ),
        ));
    }
    let plan = marker_plan(style.mode);
    findings.extend(plan.findings);

    if let Some(half) = marker_half_extent(size, (max.y - min.y).abs()) {
        let drawn = if points.len() > POINT_MARKER_LIMIT {
            &points[..POINT_MARKER_LIMIT]
        } else {
            points
        };
        // One glyph per point: `plan.arms` explicit `LineList` segment pairs, plus
        // the requested surrounding shapes, in the same batch.
        let segments_per_marker = plan.arms.len() + plan.surrounds.segment_count();
        let mut vertices: Vec<Point3> = Vec::with_capacity(drawn.len() * segments_per_marker * 2);
        for point in drawn {
            for (dx0, dy0, dx1, dy1) in plan.arms {
                vertices.push(Point3 {
                    x: point.x + dx0 * half,
                    y: point.y + dy0 * half,
                    z: point.z,
                });
                vertices.push(Point3 {
                    x: point.x + dx1 * half,
                    y: point.y + dy1 * half,
                    z: point.z,
                });
            }
            push_point_surrounds(&mut vertices, *point, half, plan.surrounds);
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
            findings.push((
                "overlay.point-marker-limit",
                format!(
                    "{} visible POINT entities exceed the {POINT_MARKER_LIMIT}-marker cap; \
                     drawing the first {POINT_MARKER_LIMIT}",
                    points.len()
                ),
            ));
        }
    }
    let mut messages: Vec<String> = Vec::with_capacity(findings.len());
    for (code, message) in findings {
        messages.push(message.clone());
        out.diagnostics.push(Diagnostic {
            object: None,
            code: code.into(),
            message,
        });
    }
    if !messages.is_empty() {
        out.completeness = Completeness::Partial(messages);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_scene::CadSceneController;
    use crate::viewer_config::OverlayVisibility;
    use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
    use cad_domain::{BlockId, DatabaseId, EntityId, LayerId, ObjectId, Revision, SpaceId};
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

    /// `$PDMODE = 2` (plus) with the default viewport-relative size: the shape
    /// that pins the historical `PDSIZE = 0` formula without the mode-0 dot
    /// approximation's diagnostic in the way.
    const PLUS_STYLE: PointDisplayStyle = PointDisplayStyle { mode: 2, size: 0.0 };

    #[test]
    fn point_marker_overlay_draws_one_batch_and_scales_with_the_viewport_height() {
        let points = [p(0.0, 0.0)];
        let small = point_marker_overlay(&points, viewport(100.0), PLUS_STYLE, DocumentId(1));
        let large = point_marker_overlay(&points, viewport(400.0), PLUS_STYLE, DocumentId(1));
        assert_eq!(small.drawn(), 1, "all points share one bounded batch");
        assert_eq!(large.drawn(), 1);
        assert_eq!(small.batches[0].topology, RenderTopology::Lines);
        assert_eq!(small.batches[0].draw_order, POINT_MARKER_DRAW_ORDER);
        assert_eq!(small.batches[0].color, POINT_MARKER_COLOR);
        assert!(!small.batches[0].color_unresolved);
        // One plus marker => two segments => four vertices.
        assert_eq!(small.batches[0].vertices.len(), 4);
        // 5% of the viewport height as a full cross: half-extent = 0.025 * height.
        let small_extent = batch_extent(&small.batches[0]);
        let large_extent = batch_extent(&large.batches[0]);
        assert!((small_extent - 2.5).abs() < 1e-4, "{small_extent}");
        assert!(
            (large_extent / small_extent - 4.0).abs() < 1e-3,
            "a 4x taller viewport must give a 4x larger marker: {large_extent} vs {small_extent}"
        );
        assert_eq!(
            small.diagnostics.len(),
            0,
            "a supported base mode with the default size reports nothing"
        );
        assert_eq!(small.completeness, Completeness::Complete);
    }

    #[test]
    fn point_marker_overlay_stops_at_the_cap_with_exactly_one_diagnostic() {
        let at_cap: Vec<Point3> = (0..POINT_MARKER_LIMIT as u64)
            .map(|i| p(i as f64, 0.0))
            .collect();
        let overlay = point_marker_overlay(&at_cap, viewport(100.0), PLUS_STYLE, DocumentId(1));
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
        let overlay = point_marker_overlay(&over_cap, viewport(100.0), PLUS_STYLE, DocumentId(1));
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

    /// The cap-sized build is **structurally bounded**, and only sanity-checked
    /// for wall-clock so the docs' µs claim is not left unsupported.
    ///
    /// Structural bound: **one** batch, exactly four vertices per drawn marker
    /// (two `LineList` segment pairs for the fixed plus), and no cap diagnostic
    /// at the cap. No per-point allocation beyond the single capacity-reserved
    /// vertex vector is enforced by construction in [`point_marker_overlay`]
    /// (`Vec::with_capacity(drawn.len() * 4)` plus one `line_batch`), and pinned
    /// here by the exact vertex count.
    ///
    /// The wall-clock ceiling is a **sanity bound, not a performance target**:
    /// the build is tens of microseconds, so 250 ms is roughly four orders of
    /// magnitude of headroom and cannot flake on scheduler noise. It only trips
    /// on an order-of-magnitude regression (for example accidental per-point
    /// work). The `71 µs` figure in the docs is an estimate, not a measurement;
    /// this test does not measure it.
    #[test]
    fn point_marker_overlay_cap_build_is_structurally_bounded() {
        let at_cap: Vec<Point3> = (0..POINT_MARKER_LIMIT as u64)
            .map(|i| p(i as f64, 0.0))
            .collect();
        // Warm up first so the timed run does not pay one-off allocation growth.
        let _ = point_marker_overlay(&at_cap, viewport(100.0), PLUS_STYLE, DocumentId(1));
        let started = std::time::Instant::now();
        let overlay = point_marker_overlay(&at_cap, viewport(100.0), PLUS_STYLE, DocumentId(1));
        let elapsed = started.elapsed();

        assert_eq!(overlay.drawn(), 1, "the whole cap is one bounded batch");
        assert_eq!(
            overlay.batches[0].vertices.len(),
            POINT_MARKER_LIMIT * 4,
            "exactly four vertices per drawn marker, no more"
        );
        assert!(
            overlay.diagnostics.is_empty(),
            "no diagnostic at exactly the cap"
        );
        assert_eq!(overlay.completeness, Completeness::Complete);

        const SANITY_CEILING: std::time::Duration = std::time::Duration::from_millis(250);
        assert!(
            elapsed < SANITY_CEILING,
            "cap-sized build took {elapsed:?}, over the {SANITY_CEILING:?} sanity ceiling; \
             this guards against an order-of-magnitude regression, not a performance target"
        );
    }

    #[test]
    fn point_marker_overlay_is_not_drawn_in_a_3d_view() {
        let points = [p(1.0, 2.0)];
        // `viewport_bounds == None` is the 3D case: markers are deliberately not
        // drawn, whatever the style asks for (see `tests/point_styles.rs` for the
        // `PDSIZE > 0` case).
        let overlay =
            point_marker_overlay(&points, None, PointDisplayStyle::default(), DocumentId(1));
        assert!(overlay.is_empty());
        assert!(overlay.diagnostics.is_empty());
        assert_eq!(overlay.completeness, Completeness::Complete);
    }

    #[test]
    fn point_marker_overlay_emits_nothing_for_an_empty_set_or_a_degenerate_viewport() {
        assert!(point_marker_overlay(&[], viewport(100.0), PLUS_STYLE, DocumentId(1)).is_empty());
        // A zero-height viewport has no screen-relative size domain for the
        // `PDSIZE = 0` (default) size.
        assert!(
            point_marker_overlay(&[p(0.0, 0.0)], viewport(0.0), PLUS_STYLE, DocumentId(1))
                .is_empty()
        );
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

    /// A drawing with a top-level model POINT (entity 1) and a block definition
    /// (block 7) whose sole member is a POINT at `(9, 9)` (entity 2).
    fn drawing_with_a_block_resident_point() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(point_entity(1, 0, p(1.0, 1.0))).unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(7),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
        let mut member = point_entity(2, 0, p(9.0, 9.0));
        member.space = SpaceId::Block(BlockId(7));
        b.insert_entity(member).unwrap();
        b.finish().unwrap()
    }

    /// The marker scan walks top-level model/layout entities only, so a POINT
    /// nested inside a block definition is **not** marked. This is pinned as
    /// **by design**, not a bug: such a point was equally invisible before this
    /// change (insert expansion used the same chord fallback, degenerating it to
    /// a zero-length `[p, p]`). See the `visible_point_marker_positions` doc
    /// comment; expanding block members is a separate change.
    #[test]
    fn a_block_resident_point_is_not_marked_by_design() {
        let db = drawing_with_a_block_resident_point();
        // The block really does own a POINT member, so the test is not vacuous.
        let members = db.block_entities(BlockId(7));
        assert_eq!(members.len(), 1, "the block owns one member entity");
        assert!(matches!(members[0].geometry, SemanticGeometry::Point(_)));

        // Only the top-level model POINT is in scope.
        assert_eq!(
            visible_point_marker_positions(&db, &LayerOverrideSet::new(), SpaceSelection::Model),
            vec![p(1.0, 1.0)],
            "only the top-level model POINT is marked; the block-resident one is not"
        );

        // End-to-end through the scene assembly: one marker batch, one cross —
        // the block-resident POINT contributes nothing.
        let db = Arc::new(db);
        let layers = LayerOverrideSet::new();
        let mut overlays = overlays_without_reference();
        overlays.viewport_bounds = viewport(100.0);
        let mut controller = CadSceneController::default();
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
        let batches = marker_batches(scene);
        assert_eq!(
            batches.len(),
            1,
            "one marker batch for the one in-scope point"
        );
        assert_eq!(
            batches[0].vertices.len(),
            4,
            "the block-resident POINT adds no cross"
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
