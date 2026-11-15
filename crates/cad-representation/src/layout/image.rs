use std::sync::Arc;

use cad_domain::*;

use crate::ImageVertex;

use super::clip_reason;
use super::polygon::PaperClip;
use super::selection::ClipRefusal;
use super::view::finite_point;

/// The visible part of one image through a viewport clip.
///
/// A rectangle keeps the single convex result the half-plane clippers produce; a
/// general (possibly concave) clip can cut one image into several convex pieces,
/// which the caller emits as several image fragments over the same resource so
/// the union is exactly the visible region.
pub(crate) enum ImageRegion {
    /// Nothing is visible: an exact omission, not a degradation.
    Empty,
    /// The whole image is visible; `entity` is the entity clip, unchanged.
    Whole { entity: Option<Arc<[ImageVertex]>> },
    /// Convex paper-space pieces, each with interpolated texture coordinates.
    Pieces(Vec<Vec<ImageVertex>>),
}

/// Resolve the visible region of one image under `clip`.
///
/// `Err` means the region cannot be computed exactly (a non-finite quad, or a
/// non-convex entity clip that would have to be intersected); the caller keeps
/// the original primitive and reports the refusal.
pub(crate) fn visible_image(
    transform: &Transform3,
    entity: Option<&[ImageVertex]>,
    clip: &PaperClip,
) -> Result<ImageRegion, ClipRefusal> {
    if clip.as_rect().is_some() {
        return visible_image_to_rect(transform, entity, clip.center(), clip.half());
    }
    visible_image_to_polygon(transform, entity, clip)
}

/// The rectangle path: byte-for-byte the behaviour the half-plane clippers had.
fn visible_image_to_rect(
    transform: &Transform3,
    entity: Option<&[ImageVertex]>,
    center: [f64; 2],
    half: [f64; 2],
) -> Result<ImageRegion, ClipRefusal> {
    let vertices = clip_image_quad_to_rect(transform, center, half)?;
    if vertices.is_empty() {
        return Ok(ImageRegion::Empty);
    }
    if image_quad_inside(transform, center, half) {
        // The whole quad is visible; any entity clip still applies unchanged.
        return Ok(ImageRegion::Whole {
            entity: entity.map(Arc::from),
        });
    }
    match entity {
        Some(entity_clip) => {
            if !image_polygon_is_convex(entity_clip) {
                // A provider handed us a non-convex clip (its primitive
                // contract says convex); half-plane intersection would be
                // wrong, so report instead of widening or dropping it.
                return Err(ClipRefusal::new(
                    clip_reason::IMAGE_INTERSECTION_UNSUPPORTED,
                    "a non-convex image clip cannot be intersected with the viewport window exactly; the entity clip is kept unclipped",
                ));
            }
            let intersected = clip_image_polygon_to_rect(entity_clip.to_vec(), center, half);
            if intersected.is_empty() {
                Ok(ImageRegion::Empty)
            } else {
                Ok(ImageRegion::Pieces(vec![intersected]))
            }
        }
        None => Ok(ImageRegion::Pieces(vec![vertices])),
    }
}

/// The general-polygon path: trianguate the clip, intersect the (convex) image
/// region with every convex window, and keep the non-empty pieces.
fn visible_image_to_polygon(
    transform: &Transform3,
    entity: Option<&[ImageVertex]>,
    clip: &PaperClip,
) -> Result<ImageRegion, ClipRefusal> {
    let quad = image_quad(transform)?;
    let quad_positions: Vec<[f64; 2]> = quad
        .iter()
        .map(|vertex| [vertex.position.x, vertex.position.y])
        .collect();
    if clip.holds(&quad_positions) {
        // A concave clip that holds the whole quad cannot bite into it, so the
        // image needs no cut at all.
        return Ok(ImageRegion::Whole {
            entity: entity.map(Arc::from),
        });
    }
    let base = match entity {
        Some(entity_clip) => {
            if !image_polygon_is_convex(entity_clip) {
                return Err(ClipRefusal::new(
                    clip_reason::IMAGE_INTERSECTION_UNSUPPORTED,
                    "a non-convex image clip cannot be intersected with the viewport window exactly; the entity clip is kept unclipped",
                ));
            }
            entity_clip.to_vec()
        }
        None => quad,
    };
    let base_positions: Vec<[f64; 2]> = base
        .iter()
        .map(|vertex| [vertex.position.x, vertex.position.y])
        .collect();
    if clip.holds(&base_positions) {
        return Ok(ImageRegion::Whole {
            entity: entity.map(Arc::from),
        });
    }
    let mut pieces = Vec::new();
    for window in clip.triangles() {
        let piece = clip_image_vertices_to_window(base.clone(), &window);
        if piece.len() >= 3 {
            pieces.push(piece);
        }
    }
    if pieces.is_empty() {
        Ok(ImageRegion::Empty)
    } else {
        Ok(ImageRegion::Pieces(pieces))
    }
}

/// The unit square mapped to paper space, carrying each corner's UV.
fn image_quad(transform: &Transform3) -> Result<Vec<ImageVertex>, ClipRefusal> {
    let corners = [
        (Point3::default(), [0.0, 0.0]),
        (
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            [1.0, 0.0],
        ),
        (
            Point3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
            [1.0, 1.0],
        ),
        (
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            [0.0, 1.0],
        ),
    ];
    let mut polygon: Vec<ImageVertex> = Vec::with_capacity(corners.len());
    for (corner, uv) in corners {
        let position = transform.apply_point(corner);
        if !finite_point(&position) {
            return Err(ClipRefusal::new(
                clip_reason::IMAGE_DEGENERATE,
                "image transform maps the quad to a non-finite position; it cannot be clipped",
            ));
        }
        polygon.push(ImageVertex { position, uv });
    }
    Ok(polygon)
}

/// Sutherland–Hodgman clip of an image polygon against a **convex**
/// counter-clockwise window, interpolating UVs (and z) at the new vertices.
fn clip_image_vertices_to_window(input: Vec<ImageVertex>, window: &[[f64; 2]]) -> Vec<ImageVertex> {
    let mut out = input;
    let count = window.len();
    for index in 0..count {
        if out.is_empty() {
            return out;
        }
        let a = window[index];
        let b = window[(index + 1) % count];
        let side = |p: [f64; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        let n = out.len();
        let mut clipped = Vec::with_capacity(n + 1);
        for k in 0..n {
            let current = out[k];
            let previous = out[(k + n - 1) % n];
            let current_side = side([current.position.x, current.position.y]);
            let previous_side = side([previous.position.x, previous.position.y]);
            let crossing = |previous: ImageVertex, current: ImageVertex| {
                let denominator = previous_side - current_side;
                let t = if denominator.abs() > 1e-300 {
                    previous_side / denominator
                } else {
                    0.0
                };
                lerp_image_vertex(previous, current, t)
            };
            if current_side >= 0.0 {
                if previous_side < 0.0 {
                    clipped.push(crossing(previous, current));
                }
                clipped.push(current);
            } else if previous_side >= 0.0 {
                clipped.push(crossing(previous, current));
            }
        }
        out = clipped;
    }
    out.dedup_by(|a, b| a.position == b.position && a.uv == b.uv);
    if out.len() >= 2
        && out[0].position == out[out.len() - 1].position
        && out[0].uv == out[out.len() - 1].uv
    {
        out.pop();
    }
    out
}

fn lerp_image_vertex(a: ImageVertex, b: ImageVertex, t: f64) -> ImageVertex {
    ImageVertex {
        position: Point3 {
            x: a.position.x + (b.position.x - a.position.x) * t,
            y: a.position.y + (b.position.y - a.position.y) * t,
            z: a.position.z + (b.position.z - a.position.z) * t,
        },
        uv: [
            a.uv[0] + (b.uv[0] - a.uv[0]) * t,
            a.uv[1] + (b.uv[1] - a.uv[1]) * t,
        ],
    }
}

/// Clip an image's unit-square quad, mapped through `transform`, to the paper
/// rectangle and interpolate the texture coordinates at the new vertices.
///
/// `Ok(vertices)` is the surviving convex polygon (empty when the image lies
/// fully outside the window, an exact omission). `Err` means the transform maps
/// the quad to a non-finite position, so it cannot be clipped.
pub fn clip_image_quad_to_rect(
    transform: &Transform3,
    center: [f64; 2],
    half: [f64; 2],
) -> Result<Vec<ImageVertex>, ClipRefusal> {
    Ok(clip_image_polygon_to_rect(
        image_quad(transform)?,
        center,
        half,
    ))
}

/// Clip an image polygon (already in paper coordinates, carrying UVs) to the
/// paper rectangle, interpolating the texture coordinates at the new vertices.
///
/// The polygon must be convex (the renderer fan-triangulates it); Sutherland–
/// Hodgman against the four axis-aligned half-planes then yields the exact
/// intersection. An empty result means the polygon lies fully outside the
/// window — an exact omission.
pub(crate) fn clip_image_polygon_to_rect(
    mut polygon: Vec<ImageVertex>,
    center: [f64; 2],
    half: [f64; 2],
) -> Vec<ImageVertex> {
    let min = [center[0] - half[0], center[1] - half[1]];
    let max = [center[0] + half[0], center[1] + half[1]];
    polygon = clip_image_vertices(polygon, 0, min[0], true);
    polygon = clip_image_vertices(polygon, 0, max[0], false);
    polygon = clip_image_vertices(polygon, 1, min[1], true);
    polygon = clip_image_vertices(polygon, 1, max[1], false);
    polygon.dedup_by(|a, b| a.position == b.position && a.uv == b.uv);
    if polygon.len() >= 2 {
        let (first, last) = (polygon[0], polygon[polygon.len() - 1]);
        if first.position == last.position && first.uv == last.uv {
            polygon.pop();
        }
    }
    polygon
}

fn clip_image_vertices(
    input: Vec<ImageVertex>,
    axis: usize,
    bound: f64,
    keep_above: bool,
) -> Vec<ImageVertex> {
    if input.is_empty() {
        return Vec::new();
    }
    let inside = |p: [f64; 2]| {
        if keep_above {
            p[axis] >= bound
        } else {
            p[axis] <= bound
        }
    };
    let n = input.len();
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..n {
        let current = input[i];
        let previous = input[(i + n - 1) % n];
        let current_in = inside([current.position.x, current.position.y]);
        let previous_in = inside([previous.position.x, previous.position.y]);
        if current_in {
            if !previous_in {
                out.push(intersect_image_vertices(previous, current, axis, bound));
            }
            out.push(current);
        } else if previous_in {
            out.push(intersect_image_vertices(previous, current, axis, bound));
        }
    }
    out
}

fn intersect_image_vertices(
    a: ImageVertex,
    b: ImageVertex,
    axis: usize,
    bound: f64,
) -> ImageVertex {
    let a_coord = [a.position.x, a.position.y][axis];
    let b_coord = [b.position.x, b.position.y][axis];
    let denom = b_coord - a_coord;
    let t = if denom.abs() > 1e-300 {
        (bound - a_coord) / denom
    } else {
        0.0
    };
    let mut position = a.position;
    if axis == 0 {
        position.x = bound;
    } else {
        position.y = bound;
    }
    let other = 1 - axis;
    let a_other = [a.position.x, a.position.y][other];
    let b_other = [b.position.x, b.position.y][other];
    let other_value = a_other + (b_other - a_other) * t;
    if other == 0 {
        position.x = other_value;
    } else {
        position.y = other_value;
    }
    position.z = a.position.z + (b.position.z - a.position.z) * t;
    ImageVertex {
        position,
        uv: [
            a.uv[0] + (b.uv[0] - a.uv[0]) * t,
            a.uv[1] + (b.uv[1] - a.uv[1]) * t,
        ],
    }
}

/// Whether every corner of an image's unit-square quad is inside the window, so
/// the image needs no clip polygon at all.
pub(super) fn image_quad_inside(transform: &Transform3, center: [f64; 2], half: [f64; 2]) -> bool {
    let min = [center[0] - half[0], center[1] - half[1]];
    let max = [center[0] + half[0], center[1] + half[1]];
    [
        Point3::default(),
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
    ]
    .iter()
    .all(|corner| {
        let p = transform.apply_point(*corner);
        p.x >= min[0] && p.x <= max[0] && p.y >= min[1] && p.y <= max[1]
    })
}

/// Whether an image polygon (paper-space positions, at least three vertices) is
/// convex.
///
/// Consecutive collinear edges are allowed; a sign change between consecutive
/// edge cross products is a reflex vertex. The renderer fan-triangulates a clip
/// polygon assuming convexity, so only a convex polygon may be intersected with
/// the viewport rectangle by half-plane clipping.
pub(crate) fn image_polygon_is_convex(polygon: &[ImageVertex]) -> bool {
    let n = polygon.len();
    if n < 3 {
        return true;
    }
    let mut sign = 0i8;
    for i in 0..n {
        let a = polygon[i].position;
        let b = polygon[(i + 1) % n].position;
        let c = polygon[(i + 2) % n].position;
        let e1 = (b.x - a.x, b.y - a.y);
        let e2 = (c.x - b.x, c.y - b.y);
        let len = e1.0.hypot(e1.1) * e2.0.hypot(e2.1);
        if len <= 0.0 {
            // A repeated vertex: no turn to classify.
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
