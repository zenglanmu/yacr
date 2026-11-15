use crate::*;

/// The four world-space corners of an image frame, closed (first point
/// repeated) so a `Lines` primitive outlines all four edges.
pub(super) fn image_frame_corners(
    origin: Point3,
    u: Point3,
    v: Point3,
    pixels: [f64; 2],
) -> Vec<Point3> {
    let du = cad_geometry::scale(u, pixels[0]);
    let dv = cad_geometry::scale(v, pixels[1]);
    let p1 = cad_geometry::add(origin, du);
    let p2 = cad_geometry::add(p1, dv);
    let p3 = cad_geometry::add(origin, dv);
    vec![origin, p1, p2, p3, origin]
}

/// Whether an image frame's basis cannot define a plane.
///
/// A zero pixel count, a non-finite basis, or a zero-length cross product means
/// no texture can be mapped, so the caller must fall back to the plain frame.
/// The **scaled** basis (`u*pixels[0]`, `v*pixels[1]`) is checked too: a finite
/// `u`/`v` with a pixel count that overflows their product still has no finite
/// world-space extent, so it cannot be textured.
pub(super) fn image_basis_is_degenerate(u: Point3, v: Point3, pixels: [f64; 2]) -> bool {
    if !pixels[0].is_finite() || !pixels[1].is_finite() || pixels[0] == 0.0 || pixels[1] == 0.0 {
        return true;
    }
    if !cad_geometry::is_finite(u) || !cad_geometry::is_finite(v) {
        return true;
    }
    let du = cad_geometry::scale(u, pixels[0]);
    let dv = cad_geometry::scale(v, pixels[1]);
    if !cad_geometry::is_finite(du) || !cad_geometry::is_finite(dv) {
        return true;
    }
    let cross = cad_geometry::cross(du, dv);
    if !cad_geometry::is_finite(cross) {
        return true;
    }
    cad_geometry::length(cross) <= 1e-24
}

/// Resolve an image entity's texture from the decoded-image cache.
///
/// The path-preserving [`image_resource_key`] is tried first so two files with
/// the same base name stay distinct; a cache populated under the old bare-name
/// key still resolves through the fallback. Returns the key that actually hit
/// (so the primitive carries the one the renderer can look up) and the shared
/// image, or — on a miss — the preferred key and `None`. An unsafe reference is
/// never looked up and yields no key, so the caller reports it unresolved.
pub(super) fn resolve_image_texture(
    file: Option<&str>,
    images: Option<&DecodedImageCache>,
) -> (Option<ResourceKey>, Option<Arc<DecodedImage>>) {
    let Some(reference) = file else {
        return (None, None);
    };
    let Some(relative) = image_resource_key(reference) else {
        return (None, None);
    };
    let Some(cache) = images else {
        return (Some(relative), None);
    };
    if let Some(image) = cache.get(&relative) {
        return (Some(relative), Some(image));
    }
    // Bare-file-name fallback for a cache keyed the old way.
    let bare = ResourceKey::sanitize(reference);
    if bare != relative {
        if let Some(image) = cache.get(&bare) {
            return (Some(bare), Some(image));
        }
    }
    (Some(relative), None)
}

/// Stable diagnostic codes for an image clip this build cannot honour.
///
/// Every refusal keeps the frame drawn and marks the entity `Partial`; none of
/// them draws a guessed or widened region.
pub(super) mod image_clip_reason {
    /// An outside/mask clip (`inside == false`), whose visible region is the
    /// complement of its boundary and therefore not a visible-region polygon.
    pub const UNSUPPORTED: &str = "image.clip_unsupported";
    /// Fewer than two vertices, or two "corners" that do not span a rectangle, or
    /// a stored polygon that does not leave three distinct points.
    pub const ARITY: &str = "image.clip_arity";
    /// A non-finite coordinate, a non-finite world position, or zero area.
    pub const DEGENERATE: &str = "image.clip_degenerate";
    /// The ring crosses or touches itself, so it has no well-defined interior.
    pub const SELF_INTERSECTING: &str = "image.clip_self_intersecting";
}

/// Why one image clip could not become visible-region pieces.
#[derive(Debug)]
pub(super) struct ImageClipRefusal {
    /// Stable, machine-readable code (see [`image_clip_reason`]).
    pub code: &'static str,
    /// Human-readable half of the diagnostic.
    pub message: String,
}

impl ImageClipRefusal {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        ImageClipRefusal {
            code,
            message: message.into(),
        }
    }
}

/// Relative tolerance for the clip ring's own extent, so a 3-pixel clip and a
/// 3000-pixel clip are classified alike.
const RING_RELATIVE_TOLERANCE: f64 = 1e-9;

/// Map an entity-level image clip to world-space visible-region pieces.
///
/// `ImageClip::vertices` are pixel-space; a vertex `(vx, vy)` maps to
/// `origin + u*vx + v*vy` with UV `(vx/pixels.0, vy/pixels.1)`. Exactly two
/// vertices are read as opposite corners of a rectangle and expanded to its four
/// ordered corners; three or more are taken as the polygon directly (a repeated
/// closing vertex is dropped).
///
/// **Clip axes**: X is along `u` and Y along `v`, in the image's own pixel space,
/// which is **bottom-up** — `y = 0` is the bottom image edge, `y = pixels[1]` the
/// top. This matches the UV mapping, where `v = 0` is the bottom of the texture.
///
/// The region is intersected with the image rectangle `[0,pixels]` (the sampler
/// is `ClampToEdge`, so emitting an out-of-image rect would stretch edge texels).
///
/// **Convex and concave boundaries.** Every returned piece is convex, because
/// that is the contract the renderer's triangle fan consumes:
///
/// * a **convex** clip yields exactly one piece — the same half-plane
///   intersection the fan path has always produced, so convex output is
///   byte-identical to the previous behaviour;
/// * a **concave** clip cannot be fanned, so the ring is ear-clipped in pixel
///   space and each ear is intersected with the image rectangle on its own (a
///   triangle clipped by a convex window stays convex). The ears are disjoint and
///   tile the ring exactly, so the pieces follow the boundary — no chording, no
///   bounding-box fallback, no overlapping or missing area.
///
/// Returns `Ok(Some(pieces))` for a drawable clip, `Ok(None)` when the clip is
/// well formed but nothing survives the intersection with the image rectangle
/// (a fully clipped-away image), and `Err(reason)` for a clip that is not a
/// usable region at all — a mask clip, fewer than three points, non-finite or
/// zero-area geometry, or a self-intersecting ring. The caller draws the frame
/// and reports `reason.code`; it never draws a guessed region.
pub(super) fn image_clip_pieces(
    clip: &ImageClip,
    origin: Point3,
    u: Point3,
    v: Point3,
    pixels: [f64; 2],
) -> Result<Option<Vec<Vec<ImageVertex>>>, ImageClipRefusal> {
    if !clip.inside {
        return Err(ImageClipRefusal::new(
            image_clip_reason::UNSUPPORTED,
            "an outside (inverted) image clip keeps the region around its boundary, which a \
             visible-region polygon cannot express; only the frame is drawn",
        ));
    }
    if !pixels[0].is_finite() || !pixels[1].is_finite() || pixels[0] == 0.0 || pixels[1] == 0.0 {
        return Err(ImageClipRefusal::new(
            image_clip_reason::DEGENERATE,
            "the image has no finite pixel extent, so a clip cannot be mapped to texture \
             coordinates; only the frame is drawn",
        ));
    }
    let ring = clip_ring(clip)?;
    if !ring
        .iter()
        .all(|point| point[0].is_finite() && point[1].is_finite())
    {
        return Err(ImageClipRefusal::new(
            image_clip_reason::DEGENERATE,
            "the image clip has non-finite pixel coordinates; only the frame is drawn",
        ));
    }
    let extent = ring_extent(&ring);
    if extent <= 0.0 || polygon_area(&ring).abs() <= RING_RELATIVE_TOLERANCE * extent * extent {
        return Err(ImageClipRefusal::new(
            image_clip_reason::DEGENERATE,
            "the image clip has zero area (coincident or collinear vertices); only the frame \
             is drawn",
        ));
    }
    if !ring_is_simple(&ring) {
        return Err(ImageClipRefusal::new(
            image_clip_reason::SELF_INTERSECTING,
            "the image clip boundary crosses or touches itself, so it has no well-defined \
             interior; only the frame is drawn",
        ));
    }
    let pieces = if polygon_is_convex(&ring) {
        // Byte-identical fast path: one convex piece, from the same half-plane
        // intersection the fan triangulation has always consumed.
        clip_polygon_to_image(&ring, pixels).into_iter().collect()
    } else {
        image_concave_pieces(&ring, pixels)?
    };
    if pieces.is_empty() {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(pieces.len());
    for piece in pieces {
        let mut vertices = Vec::with_capacity(piece.len());
        for [vx, vy] in piece {
            let position = cad_geometry::add(
                origin,
                cad_geometry::add(cad_geometry::scale(u, vx), cad_geometry::scale(v, vy)),
            );
            if !cad_geometry::is_finite(position) {
                return Err(ImageClipRefusal::new(
                    image_clip_reason::DEGENERATE,
                    "the image clip maps to a non-finite world position; only the frame is drawn",
                ));
            }
            vertices.push(ImageVertex {
                position,
                uv: [vx / pixels[0], vy / pixels[1]],
            });
        }
        out.push(vertices);
    }
    Ok(Some(out))
}

/// The clip's pixel-space ring, in stored order.
///
/// Two vertices expand to the four ordered corners of their rectangle; three or
/// more are the stored polygon with a repeated closing vertex dropped.
fn clip_ring(clip: &ImageClip) -> Result<Vec<[f64; 2]>, ImageClipRefusal> {
    match clip.vertices.len() {
        2 => {
            let [a, b] = [clip.vertices[0], clip.vertices[1]];
            // Adjacent or coincident "corners" do not span a rectangle.
            if a[0] == b[0] || a[1] == b[1] {
                return Err(ImageClipRefusal::new(
                    image_clip_reason::ARITY,
                    "the two image clip corners share an x or a y coordinate, so they do not \
                     span a rectangle; only the frame is drawn",
                ));
            }
            Ok(vec![[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]])
        }
        count if count >= 3 => {
            let mut ring = clip.vertices.clone();
            if ring.first() == ring.last() {
                ring.pop();
            }
            Ok(ring)
        }
        count => Err(ImageClipRefusal::new(
            image_clip_reason::ARITY,
            format!(
                "an image clip needs two rectangle corners or a polygon of at least three \
                 points; found {count}"
            ),
        )),
    }
}

/// Ear-clip a concave pixel-space ring into convex visible-region pieces.
///
/// The ring is normalised first (consecutive coincident vertices removed, wound
/// counter-clockwise) because ear clipping needs both. `triangulate_ring` then
/// returns `n - 2` triangles that tile the ring exactly; each is intersected with
/// the image rectangle on its own, which keeps every piece convex. A triangle
/// that lies entirely outside the rectangle contributes no piece, and one that
/// only grazes it is dropped by the intersection's own area check.
fn image_concave_pieces(
    ring: &[[f64; 2]],
    pixels: [f64; 2],
) -> Result<Vec<Vec<[f64; 2]>>, ImageClipRefusal> {
    let mut boundary: Vec<[f64; 2]> = Vec::with_capacity(ring.len());
    for point in ring {
        if boundary
            .last()
            .is_none_or(|last| last[0] != point[0] || last[1] != point[1])
        {
            boundary.push(*point);
        }
    }
    while boundary.len() > 1 && boundary.first() == boundary.last() {
        boundary.pop();
    }
    if boundary.len() < 3 {
        return Err(ImageClipRefusal::new(
            image_clip_reason::DEGENERATE,
            "the image clip has fewer than three distinct points; only the frame is drawn",
        ));
    }
    if polygon_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    let triangles = triangulate_ring(&boundary).ok_or_else(|| {
        ImageClipRefusal::new(
            image_clip_reason::DEGENERATE,
            "the image clip boundary could not be triangulated (numerically degenerate \
             geometry); only the frame is drawn",
        )
    })?;
    let mut pieces = Vec::with_capacity(triangles.len());
    for triangle in triangles {
        if let Some(piece) = clip_polygon_to_image(&triangle, pixels) {
            pieces.push(piece);
        }
    }
    Ok(pieces)
}

/// The larger axis span of a pixel-space ring.
fn ring_extent(ring: &[[f64; 2]]) -> f64 {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for point in ring {
        for (axis, bound) in min.iter_mut().enumerate() {
            *bound = bound.min(point[axis]);
        }
        for (axis, bound) in max.iter_mut().enumerate() {
            *bound = bound.max(point[axis]);
        }
    }
    (max[0] - min[0]).max(max[1] - min[1])
}

/// Whether a pixel-space ring is **simple**: no two non-adjacent edges cross or
/// touch.
fn ring_is_simple(ring: &[[f64; 2]]) -> bool {
    let count = ring.len();
    let tolerance = RING_RELATIVE_TOLERANCE * ring_extent(ring).max(1.0);
    for first in 0..count {
        let first_next = (first + 1) % count;
        for second in (first + 1)..count {
            let second_next = (second + 1) % count;
            // Adjacent edges share a vertex by construction; only an edge that
            // meets an edge it shares no vertex with proves self-intersection.
            if second == first_next || second_next == first {
                continue;
            }
            if segments_cross(
                ring[first],
                ring[first_next],
                ring[second],
                ring[second_next],
                tolerance,
            ) {
                return false;
            }
        }
    }
    true
}

/// Whether segments `a1→a2` and `b1→b2` cross or touch, within `tolerance`.
fn segments_cross(a1: [f64; 2], a2: [f64; 2], b1: [f64; 2], b2: [f64; 2], tolerance: f64) -> bool {
    let limit = tolerance
        * (a2[0] - a1[0])
            .abs()
            .max((a2[1] - a1[1]).abs())
            .max((b2[0] - b1[0]).abs())
            .max((b2[1] - b1[1]).abs())
            .max(1.0);
    let d1 = turn(b1, b2, a1);
    let d2 = turn(b1, b2, a2);
    let d3 = turn(a1, a2, b1);
    let d4 = turn(a1, a2, b2);
    let opposite = |first: f64, second: f64| {
        first.abs() > limit && second.abs() > limit && first.signum() != second.signum()
    };
    if opposite(d1, d2) && opposite(d3, d4) {
        return true;
    }
    point_on_segment(a1, b1, b2, tolerance)
        || point_on_segment(a2, b1, b2, tolerance)
        || point_on_segment(b1, a1, a2, tolerance)
        || point_on_segment(b2, a1, a2, tolerance)
}

/// Whether `point` lies on the closed segment `a`–`b`, within `tolerance`.
fn point_on_segment(point: [f64; 2], a: [f64; 2], b: [f64; 2], tolerance: f64) -> bool {
    if turn(a, b, point).abs() > tolerance * (b[0] - a[0]).hypot(b[1] - a[1]).max(tolerance) {
        return false;
    }
    point[0] >= a[0].min(b[0]) - tolerance
        && point[0] <= a[0].max(b[0]) + tolerance
        && point[1] >= a[1].min(b[1]) - tolerance
        && point[1] <= a[1].max(b[1]) + tolerance
}

/// The signed cross product `(b - a) x (c - a)`; positive is a left turn.
fn turn(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Whether `point` is inside the triangle `a`,`b`,`c` (either winding), boundary
/// included.
fn point_in_triangle(point: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let area = turn(a, b, c);
    if area == 0.0 {
        return false;
    }
    let sign = if area > 0.0 { 1.0 } else { -1.0 };
    sign * turn(a, b, point) >= 0.0
        && sign * turn(b, c, point) >= 0.0
        && sign * turn(c, a, point) >= 0.0
}

/// Ear-clip a counter-clockwise simple pixel-space ring into triangles.
///
/// This is the same ear-clipping algorithm the paper clip boundary uses
/// (`cad_representation::layout::polygon`, which is module-private and therefore
/// not reachable here): every convex vertex whose triangle contains no other
/// vertex is an ear, and removing `n - 3` ears leaves the final triangle. A
/// validated simple ring always has an ear, so `None` means the ring is
/// numerically unusable — the caller refuses by name instead of dropping
/// vertices and silently losing area.
fn triangulate_ring(ring: &[[f64; 2]]) -> Option<Vec<[[f64; 2]; 3]>> {
    let count = ring.len();
    if count < 3 {
        return None;
    }
    if count == 3 {
        return Some(vec![[ring[0], ring[1], ring[2]]]);
    }
    let mut indices: Vec<usize> = (0..count).collect();
    let mut triangles = Vec::with_capacity(count - 2);
    while indices.len() > 3 {
        let remaining = indices.len();
        let mut ear = None;
        for k in 0..remaining {
            let (previous, current, next) = (
                indices[(k + remaining - 1) % remaining],
                indices[k],
                indices[(k + 1) % remaining],
            );
            let (a, b, c) = (ring[previous], ring[current], ring[next]);
            // A reflex or collinear vertex is never an ear of a CCW ring.
            if turn(a, b, c) <= 0.0 {
                continue;
            }
            let blocked = indices.iter().any(|index| {
                *index != previous
                    && *index != current
                    && *index != next
                    && point_in_triangle(ring[*index], a, b, c)
            });
            if blocked {
                continue;
            }
            ear = Some((k, [a, b, c]));
            break;
        }
        let (k, triangle) = ear?;
        triangles.push(triangle);
        indices.remove(k);
    }
    triangles.push([ring[indices[0]], ring[indices[1]], ring[indices[2]]]);
    Some(triangles)
}

/// Intersect a pixel-space convex polygon with the image rectangle `[0,pixels]`.
///
/// Sutherland–Hodgman against the four axis-aligned edges, in order. The result
/// is open (not closed) and duplicates consecutive/coincident vertices are
/// dropped. `None` when fewer than three distinct vertices or an effectively
/// zero area survive, so a fully-outside or edge-only clip is refused rather
/// than emitting a degenerate primitive.
fn clip_polygon_to_image(points: &[[f64; 2]], pixels: [f64; 2]) -> Option<Vec<[f64; 2]>> {
    let mut polygon = points.to_vec();
    for (axis, bound, keep_greater) in [
        (0usize, 0.0, true),
        (0, pixels[0], false),
        (1usize, 0.0, true),
        (1, pixels[1], false),
    ] {
        polygon = clip_half_plane(&polygon, axis, bound, keep_greater);
        if polygon.len() < 3 {
            return None;
        }
    }
    let mut deduped: Vec<[f64; 2]> = Vec::with_capacity(polygon.len());
    for point in polygon {
        if !point[0].is_finite() || !point[1].is_finite() {
            return None;
        }
        if deduped
            .last()
            .is_none_or(|last| last[0] != point[0] || last[1] != point[1])
        {
            deduped.push(point);
        }
    }
    if deduped.len() > 2 && deduped.first() == deduped.last() {
        deduped.pop();
    }
    if deduped.len() < 3 || polygon_area(&deduped).abs() <= 1e-12 {
        return None;
    }
    Some(deduped)
}

/// Keep the half-plane `coordinate[axis] >= bound` (or `<=` when
/// `keep_greater` is false) of an open polygon.
fn clip_half_plane(
    polygon: &[[f64; 2]],
    axis: usize,
    bound: f64,
    keep_greater: bool,
) -> Vec<[f64; 2]> {
    let inside = |point: [f64; 2]| {
        if keep_greater {
            point[axis] >= bound
        } else {
            point[axis] <= bound
        }
    };
    let mut out = Vec::with_capacity(polygon.len() + 1);
    for index in 0..polygon.len() {
        let current = polygon[index];
        let previous = polygon[(index + polygon.len() - 1) % polygon.len()];
        let current_inside = inside(current);
        let previous_inside = inside(previous);
        if current_inside {
            if !previous_inside {
                out.push(edge_intersection(previous, current, axis, bound));
            }
            out.push(current);
        } else if previous_inside {
            out.push(edge_intersection(previous, current, axis, bound));
        }
    }
    out
}

/// The point where segment `a→b` crosses `coordinate[axis] == bound`.
fn edge_intersection(a: [f64; 2], b: [f64; 2], axis: usize, bound: f64) -> [f64; 2] {
    let denominator = b[axis] - a[axis];
    if denominator == 0.0 {
        // The edge is parallel to the clip line; the crossing point is the
        // current endpoint projected onto the bound.
        let mut point = b;
        point[axis] = bound;
        return point;
    }
    let t = (bound - a[axis]) / denominator;
    [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
}

/// Twice the signed area of an open pixel-space polygon.
fn polygon_area(points: &[[f64; 2]]) -> f64 {
    let mut sum = 0.0;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum / 2.0
}

/// Whether a pixel-space polygon is convex.
///
/// `points` is the open polygon (a repeated closing vertex already dropped).
/// Consecutive collinear edges are allowed — a zero cross product does not fix a
/// turn direction — but any sign change between consecutive edge cross products
/// is a reflex vertex, i.e. non-convex. Fewer than three points is trivially
/// convex.
///
/// This selects the clip's **fast path**, not a refusal: a convex ring is one
/// half-plane intersection (byte-identical to the behaviour before concave clips
/// were supported), while a non-convex ring is ear-clipped into convex pieces.
/// It is not a simplicity test — a pentagram has a consistent turn sign and is
/// classified convex, which is why [`ring_is_simple`] runs first.
fn polygon_is_convex(points: &[[f64; 2]]) -> bool {
    let n = points.len();
    if n < 3 {
        return true;
    }
    let mut sign = 0i8;
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        let c = points[(i + 2) % n];
        let e1 = (b[0] - a[0], b[1] - a[1]);
        let e2 = (c[0] - b[0], c[1] - b[1]);
        let len = e1.0.hypot(e1.1) * e2.0.hypot(e2.1);
        if len <= 0.0 {
            // A zero-length edge is a repeated vertex: no turn to classify.
            continue;
        }
        let cross = e1.0 * e2.1 - e1.1 * e2.0;
        if cross.abs() <= 1e-9 * len {
            // Collinear: allowed, does not fix the winding sign.
            continue;
        }
        let s = if cross > 0.0 { 1 } else { -1 };
        if sign == 0 {
            sign = s;
        } else if sign != s {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests;
