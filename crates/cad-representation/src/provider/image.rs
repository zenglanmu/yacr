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
pub(super) fn image_basis_is_degenerate(u: Point3, v: Point3, pixels: [f64; 2]) -> bool {
    if !pixels[0].is_finite() || !pixels[1].is_finite() || pixels[0] == 0.0 || pixels[1] == 0.0 {
        return true;
    }
    if !cad_geometry::is_finite(u) || !cad_geometry::is_finite(v) {
        return true;
    }
    cad_geometry::length(cad_geometry::cross(u, v)) <= 1e-24
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
/// closing vertex is dropped). Returns `None` when the clip cannot be expressed
/// as a visible-region polygon — an outside/mask clip (`inside == false`), a
/// degenerate rectangle, fewer than two vertices, a non-finite position, or a
/// non-convex polygon (the renderer fan-triangulates assuming convexity) — so
/// the caller reports it instead of silently dropping it.
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
    let mut out = Vec::with_capacity(corners.len());
    for [vx, vy] in corners {
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
