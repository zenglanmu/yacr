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

/// Map an entity-level image clip to a world-space visible-region polygon.
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
/// The polygon is intersected with the image rectangle `[0,pixels]` (the sampler
/// is `ClampToEdge`, so emitting an out-of-image rect would stretch edge texels).
/// Returns `None` when the clip cannot be expressed as a non-empty visible-region
/// polygon — an outside/mask clip (`inside == false`), a degenerate rectangle,
/// fewer than two vertices, a non-finite position, a non-convex polygon (the
/// renderer fan-triangulates assuming convexity), or a fully clipped-away /
/// degenerate intersection — so the caller reports it instead of silently
/// dropping it or drawing stretched edges.
pub(super) fn image_clip_vertices(
    clip: &ImageClip,
    origin: Point3,
    u: Point3,
    v: Point3,
    pixels: [f64; 2],
) -> Option<Vec<ImageVertex>> {
    if !clip.inside {
        return None;
    }
    if !pixels[0].is_finite() || !pixels[1].is_finite() || pixels[0] == 0.0 || pixels[1] == 0.0 {
        return None;
    }
    let corners: Vec<[f64; 2]> = match clip.vertices.len() {
        2 => {
            let [a, b] = [clip.vertices[0], clip.vertices[1]];
            // Adjacent or coincident "corners" do not span a rectangle.
            if a[0] == b[0] || a[1] == b[1] {
                return None;
            }
            vec![[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]]
        }
        n if n >= 3 => {
            let mut polygon = clip.vertices.clone();
            if polygon.first() == polygon.last() {
                polygon.pop();
            }
            polygon
        }
        _ => return None,
    };
    if corners.len() < 3 {
        return None;
    }
    // The renderer fan-triangulates the visible-region polygon, which is only
    // correct for a convex polygon. A non-convex DXF image clip is legal but
    // cannot be expressed here, so it is refused (the caller draws the frame and
    // reports it) rather than rendered as a silently wrong region.
    if !polygon_is_convex(&corners) {
        return None;
    }
    // Clip to the image rectangle. The intersection keeps the inside of every
    // half-plane, so it stays convex and its UVs stay inside `[0,1]`.
    let clipped = clip_polygon_to_image(&corners, pixels)?;
    let mut out = Vec::with_capacity(clipped.len());
    for [vx, vy] in clipped {
        let position = cad_geometry::add(
            origin,
            cad_geometry::add(cad_geometry::scale(u, vx), cad_geometry::scale(v, vy)),
        );
        if !cad_geometry::is_finite(position) {
            return None;
        }
        out.push(ImageVertex {
            position,
            uv: [vx / pixels[0], vy / pixels[1]],
        });
    }
    Some(out)
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
/// convex. The renderer fan-triangulates a clip polygon assuming convexity, so a
/// non-convex polygon must be refused instead of rendered wrong.
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
mod tests {
    use super::*;

    fn axis(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn a_scaled_basis_that_overflows_is_degenerate() {
        let u = axis(1.0, 0.0);
        let v = axis(0.0, 1.0);
        assert!(!image_basis_is_degenerate(u, v, [2.0, 3.0]));
        // The unscaled `u`/`v` are finite, but `u * pixels[0]` overflows the
        // world-space extent, so no valid texture transform exists.
        assert!(image_basis_is_degenerate(u, v, [f64::MAX, 2.0]));
    }

    #[test]
    fn a_clipped_polygon_never_escapes_the_image_rectangle() {
        let pixels = [2.0, 3.0];
        let clipped = clip_polygon_to_image(
            &[[-1.0, -1.0], [1.0, -1.0], [1.0, 4.0], [-1.0, 4.0]],
            pixels,
        )
        .expect("the intersection is the full-height strip");
        assert_eq!(
            clipped,
            vec![[0.0, 3.0], [0.0, 0.0], [1.0, 0.0], [1.0, 3.0]]
        );
        assert!(clipped
            .iter()
            .all(|point| (0.0..=pixels[0]).contains(&point[0])
                && (0.0..=pixels[1]).contains(&point[1])));
    }
}
