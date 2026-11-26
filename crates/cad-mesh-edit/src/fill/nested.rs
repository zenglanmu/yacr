//! Coplanar nested boundary-loop (annulus) filling for P-MESH-3.
//!
//! A boundary loop nested inside another coplanar loop is not an independent
//! cap: filling both would cover the hole between them. This module groups the
//! loops by containment depth, bridges each direct child into the outer polygon
//! with a keyhole cut, and ear-clips the resulting annulus.

use cad_domain::Point3;

use super::{basis, cross2, diagonal, dot, point_in_triangle, self_intersects, signed_area};
use crate::topology::newell_normal;

/// One loop's plane description used for nesting and annulus filling.
struct LoopShape {
    normal: Point3,
    centroid: Point3,
    area: f64,
    diagonal: f64,
}

impl LoopShape {
    fn new(vertices: &[Point3], boundary: &[u32]) -> Option<Self> {
        if boundary.len() < 3 {
            return None;
        }
        let points: Vec<Point3> = boundary
            .iter()
            .map(|index| vertices[*index as usize])
            .collect();
        let normal = newell_normal(&points);
        let length = (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z).sqrt();
        if length <= 1e-12 {
            return None;
        }
        let normal = Point3 {
            x: normal.x / length,
            y: normal.y / length,
            z: normal.z / length,
        };
        let count = points.len() as f64;
        let centroid = points.iter().fold(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            |sum, point| Point3 {
                x: sum.x + point.x,
                y: sum.y + point.y,
                z: sum.z + point.z,
            },
        );
        Some(Self {
            normal,
            centroid: Point3 {
                x: centroid.x / count,
                y: centroid.y / count,
                z: centroid.z / count,
            },
            area: length / 2.0,
            diagonal: diagonal(&points),
        })
    }

    fn tolerance(&self) -> f64 {
        1e-6 * self.diagonal.max(1.0)
    }

    /// Whether every vertex of `child` projects inside `parent`'s plane polygon.
    fn contains_projected(&self, vertices: &[Point3], parent: &[u32], child: &[u32]) -> bool {
        let parent_points: Vec<Point3> = parent
            .iter()
            .map(|index| vertices[*index as usize])
            .collect();
        let (u, v) = basis(self.normal, &parent_points);
        let polygon: Vec<[f64; 2]> = parent_points
            .iter()
            .map(|point| [dot(*point, u), dot(*point, v)])
            .collect();
        child.iter().all(|index| {
            let point = vertices[*index as usize];
            point_in_polygon([dot(point, u), dot(point, v)], &polygon)
        })
    }

    fn max_plane_distance(&self, vertices: &[Point3], boundary: &[u32]) -> f64 {
        boundary
            .iter()
            .map(|index| {
                let point = vertices[*index as usize];
                ((point.x - self.centroid.x) * self.normal.x
                    + (point.y - self.centroid.y) * self.normal.y
                    + (point.z - self.centroid.z) * self.normal.z)
                    .abs()
            })
            .fold(0.0, f64::max)
    }
}

/// Containment forest of boundary loops in one plane.
pub(super) struct Nesting {
    parents: Vec<Option<usize>>,
    depth: Vec<usize>,
}

impl Nesting {
    pub(super) fn build(vertices: &[Point3], loops: &[Vec<u32>]) -> Self {
        let count = loops.len();
        let shapes: Vec<Option<LoopShape>> = loops
            .iter()
            .map(|boundary| LoopShape::new(vertices, boundary))
            .collect();
        let mut parents = vec![None; count];
        for child in 0..count {
            let Some(child_shape) = &shapes[child] else {
                continue;
            };
            // Only coplanar containment is nesting: loops in parallel planes
            // (the two rims of a tube) are independent, not holes of each other.
            let mut candidate: Option<(usize, f64)> = None;
            for parent in 0..count {
                if parent == child {
                    continue;
                }
                let Some(parent_shape) = &shapes[parent] else {
                    continue;
                };
                if parent_shape.area <= child_shape.area {
                    continue;
                }
                if !parent_shape.contains_projected(vertices, &loops[parent], &loops[child]) {
                    continue;
                }
                if parent_shape.max_plane_distance(vertices, &loops[child])
                    > parent_shape.tolerance()
                {
                    continue;
                }
                if candidate
                    .map(|(_, area)| parent_shape.area < area)
                    .unwrap_or(true)
                {
                    candidate = Some((parent, parent_shape.area));
                }
            }
            if let Some((parent, _)) = candidate {
                parents[child] = Some(parent);
            }
        }
        let mut depth = vec![0usize; count];
        for index in 0..count {
            let mut levels = 0usize;
            let mut current = parents[index];
            while let Some(parent) = current {
                levels += 1;
                current = parents[parent];
                if levels > count {
                    levels = 0;
                    break;
                }
            }
            depth[index] = levels;
        }
        Self { parents, depth }
    }

    pub(super) fn depth(&self, index: usize) -> usize {
        self.depth[index]
    }

    pub(super) fn children(&self, parent: usize) -> Vec<usize> {
        self.parents
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| (*candidate == Some(parent)).then_some(index))
            .collect()
    }

    pub(super) fn subtree(&self, root: usize) -> Vec<usize> {
        let mut rows = vec![root];
        let mut cursor = 0;
        while cursor < rows.len() {
            let parent = rows[cursor];
            cursor += 1;
            rows.extend(self.children(parent));
        }
        rows
    }
}

#[derive(Clone, Copy)]
struct ProjectedVertex {
    point: [f64; 2],
    vertex: u32,
}

pub(super) struct GroupFill {
    pub(super) triangles: Vec<[u32; 3]>,
    pub(super) non_planar: bool,
}

/// Fill the region between an outer loop and its direct nested holes.
pub(super) fn fill_group(
    vertices: &[Point3],
    outer: &[u32],
    holes: &[&Vec<u32>],
) -> Result<GroupFill, (&'static str, String)> {
    let outer_points: Vec<Point3> = outer
        .iter()
        .map(|index| vertices[*index as usize])
        .collect();
    let normal = newell_normal(&outer_points);
    let length = (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z).sqrt();
    if length <= 1e-12 {
        return Err((
            "mesh.hole_degenerate",
            "the outer boundary loop has zero area".into(),
        ));
    }
    let normal = Point3 {
        x: normal.x / length,
        y: normal.y / length,
        z: normal.z / length,
    };
    let diagonal = diagonal(&outer_points);
    let tolerance = 1e-6 * diagonal.max(1.0);
    let count = outer_points.len() as f64;
    let centroid = outer_points.iter().fold(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        |sum, point| Point3 {
            x: sum.x + point.x,
            y: sum.y + point.y,
            z: sum.z + point.z,
        },
    );
    let centroid = Point3 {
        x: centroid.x / count,
        y: centroid.y / count,
        z: centroid.z / count,
    };
    let non_planar = deviates_from_plane(vertices, outer, normal, centroid, tolerance)
        || holes
            .iter()
            .any(|hole| deviates_from_plane(vertices, hole, normal, centroid, tolerance));
    let (u, v) = basis(normal, &outer_points);
    // Every loop is reversed so the cap traverses each shell boundary edge in
    // the opposite direction, exactly like the single-loop path.
    let project = |boundary: &[u32]| -> Result<Vec<ProjectedVertex>, (&'static str, String)> {
        if boundary.len() < 3 {
            return Err((
                "mesh.hole_degenerate",
                "a nested boundary loop needs at least three vertices".into(),
            ));
        }
        let projected: Vec<ProjectedVertex> = boundary
            .iter()
            .rev()
            .map(|index| {
                let point = vertices[*index as usize];
                ProjectedVertex {
                    point: [dot(point, u), dot(point, v)],
                    vertex: *index,
                }
            })
            .collect();
        let coords: Vec<[f64; 2]> = projected.iter().map(|vertex| vertex.point).collect();
        if self_intersects(&coords) {
            return Err((
                "mesh.hole_self_intersecting",
                "a boundary loop in the nested group intersects itself in the group plane".into(),
            ));
        }
        Ok(projected)
    };
    let mut merged = project(outer)?;
    let coords: Vec<[f64; 2]> = merged.iter().map(|vertex| vertex.point).collect();
    let outer_area = signed_area(&coords);
    let area_tolerance = 1e-12 * diagonal.max(1.0) * diagonal.max(1.0);
    if outer_area.abs() <= area_tolerance {
        return Err((
            "mesh.hole_degenerate",
            "the outer boundary loop has zero area when projected".into(),
        ));
    }
    let mut expected = outer_area.abs();
    for hole in holes {
        let projected = project(hole)?;
        let coords: Vec<[f64; 2]> = projected.iter().map(|vertex| vertex.point).collect();
        let hole_area = signed_area(&coords);
        if hole_area.abs() <= area_tolerance {
            return Err((
                "mesh.hole_degenerate",
                "a nested hole loop has zero area when projected".into(),
            ));
        }
        if outer_area.is_sign_positive() == hole_area.is_sign_positive() {
            return Err((
                "mesh.hole_nested_inconsistent_winding",
                "a nested loop winds the same way as its outer loop; the shell orientation is \
                 inconsistent"
                    .into(),
            ));
        }
        expected -= hole_area.abs();
        merge_hole(&mut merged, &projected)?;
    }
    if expected <= area_tolerance {
        return Err((
            "mesh.hole_degenerate",
            "the nested loops enclose no net area".into(),
        ));
    }
    let coords: Vec<[f64; 2]> = merged.iter().map(|vertex| vertex.point).collect();
    let patch = ear_clip_merged(&coords).ok_or_else(|| {
        (
            "mesh.hole_not_simple",
            "the nested boundary group could not be triangulated as a simple annulus".to_string(),
        )
    })?;
    let area: f64 = patch
        .iter()
        .map(|triangle| {
            cross2(
                coords[triangle[0]],
                coords[triangle[1]],
                coords[triangle[2]],
            )
            .abs()
                / 2.0
        })
        .sum();
    if (area - expected).abs() > 1e-6 * expected.max(1.0) {
        return Err((
            "mesh.hole_not_simple",
            format!("the nested patch covers {area}, but the region area is {expected}"),
        ));
    }
    Ok(GroupFill {
        triangles: patch
            .into_iter()
            .map(|triangle| {
                [
                    merged[triangle[0]].vertex,
                    merged[triangle[1]].vertex,
                    merged[triangle[2]].vertex,
                ]
            })
            .collect(),
        non_planar,
    })
}

fn deviates_from_plane(
    vertices: &[Point3],
    boundary: &[u32],
    normal: Point3,
    centroid: Point3,
    tolerance: f64,
) -> bool {
    boundary.iter().any(|index| {
        let point = vertices[*index as usize];
        ((point.x - centroid.x) * normal.x
            + (point.y - centroid.y) * normal.y
            + (point.z - centroid.z) * normal.z)
            .abs()
            > tolerance
    })
}

/// Splice one hole into the outer polygon with a keyhole bridge.
fn merge_hole(
    merged: &mut Vec<ProjectedVertex>,
    hole: &[ProjectedVertex],
) -> Result<(), (&'static str, String)> {
    let mut best: Option<(usize, usize, f64)> = None;
    for (hole_index, hole_vertex) in hole.iter().enumerate() {
        for (merged_index, merged_vertex) in merged.iter().enumerate() {
            if !bridge_visible(merged, hole, merged_index, hole_index) {
                continue;
            }
            let dx = merged_vertex.point[0] - hole_vertex.point[0];
            let dy = merged_vertex.point[1] - hole_vertex.point[1];
            let distance = dx * dx + dy * dy;
            if distance <= 1e-24 {
                continue;
            }
            if best
                .map(|(_, _, current)| distance < current)
                .unwrap_or(true)
            {
                best = Some((merged_index, hole_index, distance));
            }
        }
    }
    let Some((merged_index, hole_index, _)) = best else {
        return Err((
            "mesh.hole_not_simple",
            "no visible bridge connects a nested hole to the outer region".into(),
        ));
    };
    let mut spliced = Vec::with_capacity(merged.len() + hole.len() + 2);
    spliced.extend_from_slice(&merged[..=merged_index]);
    spliced.extend_from_slice(&hole[hole_index..]);
    spliced.extend_from_slice(&hole[..=hole_index]);
    spliced.extend_from_slice(&merged[merged_index..]);
    *merged = spliced;
    Ok(())
}

fn bridge_visible(
    merged: &[ProjectedVertex],
    hole: &[ProjectedVertex],
    merged_index: usize,
    hole_index: usize,
) -> bool {
    let a = merged[merged_index].point;
    let b = hole[hole_index].point;
    let midpoint = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    let merged_coords: Vec<[f64; 2]> = merged.iter().map(|vertex| vertex.point).collect();
    let hole_coords: Vec<[f64; 2]> = hole.iter().map(|vertex| vertex.point).collect();
    if !point_in_polygon(midpoint, &merged_coords) || point_in_polygon(midpoint, &hole_coords) {
        return false;
    }
    for (index, vertex) in merged.iter().enumerate() {
        if index == merged_index || same_point(vertex.point, a) || same_point(vertex.point, b) {
            continue;
        }
        if point_on_segment(vertex.point, a, b) {
            return false;
        }
    }
    for (index, vertex) in hole.iter().enumerate() {
        if index == hole_index || same_point(vertex.point, a) || same_point(vertex.point, b) {
            continue;
        }
        if point_on_segment(vertex.point, a, b) {
            return false;
        }
    }
    for (index, vertex) in merged.iter().enumerate() {
        let next = merged[(index + 1) % merged.len()].point;
        if same_point(vertex.point, a) || same_point(next, a) {
            continue;
        }
        if proper_cross(vertex.point, next, a, b) {
            return false;
        }
    }
    for (index, vertex) in hole.iter().enumerate() {
        let next = hole[(index + 1) % hole.len()].point;
        if same_point(vertex.point, b) || same_point(next, b) {
            continue;
        }
        if proper_cross(vertex.point, next, a, b) {
            return false;
        }
    }
    true
}

/// Ear clipping that tolerates the duplicated vertices a keyhole bridge adds.
fn ear_clip_merged(points: &[[f64; 2]]) -> Option<Vec<[usize; 3]>> {
    let count = points.len();
    if count < 3 {
        return None;
    }
    let mut order: Vec<usize> = (0..count).collect();
    let area = signed_area(points);
    let flipped = area < 0.0;
    if flipped {
        order.reverse();
    }
    let mut result = Vec::with_capacity(count - 2);
    let mut guard = count.saturating_mul(count).saturating_add(4);
    while order.len() > 3 {
        guard = guard.checked_sub(1)?;
        let len = order.len();
        let mut ear = None;
        for index in 0..len {
            let prev = order[(index + len - 1) % len];
            let current = order[index];
            let next = order[(index + 1) % len];
            if cross2(points[prev], points[current], points[next]) <= 1e-12 {
                continue;
            }
            let mut contains = false;
            for other in &order {
                if *other == prev || *other == current || *other == next {
                    continue;
                }
                // A keyhole bridge duplicates the two endpoints; the copy must
                // not block the ear it belongs to.
                if same_point(points[*other], points[prev])
                    || same_point(points[*other], points[current])
                    || same_point(points[*other], points[next])
                {
                    continue;
                }
                if point_in_triangle(points[*other], points[prev], points[current], points[next]) {
                    contains = true;
                    break;
                }
            }
            if !contains {
                ear = Some((index, [prev, current, next]));
                break;
            }
        }
        let (index, triangle) = ear?;
        result.push(triangle);
        order.remove(index);
    }
    result.push([order[0], order[1], order[2]]);
    if flipped {
        for triangle in &mut result {
            triangle.swap(0, 1);
        }
    }
    Some(result)
}

fn same_point(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() <= 1e-12 && (a[1] - b[1]).abs() <= 1e-12
}

fn point_on_segment(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> bool {
    if cross2(a, b, point).abs() > 1e-12 {
        return false;
    }
    point[0] >= a[0].min(b[0]) - 1e-12
        && point[0] <= a[0].max(b[0]) + 1e-12
        && point[1] >= a[1].min(b[1]) - 1e-12
        && point[1] <= a[1].max(b[1]) + 1e-12
}

fn proper_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let d1 = cross2(c, d, a);
    let d2 = cross2(c, d, b);
    let d3 = cross2(a, b, c);
    let d4 = cross2(a, b, d);
    ((d1 > 1e-12 && d2 < -1e-12) || (d1 < -1e-12 && d2 > 1e-12))
        && ((d3 > 1e-12 && d4 < -1e-12) || (d3 < -1e-12 && d4 > 1e-12))
}

fn point_in_polygon(point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[j];
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let x = (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0];
            if point[0] < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}
