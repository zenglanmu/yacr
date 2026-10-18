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
        DrawToolKind::Line
        | DrawToolKind::Move
        | DrawToolKind::Trim
        | DrawToolKind::Copy
        | DrawToolKind::Extend
        | DrawToolKind::Rotate
        | DrawToolKind::Scale
        | DrawToolKind::Mirror
        | DrawToolKind::Offset => {
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

/// Build the object-snap hint overlay for `hints`.
///
/// Each hint is drawn as **one distinct marker whose shape encodes its
/// [`SnapHintKind`]**, so a user can tell an endpoint snap from a midpoint or
/// centre snap at a glance (the run-time distinction [`SnapKind::as_str`]
/// exposes in diagnostics). The whole set is a single bounded `Lines` batch at
/// [`SNAP_HINT_DRAW_ORDER`], below the live tool preview so the rubber band the
/// user is acting on stays on top.
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

/// The maximum number of line segments any single snap marker emits, used to
/// pre-size the vertex buffer without a second pass.
fn snap_hint_segment_count() -> usize {
    // `Center` is the densest marker (a tessellated circle).
    SNAP_HINT_CIRCLE_SEGMENTS.max(4)
}

/// Append one snap marker's segment endpoints to `points`.
///
/// `LineList` topology pairs consecutive vertices, so every shape is emitted as
/// explicit, closed segment pairs and shapes are independent of each other.
/// Shape by kind: endpoint → square, midpoint → diamond, center → circle,
/// quadrant → triangle, perpendicular → plus, local intersection → diagonal X.
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
            // Diamond (square rotated 45°).
            let top = offset(0.0, size);
            let right = offset(size, 0.0);
            let bottom = offset(0.0, -size);
            let left = offset(-size, 0.0);
            push_segment(points, top, right);
            push_segment(points, right, bottom);
            push_segment(points, bottom, left);
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
            // Upward triangle.
            let top = offset(0.0, size);
            let right = offset(size, -size * 0.8);
            let left = offset(-size, -size * 0.8);
            push_segment(points, top, right);
            push_segment(points, right, left);
            push_segment(points, left, top);
        }
        SnapKind::Perpendicular => {
            // Axis-aligned plus.
            push_segment(points, offset(-size, 0.0), offset(size, 0.0));
            push_segment(points, offset(0.0, -size), offset(0.0, size));
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
