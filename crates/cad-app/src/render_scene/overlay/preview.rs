use super::*;

/// Build the tool-preview overlay for an in-progress measurement.
///
/// The preview is pure: it only reads the captured points and the live cursor.
/// It never commits anything to the database.
///
/// `snap_hints` gates the axis-aligned point crosses only. They are the current
/// cursor/snap markers, so a host that turned `view.overlays.snapHints` off
/// still gets the rubber-band chain, just without the crosses.
pub fn preview_overlay(
    measurement: Option<&MeasurementPreview>,
    document: DocumentId,
    snap_hints: bool,
    options: &PreviewOptions,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    if let Some(preview) = measurement {
        out.merge(measurement_preview_overlay(
            preview, document, snap_hints, options,
        ));
    }
    out
}

/// Build the live draw/edit preview: a rubber-band chain for line/move/trim and
/// a circle for the circle tool.
///
/// Presentation only; the committed operation is the exact intent from the draw
/// tool's own state machine and never this overlay.
pub fn draw_preview_overlay(
    preview: &DrawPreview,
    document: DocumentId,
    snap_hints: bool,
    options: &PreviewOptions,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    let mut order = options.draw_order;
    let geometry: Option<Vec<Point3>> = match preview.kind {
        DrawToolKind::Circle => circle_points(&preview.points, preview.cursor),
        DrawToolKind::Arc => arc_points(&preview.points, preview.cursor).or_else(|| {
            let chain = preview_chain(&preview.points, preview.cursor);
            (chain.len() >= 2).then_some(chain)
        }),
        // RECTANG: the live rectangle outline between the captured first corner
        // and the cursor (the second corner). Uses the same `DrawPreview` points +
        // cursor plumbing as the chain and circle previews, so no new overlay
        // channel is introduced; it falls back to the rubber-band chain while the
        // span is degenerate.
        DrawToolKind::Rectang => rectangle_points(&preview.points, preview.cursor).or_else(|| {
            let chain = preview_chain(&preview.points, preview.cursor);
            (chain.len() >= 2).then_some(chain)
        }),
        // ELLIPSE: a bounded chord tessellation of the exact ellipse the commit
        // stores, between the captured axis-endpoint picks and the live cursor.
        // The preview is an **approximation** (the stored entity is exact); it
        // falls back to the rubber-band chain while the capture cannot define an
        // ellipse (fewer than two picks, coincident endpoints, or the third pick
        // on the first axis), never a fabricated full ellipse.
        DrawToolKind::Ellipse => ellipse_points(&preview.points, preview.cursor).or_else(|| {
            let chain = preview_chain(&preview.points, preview.cursor);
            (chain.len() >= 2).then_some(chain)
        }),
        // POLYGON: the closed `n`-vertex outline (n = the current session side
        // count carried on the preview) between the captured centre and the live
        // cursor as the radius point. It falls back to the rubber-band chain
        // while the radius is degenerate or the side count is unknown, so a
        // nonsense ring is never fabricated.
        DrawToolKind::Polygon => {
            polygon_points(&preview.points, preview.cursor, preview.polygon_sides).or_else(|| {
                let chain = preview_chain(&preview.points, preview.cursor);
                (chain.len() >= 2).then_some(chain)
            })
        }
        // SPLINE: a bounded chord approximation of the exact interpolated curve
        // the commit stores, through the captured fit points plus the live cursor.
        // It falls back to the rubber-band chain while fewer than four fit points
        // are known or the interpolation refuses (a degenerate chord, a
        // near-coincident cursor, a non-coplanar set), never a fabricated curve.
        DrawToolKind::Spline => spline_points(&preview.points, preview.cursor).or_else(|| {
            let chain = preview_chain(&preview.points, preview.cursor);
            (chain.len() >= 2).then_some(chain)
        }),
        // POINT: a single placement; the live preview is the existing
        // cursor/chain (no new overlay channel). TEXT captures its single
        // insertion point the same way (the typed text is a prompt, not a
        // geometry preview), so it also uses the cursor/chain.
        DrawToolKind::Point
        | DrawToolKind::Text
        // MTEXT captures the two opposite corners of its wrap rectangle; its live
        // preview is the existing cursor/chain rubber band (the typed text is a
        // prompt, and the width/layout is derived at commit, so no fabricated
        // wrapped text box is drawn here).
        | DrawToolKind::MText
        // MLEADER captures the arrow position and the landing point; its live
        // preview is the existing cursor/chain (the typed text is a prompt, not a
        // geometry preview, and the dogleg is derived at commit from the style
        // table, so no style-dependent outline is fabricated here).
        | DrawToolKind::MLeader
        // TEXTEDIT's step is an entity pick; its only live affordance is the
        // host's capture tint plus the cursor/chain, never a fabricated geometry.
        | DrawToolKind::TextEdit
        | DrawToolKind::Line
        | DrawToolKind::Polyline
        | DrawToolKind::Move
        | DrawToolKind::Trim
        | DrawToolKind::Copy
        | DrawToolKind::Extend
        | DrawToolKind::Rotate
        | DrawToolKind::Scale
        | DrawToolKind::Mirror
        | DrawToolKind::Offset
        // CHAMFER/FILLET are entity picks with two raw pick points; the live
        // preview is the existing cursor/chain (no new overlay channel).
        | DrawToolKind::Chamfer
        | DrawToolKind::Fillet
        // ARRAYPOLAR captures one point (the centre); the live cursor is the
        // existing chain. ARRAYRECT captures no points at all, so `preview_chain`
        // yields nothing for it (no fabricated preview) — the array result depends
        // on the session parameters plus the selection, not on the cursor. HATCH
        // has the same zero-point pre-selection shape (ADR 0018 decision D), so it
        // also yields no chain.
        | DrawToolKind::ArrayRect
        | DrawToolKind::ArrayPolar
        | DrawToolKind::Hatch
        // ALIGN captures four free points (two source/destination pairs); STRETCH
        // captures four free points (two window corners plus a base and a
        // displacement point). Both use the existing cursor/chain preview (no new
        // overlay channel).
        | DrawToolKind::Align
        | DrawToolKind::Stretch
        // DIMLINEAR/DIMALIGNED capture three free points (two origins plus the
        // dimension-line location); the live preview is the existing cursor/chain
        // (no new overlay channel — the visible dimension affordance is the
        // command's, not a preview overlay).
        | DrawToolKind::DimLinear
        | DrawToolKind::DimAligned
        // DIMRADIUS/DIMDIAMETER capture an entity pick plus a location point;
        // DIMCENTER captures one entity pick. Their live preview is the existing
        // cursor/chain (no new overlay channel; the picked entity is shown by the
        // host's capture tint, and the dimension affordance is the command's).
        | DrawToolKind::DimRadius
        | DrawToolKind::DimDiameter
        | DrawToolKind::DimCenter
        // DIMANGULAR captures two line picks plus the arc location; DIMORDINATE
        // captures the feature point and the leader endpoint. Both use the
        // existing cursor/chain preview (no new overlay channel; the picked lines
        // are shown by the host's capture tint).
        | DrawToolKind::DimAngular
        | DrawToolKind::DimOrdinate
        // DIMBASELINE/DIMCONTINUE pick a base dimension and then capture an
        // open-ended run of free points; the live preview is the existing
        // cursor/chain (the stacking normal and the chain geometry belong to the
        // command, not to a preview overlay).
        | DrawToolKind::DimBaseline
        | DrawToolKind::DimContinue => {
            let chain = preview_chain(&preview.points, preview.cursor);
            (chain.len() >= 2).then_some(chain)
        }
    };
    if let Some(points) = geometry {
        if let Some(batch) =
            line_batch(&points, DRAW_PREVIEW_COLOR, document, order, "draw.preview")
        {
            out.batches.push(batch);
            order += 1;
        }
    }
    if snap_hints {
        if let Some(batch) = marker_batch(
            &preview.points,
            options.marker_size,
            DRAW_PREVIEW_COLOR,
            document,
            order,
            "draw.preview",
        ) {
            out.batches.push(batch);
        }
    }
    out
}

/// The arc for an arc-tool preview: once two points are captured the live cursor
/// supplies the third (on-arc) point; a third captured point replaces it.
///
/// Returns `None` (so the caller falls back to the rubber-band chain) while fewer
/// than three points are known or when the three are collinear.
fn arc_points(points: &[Point3], cursor: Option<Point3>) -> Option<Vec<Point3>> {
    let (start, through, end) = match points {
        [a, b, c, ..] => (*a, *b, *c),
        [a, b] => (*a, *b, cursor?),
        _ => return None,
    };
    let parameters = crate::draw_tool::arc_parameters(start, through, end).ok()?;
    let sweep = parameters.sweep.abs();
    let segments = ((64.0 * sweep / std::f64::consts::TAU).ceil() as usize).clamp(8, 128);
    let mut out = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let angle = parameters.start + parameters.sweep * (i as f64) / (segments as f64);
        let (sin, cos) = angle.sin_cos();
        out.push(Point3 {
            x: parameters.center.x + parameters.radius * cos,
            y: parameters.center.y + parameters.radius * sin,
            z: parameters.center.z,
        });
    }
    Some(out)
}

/// The circle for a circle-tool preview: centre + edge point (the live cursor
/// fills the edge while only the centre is captured).
fn circle_points(points: &[Point3], cursor: Option<Point3>) -> Option<Vec<Point3>> {
    let center = *points.first()?;
    let edge = points.get(1).copied().or(cursor)?;
    let radius = (edge.x - center.x).hypot(edge.y - center.y);
    if !radius.is_finite() || radius < 1e-12 {
        return None;
    }
    let segments = 64usize;
    let mut out = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let t = std::f64::consts::TAU * (i as f64) / (segments as f64);
        let (sin, cos) = t.sin_cos();
        out.push(Point3 {
            x: center.x + radius * cos,
            y: center.y + radius * sin,
            z: center.z,
        });
    }
    Some(out)
}

/// The closed outline for a RECTANG preview: the captured first corner plus the
/// live cursor as the second, closed back onto the first.
///
/// `None` (so the caller falls back to the rubber-band chain) while fewer than two
/// corners are known or the span has no width/height, so a zero-area rectangle is
/// never previewed as if it were a real one. The four corners are the same
/// canonical outline the commit builds (lower-left first, counter-clockwise).
fn rectangle_points(points: &[Point3], cursor: Option<Point3>) -> Option<Vec<Point3>> {
    let corner1 = *points.first()?;
    let corner2 = points.get(1).copied().or(cursor)?;
    let min_x = corner1.x.min(corner2.x);
    let max_x = corner1.x.max(corner2.x);
    let min_y = corner1.y.min(corner2.y);
    let max_y = corner1.y.max(corner2.y);
    if max_x - min_x <= 1e-12 || max_y - min_y <= 1e-12 {
        return None;
    }
    let z = corner1.z;
    let corner = |x: f64, y: f64| Point3 { x, y, z };
    Some(vec![
        corner(min_x, min_y),
        corner(max_x, min_y),
        corner(max_x, max_y),
        corner(min_x, max_y),
        corner(min_x, min_y),
    ])
}

/// Bounds on the ELLIPSE preview tessellation segment count.
///
/// The stored entity is the exact ellipse; the preview is a bounded chord
/// approximation, so its cost stays bounded no matter how eccentric the capture
/// is. The count grows as the axis ratio shrinks (a flatter ellipse needs more
/// chords) and is clamped into `[MIN, MAX]`.
const MIN_ELLIPSE_PREVIEW_SEGMENTS: usize = 16;
const MAX_ELLIPSE_PREVIEW_SEGMENTS: usize = 128;

/// The closed tessellated outline for an ELLIPSE preview.
///
/// The axis-endpoint capture needs three picks (the two endpoints of the first
/// axis plus the other-axis distance); the live cursor fills the next missing
/// pick. The outline is a **bounded chord approximation** of the exact ellipse
/// the commit stores ([`MAX_ELLIPSE_PREVIEW_SEGMENTS`] at most), not the stored
/// entity. `None` (so the caller falls back to the rubber-band chain) while fewer
/// than two picks are known or the picks cannot define an ellipse — coincident
/// endpoints or a third pick on the first-axis line — so a degenerate capture is
/// never previewed as a fabricated full ellipse.
fn ellipse_points(points: &[Point3], cursor: Option<Point3>) -> Option<Vec<Point3>> {
    let (p1, p2, p3) = match points {
        [a, b, c, ..] => (*a, *b, *c),
        [a, b] => (*a, *b, cursor?),
        _ => return None,
    };
    let frame = crate::draw_tool::ellipse_parameters(p1, p2, p3).ok()?;
    let major = frame.major_axis;
    let major_len = (major.x * major.x + major.y * major.y + major.z * major.z).sqrt();
    if !major_len.is_finite() || major_len <= 0.0 {
        return None;
    }
    // More eccentric ellipses get more chords so the flat ends still read as a
    // smooth curve; the count is hard-capped so the preview cost is bounded
    // regardless of the capture (the third pick only reaches it through `ratio`).
    let segments = (64.0 / frame.ratio).clamp(
        MIN_ELLIPSE_PREVIEW_SEGMENTS as f64,
        MAX_ELLIPSE_PREVIEW_SEGMENTS as f64,
    ) as usize;
    // `minor = cross(+Z, major_unit) · (major_len · ratio)` keeps the tessellation
    // in the ellipse's own `+Z` plane, exactly like `cad-geometry::tessellate`.
    let minor_len = major_len * frame.ratio;
    let minor = Point3 {
        x: -major.y / major_len * minor_len,
        y: major.x / major_len * minor_len,
        z: 0.0,
    };
    let mut out = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let t = std::f64::consts::TAU * (i as f64) / (segments as f64);
        let (sin, cos) = t.sin_cos();
        out.push(Point3 {
            x: frame.center.x + major.x * cos + minor.x * sin,
            y: frame.center.y + major.y * cos + minor.y * sin,
            z: frame.center.z + major.z * cos + minor.z * sin,
        });
    }
    Some(out)
}

/// The closed outline for a POLYGON preview: the captured centre plus the live
/// cursor (or second pick) as the radius point, closed back onto the first
/// vertex.
///
/// Uses the **current** session side count carried on the preview
/// ([`DrawPreview::polygon_sides`]) and the same pure [`polygon_vertices`] the
/// commit uses, so the preview is the exact ring the command will store (the
/// closing vertex repeats the first so the `Lines` batch draws `n` segments).
/// `None` (so the caller falls back to the rubber-band chain) while fewer than
/// two picks are known, the side count is unknown, or the capture is degenerate
/// (non-finite, non-coplanar, zero radius, out-of-range count), never a
/// fabricated ring.
fn polygon_points(
    points: &[Point3],
    cursor: Option<Point3>,
    sides: Option<u32>,
) -> Option<Vec<Point3>> {
    let center = *points.first()?;
    let radius_point = points.get(1).copied().or(cursor)?;
    let mut outline = crate::draw_tool::polygon_vertices(center, radius_point, sides?).ok()?;
    if let Some(first) = outline.first().copied() {
        outline.push(first);
    }
    Some(outline)
}

/// Bounds on the SPLINE preview chord approximation segment count.
///
/// The stored entity is the exact interpolated spline; the preview is a bounded
/// chord approximation, so its cost stays bounded no matter how many fit points
/// are captured. The count grows with the fit-point count and is clamped into
/// `[MIN, MAX]`.
const MIN_SPLINE_PREVIEW_SEGMENTS: usize = 16;
const MAX_SPLINE_PREVIEW_SEGMENTS: usize = 256;

/// The bounded chord approximation of a fit-point SPLINE preview.
///
/// The captured fit points plus the live cursor (as the next tentative fit point)
/// are interpolated by the same pure helper the commit uses
/// ([`crate::draw_tool::spline_control_points`]), and the resulting degree-3
/// NURBS is sampled at a bounded number of uniform parameters — the **same curve**
/// the commit stores, drawn as a chord approximation. `None` (so the caller falls
/// back to the rubber-band chain) while fewer than four fit points are known or
/// the interpolation refuses (a degenerate chord, a near-coincident cursor, a
/// non-coplanar set), so a degenerate capture is never previewed as a curve.
fn spline_points(points: &[Point3], cursor: Option<Point3>) -> Option<Vec<Point3>> {
    let mut fit = points.to_vec();
    if let Some(cursor) = cursor {
        if fit.last() != Some(&cursor) {
            fit.push(cursor);
        }
    }
    let (knots, control_points) = crate::draw_tool::spline_control_points(&fit).ok()?;
    let weights = vec![1.0; control_points.len()];
    let curve = cad_geometry::NurbsCurve::new(
        cad_geometry::FIT_SPLINE_DEGREE as usize,
        control_points,
        knots,
        weights,
    )
    .ok()?;
    let (t_min, t_max) = curve.domain();
    let segments = (fit.len() * 16).clamp(MIN_SPLINE_PREVIEW_SEGMENTS, MAX_SPLINE_PREVIEW_SEGMENTS);
    let mut out = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let t = t_min + (t_max - t_min) * (i as f64) / (segments as f64);
        out.push(curve.evaluate(t));
    }
    Some(out)
}

/// Build the object-snap hint overlay for `hints`.
///
/// Each hint is drawn as **one distinct marker whose shape encodes its
/// [`SnapHintKind`]**, so a user can tell an endpoint snap from a midpoint or
/// centre snap at a glance (the run-time distinction [`SnapKind::as_str`]
/// exposes in diagnostics). The whole set is a single bounded `Lines` batch at
/// [`SNAP_HINT_DRAW_ORDER`], drawn **above** every other overlay — including the
/// pointer cursor crosshair and its pickbox and the live tool preview — because
/// the marker is what the user reads at the snap point and the pickbox would
/// otherwise paint over it.
///
/// This is separate from the axis-aligned pointer-cursor crosses that
/// [`preview_overlay`] gates on the same `view.overlays.snapHints` flag: those
/// mark the *cursor* position while drawing, these mark *resolved snap targets*.
/// An empty `hints` slice produces no batch (an explicit empty overlay, never a
/// fabricated one).
pub fn snap_hint_overlay(
    hints: &[SnapHint],
    document: DocumentId,
    options: &PreviewOptions,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    if hints.is_empty() {
        return out;
    }
    let size = options.marker_size;
    if !size.is_finite() || size <= 0.0 {
        return out;
    }
    let mut points: Vec<Point3> = Vec::with_capacity(hints.len() * snap_hint_segment_count() * 2);
    for hint in hints {
        push_snap_hint(&mut points, hint.kind, hint.point, size);
    }
    // One bounded batch; `line_batch` rejects fewer than two vertices.
    if let Some(batch) = line_batch(
        &points,
        SNAP_HINT_COLOR,
        document,
        SNAP_HINT_DRAW_ORDER,
        "overlay.snap-hint",
    ) {
        out.batches.push(batch);
    }
    out
}

/// Build the live box-selection rubber band (ADR 0014 decision B.9).
///
/// The host tessellates the rectangle into world-space segments (any dashing
/// included); this builder only groups them into **one** `Lines` batch at
/// [`BOX_SELECT_DRAW_ORDER`] with the WINDOW/CROSSING colour. It is live gesture
/// feedback, not a selection tint, so it is independent of
/// `visibility.selection_highlight`; an empty segment list produces no batch
/// (an explicit empty overlay, never a fabricated one).
///
/// This never writes the database, the revision or the history: a cancelled
/// drag simply stops passing the preview in.
pub fn box_selection_overlay(
    selection: &BoxSelectionPreview,
    document: DocumentId,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    if selection.segments.is_empty() {
        return out;
    }
    let mut points: Vec<Point3> = Vec::with_capacity(selection.segments.len() * 2);
    for segment in &selection.segments {
        points.extend_from_slice(segment);
    }
    let color = if selection.crossing {
        BOX_SELECT_CROSSING_COLOR
    } else {
        BOX_SELECT_WINDOW_COLOR
    };
    // One batch; `line_batch` rejects fewer than two vertices.
    if let Some(batch) = line_batch(
        &points,
        color,
        document,
        BOX_SELECT_DRAW_ORDER,
        "overlay.box-select",
    ) {
        out.batches.push(batch);
    }
    out
}

/// The maximum number of line segments any single snap marker emits, used to
/// pre-size the vertex buffer without a second pass.
fn snap_hint_segment_count() -> usize {
    // `Center` is the densest marker (a tessellated circle).
    SNAP_HINT_CIRCLE_SEGMENTS.max(4)
}

/// Append one snap marker's segment endpoints to `points`.
///
/// `LineList` topology pairs consecutive vertices, so every shape is emitted as
/// explicit segment pairs and shapes are independent of each other.
///
/// The glyph per kind follows **AutoCAD's object-snap marker convention**, so a
/// user who knows AutoCAD reads the same kind from the same shape:
///
/// | kind | AutoCAD marker | glyph here |
/// |---|---|---|
/// | `Endpoint` | square | axis-aligned square |
/// | `Midpoint` | triangle | upward triangle |
/// | `Center` | circle | tessellated circle |
/// | `Quadrant` | diamond | axis-aligned diamond |
/// | `Perpendicular` | perpendicular (⊥) | vertical stem on a bottom bar |
/// | `LocalIntersection` | X | diagonal cross |
///
/// `Midpoint`/`Quadrant` used to be swapped relative to AutoCAD (midpoint was a
/// diamond, quadrant a triangle); they are corrected here. The `Perpendicular`
/// marker is AutoCAD's ⊥ rather than a plain plus, which also keeps it distinct
/// from the `LocalIntersection` X.
fn push_snap_hint(points: &mut Vec<Point3>, kind: SnapHintKind, center: Point3, size: f64) {
    let offset = |dx: f64, dy: f64| Point3 {
        x: center.x + dx,
        y: center.y + dy,
        z: center.z,
    };
    match kind {
        SnapKind::Endpoint => {
            // Axis-aligned square.
            let a = offset(-size, -size);
            let b = offset(size, -size);
            let c = offset(size, size);
            let d = offset(-size, size);
            push_segment(points, a, b);
            push_segment(points, b, c);
            push_segment(points, c, d);
            push_segment(points, d, a);
        }
        SnapKind::Midpoint => {
            // Upward triangle (AutoCAD's MID marker).
            let top = offset(0.0, size);
            let right = offset(size, -size * 0.8);
            let left = offset(-size, -size * 0.8);
            push_segment(points, top, right);
            push_segment(points, right, left);
            push_segment(points, left, top);
        }
        SnapKind::Center => {
            // Approximated circle.
            let segments = SNAP_HINT_CIRCLE_SEGMENTS.max(3);
            let mut previous: Option<Point3> = None;
            let mut first: Option<Point3> = None;
            for i in 0..segments {
                let theta = std::f64::consts::TAU * (i as f64) / (segments as f64);
                let point = offset(theta.cos() * size, theta.sin() * size);
                if first.is_none() {
                    first = Some(point);
                }
                if let Some(previous) = previous {
                    push_segment(points, previous, point);
                }
                previous = Some(point);
            }
            if let (Some(first), Some(last)) = (first, previous) {
                push_segment(points, last, first);
            }
        }
        SnapKind::Quadrant => {
            // Diamond (square rotated 45°; AutoCAD's QUA marker).
            let top = offset(0.0, size);
            let right = offset(size, 0.0);
            let bottom = offset(0.0, -size);
            let left = offset(-size, 0.0);
            push_segment(points, top, right);
            push_segment(points, right, bottom);
            push_segment(points, bottom, left);
            push_segment(points, left, top);
        }
        SnapKind::Perpendicular => {
            // Perpendicular symbol (AutoCAD's PER marker): a vertical stem rising
            // from a horizontal base bar. Distinct from the `LocalIntersection` X
            // and from the `Midpoint` triangle at the ~10 px marker size.
            push_segment(points, offset(0.0, -size), offset(0.0, size));
            push_segment(points, offset(-size, -size), offset(size, -size));
        }
        SnapKind::LocalIntersection => {
            // Diagonal cross (X), visually distinct from the plus.
            push_segment(points, offset(-size, -size), offset(size, size));
            push_segment(points, offset(-size, size), offset(size, -size));
        }
    }
}

/// Push one independent line segment as two consecutive vertices.
fn push_segment(points: &mut Vec<Point3>, start: Point3, end: Point3) {
    points.push(start);
    points.push(end);
}

/// The rubber-band chain for a preview: captured points followed by the live
/// cursor when one is present and there is at least one captured point.
fn preview_chain(points: &[Point3], cursor: Option<Point3>) -> Vec<Point3> {
    let mut chain: Vec<Point3> = points.to_vec();
    if let Some(cursor) = cursor {
        if !chain.is_empty() && chain.last() != Some(&cursor) {
            chain.push(cursor);
        }
    }
    chain
}

fn measurement_preview_overlay(
    preview: &MeasurementPreview,
    document: DocumentId,
    snap_hints: bool,
    options: &PreviewOptions,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    let mut order = options.draw_order;

    let chain = preview_chain(&preview.points, preview.cursor);
    if chain.len() >= 2 {
        let batch = line_batch(
            &chain,
            MEASUREMENT_PREVIEW_COLOR,
            document,
            order,
            "measurement.preview",
        );
        if let Some(batch) = batch {
            out.batches.push(batch);
            order += 1;
        }
    }
    if snap_hints {
        let markers = marker_batch(
            &preview.points,
            options.marker_size,
            MEASUREMENT_PREVIEW_COLOR,
            document,
            order,
            "measurement.preview",
        );
        if let Some(batch) = markers {
            out.batches.push(batch);
        }
    }
    out
}

/// Build one `Lines` batch through `points` (a polyline; consecutive duplicates
/// are preserved as zero-length segments, exactly as captured).
pub(super) fn line_batch(
    points: &[Point3],
    color: [f32; 3],
    document: DocumentId,
    draw_order: i64,
    code: &str,
) -> Option<RenderBatch> {
    if points.len() < 2 {
        return None;
    }
    let origin = points[0];
    let vertices: Vec<[f32; 3]> = points
        .iter()
        .map(|p| {
            [
                (p.x - origin.x) as f32,
                (p.y - origin.y) as f32,
                (p.z - origin.z) as f32,
            ]
        })
        .collect();
    Some(RenderBatch {
        conics: Vec::new(),
        local_origin: origin,
        topology: RenderTopology::Lines,
        vertices,
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: cad_scene::sanitize_color(color),
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: cad_db::LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![SelectionRef {
            document,
            entity: cad_domain::EntityId(preview_source_id(code)),
            instance: cad_domain::InstancePath::default(),
            sub_element: None,
        }],
        draw_order,
    })
}

/// Build one `Lines` batch of axis-aligned crosses (two segments per point).
fn marker_batch(
    points: &[Point3],
    marker_size: f64,
    color: [f32; 3],
    document: DocumentId,
    draw_order: i64,
    code: &str,
) -> Option<RenderBatch> {
    if points.is_empty() || !marker_size.is_finite() || marker_size <= 0.0 {
        return None;
    }
    let half = marker_size;
    let origin = points[0];
    let mut vertices: Vec<[f32; 3]> = Vec::with_capacity(points.len() * 4);
    let push = |vertices: &mut Vec<[f32; 3]>, p: Point3| {
        vertices.push([
            (p.x - origin.x) as f32,
            (p.y - origin.y) as f32,
            (p.z - origin.z) as f32,
        ]);
    };
    for point in points {
        push(
            &mut vertices,
            Point3 {
                x: point.x - half,
                y: point.y,
                z: point.z,
            },
        );
        push(
            &mut vertices,
            Point3 {
                x: point.x + half,
                y: point.y,
                z: point.z,
            },
        );
        push(
            &mut vertices,
            Point3 {
                x: point.x,
                y: point.y - half,
                z: point.z,
            },
        );
        push(
            &mut vertices,
            Point3 {
                x: point.x,
                y: point.y + half,
                z: point.z,
            },
        );
    }
    Some(RenderBatch {
        conics: Vec::new(),
        local_origin: origin,
        topology: RenderTopology::Lines,
        vertices,
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: cad_scene::sanitize_color(color),
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: cad_db::LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![SelectionRef {
            document,
            entity: cad_domain::EntityId(preview_source_id(code)),
            instance: cad_domain::InstancePath::default(),
            sub_element: None,
        }],
        draw_order,
    })
}

/// A stable synthetic source id for a preview batch.
///
/// Previews are not database objects, so the id is a fixed namespace derived
/// from the overlay kind. It is never a real `EntityId` and never reaches the
/// database; it only lets a host attribute the batch.
fn preview_source_id(code: &str) -> u128 {
    let mut hash: u128 = 0xcbf2_9ce4_8422_2325;
    for byte in code.bytes() {
        hash ^= byte as u128;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
