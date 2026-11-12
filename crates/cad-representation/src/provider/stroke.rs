//! Wide polyline stroke tessellation (uniform PLINE width, v1).
//!
//! `SemanticGeometry::Polyline` carries one **uniform** width in world units
//! (`width`), centred on the vertices: AutoCAD's constant LWPOLYLINE width
//! (DXF group 43). Width `0.0` is the hairline default and stays on the
//! existing centreline `Lines` path untouched; a positive width becomes a
//! filled triangle **band** emitted as [`DisplayPrimitive::Mesh`].
//!
//! ## Geometry rule
//!
//! The centreline is tessellated with the **same** call the hairline path uses
//! ([`cad_geometry::tessellate_geometry`] with the build's chord tolerance), so
//! bulges become the same arcs and no second curve model exists. Each
//! tessellated vertex is then offset by `+width / 2` and `-width / 2` along the
//! in-plane normal of its segment, and consecutive offsets are joined into two
//! triangles per span (a closed polyline wraps the last span to the first).
//!
//! * **Joins**: a mitre join per interior vertex (the offset direction is the
//!   normalised sum of the two adjacent segment normals, scaled by
//!   `1 / cos(θ / 2)`). The mitre length is clamped to
//!   [`MITER_LIMIT_FACTOR`] × half-width; beyond that the corner is
//!   **bevelled** (the offset falls back to the single segment normal), so a
//!   near-reversal cannot create an arbitrarily long spike.
//! * **Caps**: an open polyline ends flush at its first/last vertex (butt
//!   caps), never a rounded or extended cap.
//! * **Arcs**: a bulge arc is tessellated first, so its band is the offset of
//!   that chord approximation (a polygonal approximation of the true annulus
//!   sector with the build's display tolerance).
//! * **Plane**: the band lies in the polyline's own plane
//!   ([`cad_geometry::polyline_plane_normal`]); a two-point or degenerate plane
//!   falls back to an arbitrary perpendicular of the segment direction, never
//!   to a flattened world-XY assumption.
//!
//! ## Declared v1 boundaries
//!
//! * **Per-segment taper** (per-vertex start/end widths, DXF groups 40/41) is
//!   not modelled: the band has one width everywhere.
//! * **Self-intersections / overlapping joins** are not resolved (the band is
//!   painted as-is, like a filled polygon); no boolean union is attempted.
//! * A **dashed linetype** is not subdivided for a wide stroke (the band is
//!   drawn solid); `cad-representation`'s expansion reports
//!   `representation.polyline_width_dashes` instead of silently dropping the
//!   pattern.
//!
//! Selection, bounds, snapping and measurement keep using the centreline, which
//! the domain documents as the AutoCAD geometry semantics; this module only
//! builds display geometry.

use cad_domain::{Mesh, Point3, SemanticGeometry};
use cad_geometry::{
    cross, distance, dot, length, normalize, polyline_plane_normal, scale, tessellate_geometry,
    TessellationParams,
};

/// Mitre-length clamp, as a multiple of the stroke half-width.
///
/// A join whose mitre would be longer than this is bevelled instead: `4.0`
/// keeps a real sharp corner (up to about 29° interior angle) while rejecting
/// the unbounded spike a near-reversal would produce.
pub(crate) const MITER_LIMIT_FACTOR: f64 = 4.0;

/// Smallest distance two consecutive vertices must differ by to form a segment.
/// Below it the pair is treated as one vertex (the same order of magnitude as
/// `cad-db`'s `MIN_GEOMETRY_EXTENT`).
const MIN_SEGMENT_LENGTH: f64 = 1e-12;

/// Build the triangle band for a polyline whose `width` is finite and `> 0`.
///
/// Returns `None` when the tessellated centreline has fewer than two distinct
/// vertices (nothing can be stroked) or a coordinate is non-finite; the caller
/// then keeps the centreline draw and reports the degeneracy rather than
/// emitting a partial band as a success.
pub(crate) fn wide_polyline_mesh(
    geometry: &SemanticGeometry,
    params: TessellationParams,
    width: f64,
) -> Option<Mesh> {
    let SemanticGeometry::Polyline {
        points: vertices,
        closed,
        ..
    } = geometry
    else {
        return None;
    };
    if !width.is_finite() || width <= 0.0 {
        return None;
    }
    let centerline = dedupe(&tessellate_geometry(geometry, params), *closed);
    if centerline.len() < 2 {
        return None;
    }
    if !centerline
        .iter()
        .all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
    {
        return None;
    }
    let half = width / 2.0;
    let normal = plane_normal(vertices, &centerline);
    let offsets = vertex_offsets(&centerline, *closed, normal, half)?;

    let count = centerline.len();
    let mut mesh = Mesh {
        vertices: Vec::with_capacity(count * 2),
        triangles: Vec::with_capacity(count * 2),
        normals: Vec::with_capacity(count * 2),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    for (point, offset) in centerline.iter().zip(offsets.iter()) {
        mesh.vertices.push(add(*point, *offset));
        mesh.vertices.push(add(*point, scale(*offset, -1.0)));
        mesh.normals.push(normal);
        mesh.normals.push(normal);
    }
    let spans = if *closed { count } else { count - 1 };
    for index in 0..spans {
        let next = (index + 1) % count;
        let (left_a, right_a) = (index as u32 * 2, index as u32 * 2 + 1);
        let (left_b, right_b) = (next as u32 * 2, next as u32 * 2 + 1);
        mesh.triangles.push([left_a, right_a, right_b]);
        mesh.triangles.push([left_a, right_b, left_b]);
    }
    (!mesh.triangles.is_empty()).then_some(mesh)
}

fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

/// Drop a repeated closing vertex and consecutive coincident points.
fn dedupe(points: &[Point3], closed: bool) -> Vec<Point3> {
    let mut out: Vec<Point3> = Vec::with_capacity(points.len());
    for point in points {
        if out
            .last()
            .is_none_or(|last| distance(*last, *point) > MIN_SEGMENT_LENGTH)
        {
            out.push(*point);
        }
    }
    if closed && out.len() > 1 {
        let first = out[0];
        let last = out[out.len() - 1];
        if distance(first, last) <= MIN_SEGMENT_LENGTH {
            out.pop();
        }
    }
    out
}

/// The band's plane normal, derived from the source vertices when they span a
/// plane and from the segment direction otherwise.
fn plane_normal(source: &[Point3], centerline: &[Point3]) -> Point3 {
    let candidate = polyline_plane_normal(source);
    let mut segments: Vec<(Point3, Point3)> = centerline
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .collect();
    if centerline.len() > 2 {
        segments.push((centerline[centerline.len() - 1], centerline[0]));
    }
    let in_plane = segments.iter().any(|(start, end)| {
        let direction = normalize(sub(*end, *start));
        length(cross(candidate, direction)) > 1e-9
    });
    if in_plane {
        return candidate;
    }
    // The candidate is parallel to every segment (for example a two-point line
    // along world Z): pick any perpendicular of the first real direction.
    let direction = normalize(sub(centerline[1], centerline[0]));
    let axis = normalize(cross(
        direction,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    ));
    if length(axis) > 1e-9 {
        normalize(cross(direction, axis))
    } else {
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        }
    }
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

/// One offset vector per centreline vertex (magnitude = half-width, direction
/// = the in-plane miter/bevel normal).
fn vertex_offsets(
    points: &[Point3],
    closed: bool,
    normal: Point3,
    half: f64,
) -> Option<Vec<Point3>> {
    let count = points.len();
    let segment_normals: Vec<Point3> = (0..if closed { count } else { count - 1 })
        .map(|index| {
            let next = (index + 1) % count;
            let direction = normalize(sub(points[next], points[index]));
            normalize(cross(normal, direction))
        })
        .collect();
    let mut offsets = Vec::with_capacity(count);
    for index in 0..count {
        let (before, after) = if closed {
            (
                segment_normals[(index + count - 1) % count],
                segment_normals[index % segment_normals.len()],
            )
        } else if index == 0 {
            (segment_normals[0], segment_normals[0])
        } else if index == count - 1 {
            let last = *segment_normals.last()?;
            (last, last)
        } else {
            (segment_normals[index - 1], segment_normals[index])
        };
        offsets.push(miter_offset(before, after, half));
    }
    Some(offsets)
}

/// Mitre at a vertex: `half * miter / cos(θ / 2)`, clamped to
/// [`MITER_LIMIT_FACTOR`]; a near-reversal (or an unbounded mitre) bevels by
/// using the incoming normal alone.
fn miter_offset(before: Point3, after: Point3, half: f64) -> Point3 {
    let sum = add(before, after);
    let sum_length = length(sum);
    if sum_length <= 1e-12 {
        return scale(before, half);
    }
    let miter = scale(sum, 1.0 / sum_length);
    let denominator = dot(miter, before);
    if denominator.abs() <= 1e-9 {
        return scale(before, half);
    }
    let factor = 1.0 / denominator;
    if !factor.is_finite() || factor.abs() > MITER_LIMIT_FACTOR {
        return scale(before, half);
    }
    scale(miter, half * factor)
}
