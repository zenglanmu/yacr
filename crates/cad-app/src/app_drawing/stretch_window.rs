//! The STRETCH crossing window: the legacy rectangle and the crossing
//! polygon/lasso.
//!
//! The rectangle path is the pre-existing one and is kept **byte-identical**:
//! `Rect::contains_point`, `Rect::contains_aabb` and the shared
//! `select_box::geometry_hits_rect` crossing test. The polygon path (a
//! `CPOLYGON` or a free-form lasso — the same containment rule, only the vertex
//! list differs) uses an **even-odd point-in-polygon** test on the XY plane for
//! containment and the shared segment/tessellation tests for a crossing hit.
//!
//! Precision is stated, never guessed. The polygon "fully enclosed" test for a
//! curve/AABB uses the conservative world AABB (sufficient, may under-select);
//! a hit on a bulge polyline, circle, arc, ellipse or spline is chord-tessellated
//! with the same tolerance budget the offset planner uses, and a kind with no
//! exact path (text, insert, mesh, …) falls back to its conservative AABB —
//! exactly the split `select_box` declares for the rectangle crossing test.

use std::borrow::Cow;

use cad_geometry::{tessellate_geometry, TessellationParams};
use cad_spatial::pick_world_bounds;

use super::*;
use crate::select_box::{geometry_hits_rect, Rect};

/// Below this a polyline bulge is a straight chord (the tessellator's rule).
const STRAIGHT_BULGE: f64 = 1e-12;

/// The enclosure classification of one entity against one or more windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Enclosure {
    /// The geometry neither intersects nor is contained by any window.
    Outside,
    /// The geometry hits a window but is not fully enclosed.
    Partial,
    /// The geometry is fully enclosed by one window.
    Full,
}

/// One crossing window: the legacy rectangle or a polygon/lasso.
#[derive(Debug, Clone)]
pub(super) enum StretchWindow {
    /// The two-corner crossing window (the legacy path, byte-identical).
    Rect(Rect),
    /// A crossing polygon/lasso: the boundary vertices in the world XY plane.
    /// Winding does not matter (even-odd), and a repeated closing vertex is
    /// harmless (a zero-length edge).
    Polygon(Vec<(f64, f64)>),
}

impl StretchWindow {
    /// Whether a point is inside (or on) the window.
    pub(super) fn contains_point(&self, point: Point3) -> bool {
        match self {
            StretchWindow::Rect(rect) => rect.contains_point(point),
            StretchWindow::Polygon(polygon) => polygon_contains(polygon, point.x, point.y),
        }
    }

    /// Whether a conservative world AABB is fully inside the window.
    pub(super) fn contains_aabb(&self, min: Point3, max: Point3) -> bool {
        match self {
            StretchWindow::Rect(rect) => rect.contains_aabb(min, max),
            StretchWindow::Polygon(polygon) => polygon_contains_aabb(polygon, min, max),
        }
    }

    /// Whether a geometry (with its transform) intersects or touches the window.
    pub(super) fn hits(&self, geometry: &SemanticGeometry, transform: &Transform3) -> bool {
        match self {
            StretchWindow::Rect(rect) => geometry_hits_rect(geometry, transform, *rect),
            StretchWindow::Polygon(polygon) => geometry_hits_polygon(geometry, transform, polygon),
        }
    }
}

/// Classify one entity's geometry against the windows.
///
/// "Fully enclosed" is two-way (ADR 0016 decision E): a straight `LINE`/polyline
/// needs every vertex inside one window; every other kind (curves can bow out of
/// the vertex hull) needs its conservative world AABB fully inside one window.
/// "Partial" is a hit on a window (the shared crossing hit test) that is not
/// fully enclosed; the per-kind edit is decided independently.
pub(super) fn classify_enclosure(
    geometry: &SemanticGeometry,
    items: &[&crate::PickItem],
    windows: &[StretchWindow],
) -> Enclosure {
    // "Fully enclosed" must be one **single** window: two disjoint windows each
    // holding one endpoint do not enclose the segment between them.
    let full = match geometry {
        SemanticGeometry::Line { start, end } => windows
            .iter()
            .any(|window| window.contains_point(*start) && window.contains_point(*end)),
        SemanticGeometry::Polyline { points, bulges, .. }
            if !points.is_empty() && bulges.iter().all(|bulge| bulge.abs() <= 1e-12) =>
        {
            windows
                .iter()
                .any(|window| points.iter().all(|point| window.contains_point(*point)))
        }
        // CIRCLE/ELLIPSE/INSERT, bulge polylines and every other kind use the
        // conservative bounds fully-inside **one** window path.
        _ => match union_bounds(items) {
            Some((min, max)) => windows.iter().any(|window| window.contains_aabb(min, max)),
            // No computable box: never fabricate an inclusion.
            None => return Enclosure::Outside,
        },
    };
    if full {
        return Enclosure::Full;
    }
    let hits = windows.iter().any(|window| {
        items
            .iter()
            .any(|item| window.hits(&item.geometry, &item.transform))
    });
    if hits {
        Enclosure::Partial
    } else {
        Enclosure::Outside
    }
}

/// The union of the conservative world AABBs of the visible pick items owned by
/// one top-level entity, or `None` when no item contributes a computable box.
fn union_bounds(items: &[&crate::PickItem]) -> Option<(Point3, Point3)> {
    let mut acc: Option<(Point3, Point3)> = None;
    for item in items {
        let Some((min, max)) = pick_world_bounds(&item.geometry, &item.transform) else {
            continue;
        };
        acc = Some(match acc {
            Some((acc_min, acc_max)) => (
                Point3 {
                    x: acc_min.x.min(min.x),
                    y: acc_min.y.min(min.y),
                    z: acc_min.z.min(min.z),
                },
                Point3 {
                    x: acc_max.x.max(max.x),
                    y: acc_max.y.max(max.y),
                    z: acc_max.z.max(max.z),
                },
            ),
            None => (min, max),
        });
    }
    acc
}

/// Even-odd (crossing number) point-in-polygon test on the XY plane.
pub(super) fn polygon_contains(polygon: &[(f64, f64)], x: f64, y: f64) -> bool {
    let count = polygon.len();
    if count < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = count - 1;
    for index in 0..count {
        let (xi, yi) = polygon[index];
        let (xj, yj) = polygon[previous];
        if (yi > y) != (yj > y) {
            let crossing = xi + (y - yi) * (xj - xi) / (yj - yi);
            if x < crossing {
                inside = !inside;
            }
        }
        previous = index;
    }
    inside
}

/// Whether a segment intersects or touches the polygon (a crossing hit).
fn segment_hits_polygon(start: Point3, end: Point3, polygon: &[(f64, f64)]) -> bool {
    if polygon_contains(polygon, start.x, start.y) || polygon_contains(polygon, end.x, end.y) {
        return true;
    }
    let count = polygon.len();
    if count < 2 {
        return false;
    }
    let mut previous = count - 1;
    for index in 0..count {
        let (x0, y0) = polygon[previous];
        let (x1, y1) = polygon[index];
        if segments_intersect(start.x, start.y, end.x, end.y, x0, y0, x1, y1) {
            return true;
        }
        previous = index;
    }
    false
}

/// Whether a sampled polyline intersects or touches the polygon.
fn polyline_hits_polygon(points: &[Point3], polygon: &[(f64, f64)]) -> bool {
    if points.len() < 2 {
        return points
            .first()
            .is_some_and(|point| polygon_contains(polygon, point.x, point.y));
    }
    points
        .windows(2)
        .any(|pair| segment_hits_polygon(pair[0], pair[1], polygon))
}

/// Whether a conservative world AABB can hit the polygon at all (broad phase).
fn polygon_hits_aabb(polygon: &[(f64, f64)], min: Point3, max: Point3) -> bool {
    let corners = [
        (min.x, min.y),
        (max.x, min.y),
        (max.x, max.y),
        (min.x, max.y),
    ];
    if corners
        .iter()
        .any(|&(x, y)| polygon_contains(polygon, x, y))
    {
        return true;
    }
    if polygon
        .iter()
        .any(|&(x, y)| x >= min.x && x <= max.x && y >= min.y && y <= max.y)
    {
        return true;
    }
    let edges = [(0usize, 1usize), (1, 2), (2, 3), (3, 0)];
    let count = polygon.len();
    let mut previous = count - 1;
    for index in 0..count {
        let (x0, y0) = polygon[previous];
        let (x1, y1) = polygon[index];
        for (a, b) in edges {
            let (ax, ay) = corners[a];
            let (bx, by) = corners[b];
            if segments_intersect(ax, ay, bx, by, x0, y0, x1, y1) {
                return true;
            }
        }
        previous = index;
    }
    false
}

/// Whether a conservative world AABB is fully inside a simple polygon.
///
/// Conservative and sound: all four corners plus the centre must be inside, the
/// polygon boundary may not cross a box edge, and no polygon vertex may lie
/// strictly inside the box (a boundary loop inside the box would mean the box is
/// not a subset of the polygon). Missing a containment is possible for a polygon
/// whose boundary hugs the box; claiming one is not.
fn polygon_contains_aabb(polygon: &[(f64, f64)], min: Point3, max: Point3) -> bool {
    let corners = [
        (min.x, min.y),
        (max.x, min.y),
        (max.x, max.y),
        (min.x, max.y),
    ];
    if !corners
        .iter()
        .all(|&(x, y)| polygon_contains(polygon, x, y))
    {
        return false;
    }
    if !polygon_contains(polygon, (min.x + max.x) * 0.5, (min.y + max.y) * 0.5) {
        return false;
    }
    if polygon
        .iter()
        .any(|&(x, y)| x > min.x && x < max.x && y > min.y && y < max.y)
    {
        return false;
    }
    let edges = [(0usize, 1usize), (1, 2), (2, 3), (3, 0)];
    let count = polygon.len();
    let mut previous = count - 1;
    for index in 0..count {
        let (x0, y0) = polygon[previous];
        let (x1, y1) = polygon[index];
        for (a, b) in edges {
            let (ax, ay) = corners[a];
            let (bx, by) = corners[b];
            if segments_properly_cross(ax, ay, bx, by, x0, y0, x1, y1) {
                return false;
            }
        }
        previous = index;
    }
    true
}

/// Whether a geometry (with its transform) hits the polygon (CROSSING).
fn geometry_hits_polygon(
    geometry: &SemanticGeometry,
    transform: &Transform3,
    polygon: &[(f64, f64)],
) -> bool {
    let Some((bounds_min, bounds_max)) = pick_world_bounds(geometry, transform) else {
        return false;
    };
    if !polygon_hits_aabb(polygon, bounds_min, bounds_max) {
        return false;
    }
    let world = if *transform == Transform3::identity() {
        Cow::Borrowed(geometry)
    } else {
        match transform_geometry(geometry, transform) {
            Ok(world) => Cow::Owned(world),
            // The AABB (which already includes the transform) hits; without a
            // transformable geometry the conservative bound is the honest answer.
            Err(_) => return true,
        }
    };
    match &*world {
        SemanticGeometry::Line { start, end } => segment_hits_polygon(*start, *end, polygon),
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            if points.is_empty() {
                return false;
            }
            if bulges.iter().any(|bulge| bulge.abs() > STRAIGHT_BULGE) {
                // A bulge segment is an arc, not its chord: the shared
                // tessellator is the only honest hit test.
                let samples = tessellate_geometry(&world, hit_tessellation());
                return polyline_hits_polygon(&samples, polygon);
            }
            let mut segments = points.clone();
            if *closed && points.len() > 1 {
                segments.push(points[0]);
            }
            polyline_hits_polygon(&segments, polygon)
        }
        SemanticGeometry::Arc { .. }
        | SemanticGeometry::Circle { .. }
        | SemanticGeometry::Ellipse { .. }
        | SemanticGeometry::Spline { .. } => {
            let samples = tessellate_geometry(&world, hit_tessellation());
            polyline_hits_polygon(&samples, polygon)
        }
        SemanticGeometry::Point(point) => polygon_contains(polygon, point.x, point.y),
        // Compound-like kinds classify exactly like their display children.
        SemanticGeometry::Dimension(dimension) => dimension
            .display()
            .iter()
            .any(|child| geometry_hits_polygon(child, &Transform3::identity(), polygon)),
        SemanticGeometry::Hatch(hatch) => hatch
            .display()
            .iter()
            .any(|child| geometry_hits_polygon(child, &Transform3::identity(), polygon)),
        SemanticGeometry::MultiLeader(multileader) => multileader
            .display()
            .iter()
            .any(|child| geometry_hits_polygon(child, &Transform3::identity(), polygon)),
        SemanticGeometry::Compound(children) => children
            .iter()
            .any(|child| geometry_hits_polygon(child, &Transform3::identity(), polygon)),
        // Text/INSERT/Mesh/Shape/Image/Mask/Opaque: no exact path, and the
        // conservative world AABB already hit (the same fallback `select_box`
        // declares for the rectangle CROSSING test).
        _ => true,
    }
}

/// The chord-tessellation budget of a polygon hit test.
fn hit_tessellation() -> TessellationParams {
    let tol = TolerancePolicy::default().computation_world.max(1e-9);
    TessellationParams {
        tolerance: (tol * 32.0).max(1e-6),
        max_segments: 512,
        min_segments: 16,
    }
}

/// A closed-or-touching segment intersection (2D), including collinear overlap.
#[allow(clippy::too_many_arguments)]
fn segments_intersect(
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    cx: f64,
    cy: f64,
    dx: f64,
    dy: f64,
) -> bool {
    let d1 = orient(ax, ay, bx, by, cx, cy);
    let d2 = orient(ax, ay, bx, by, dx, dy);
    let d3 = orient(cx, cy, dx, dy, ax, ay);
    let d4 = orient(cx, cy, dx, dy, bx, by);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    (d1 == 0.0 && on_segment(ax, ay, bx, by, cx, cy))
        || (d2 == 0.0 && on_segment(ax, ay, bx, by, dx, dy))
        || (d3 == 0.0 && on_segment(cx, cy, dx, dy, ax, ay))
        || (d4 == 0.0 && on_segment(cx, cy, dx, dy, bx, by))
}

/// A proper crossing of two 2D segments (interior intersection only).
#[allow(clippy::too_many_arguments)]
fn segments_properly_cross(
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    cx: f64,
    cy: f64,
    dx: f64,
    dy: f64,
) -> bool {
    let d1 = orient(ax, ay, bx, by, cx, cy);
    let d2 = orient(ax, ay, bx, by, dx, dy);
    let d3 = orient(cx, cy, dx, dy, ax, ay);
    let d4 = orient(cx, cy, dx, dy, bx, by);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// The orientation sign of `c` relative to the directed line `a → b`.
fn orient(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64) -> f64 {
    (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
}

/// Whether collinear point `c` lies on the segment `a → b`.
fn on_segment(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64) -> bool {
    cx >= ax.min(bx) && cx <= ax.max(bx) && cy >= ay.min(by) && cy <= ay.max(by)
}
