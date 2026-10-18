use cad_domain::*;

use crate::ImageVertex;

use super::clip_reason;
use super::selection::ClipRefusal;
use super::view::finite_point;

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
    let mut polygon: Vec<ImageVertex> = Vec::with_capacity(8);
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
    Ok(clip_image_polygon_to_rect(polygon, center, half))
}

/// Clip an image polygon (already in paper coordinates, carrying UVs) to the
/// paper rectangle, interpolating the texture coordinates at the new vertices.
///
/// The polygon must be convex (the renderer fan-triangulates it); Sutherland–
/// Hodgman against the four axis-aligned half-planes then yields the exact
/// intersection. An empty result means the polygon lies fully outside the
/// window — an exact omission.
pub(super) fn clip_image_polygon_to_rect(
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
pub(super) fn image_polygon_is_convex(polygon: &[ImageVertex]) -> bool {
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
