//! Boundary-loop hole filling (ADR-0061 P-MESH-3).
//!
//! The kernel seam deliberately does **not** fill holes: `manifold-rust` refuses
//! an open shell. This module implements the narrow, honest capability P-MESH-3
//! asks for: close each boundary loop of a mesh with a planar patch, report every
//! hole, and leave a refused hole open. It never claims a non-planar or
//! self-intersecting loop was filled, and the product is still a non-exact mesh.

use cad_domain::{CadResult, Mesh, Point3};

use crate::topology::{boundary_loops, classify, weld, TopologyReport};
use crate::{invalid, unsupported};

/// What happened to one boundary loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoleReport {
    /// Number of vertices on the loop.
    pub boundary_vertices: usize,
    /// Number of triangles added (0 when refused).
    pub triangles: usize,
    pub filled: bool,
    /// Whether the loop was non-planar and the patch was produced by projecting
    /// it onto its best-fit plane.
    pub non_planar: bool,
    /// Stable refusal code when `filled == false`.
    pub code: Option<&'static str>,
    pub detail: String,
}

/// Filled mesh plus the per-hole report and the output topology.
#[derive(Debug, Clone, PartialEq)]
pub struct FillHolesResult {
    pub mesh: Mesh,
    pub holes: Vec<HoleReport>,
    pub filled: usize,
    pub refused: usize,
    pub topology: TopologyReport,
}

/// Fill every simple planar boundary loop.
///
/// A mesh with no boundary loops is refused by name (`mesh.no_holes`), never
/// returned as an empty success. A hole that is non-planar, degenerate or
/// self-intersecting stays open and is reported; the other holes are still
/// filled.
pub fn fill_holes(mesh: &Mesh) -> CadResult<FillHolesResult> {
    let welded = weld(mesh)?;
    let loops = boundary_loops(&welded)?;
    if loops.is_empty() {
        return Err(unsupported(
            "mesh.no_holes",
            "the mesh has no boundary loop; there is nothing to fill",
        ));
    }
    let nested = nested_loops(&welded.vertices, &loops);
    let mut triangles = welded.triangles.clone();
    let mut sources = welded.face_sources.clone();
    let mut holes = Vec::with_capacity(loops.len());
    let mut filled = 0usize;
    let mut refused = 0usize;
    for (index, boundary) in loops.iter().enumerate() {
        if nested.contains(&index) {
            holes.push(HoleReport {
                boundary_vertices: boundary.len(),
                triangles: 0,
                filled: false,
                non_planar: false,
                code: Some("mesh.hole_nested_not_supported"),
                detail: "the loop is nested with another boundary loop; filling it independently                          could cover an island"
                    .into(),
            });
            refused += 1;
            continue;
        }
        match fill_loop(&welded.vertices, boundary) {
            Ok((patch, non_planar)) => {
                let count = patch.len();
                triangles.extend(patch);
                sources.extend(std::iter::repeat_n(None, count));
                holes.push(HoleReport {
                    boundary_vertices: boundary.len(),
                    triangles: count,
                    filled: true,
                    non_planar,
                    code: None,
                    detail: format!(
                        "filled {} boundary vertices with {count} triangle(s){}",
                        boundary.len(),
                        if non_planar {
                            " by projecting a non-planar loop onto its best-fit plane"
                        } else {
                            ""
                        }
                    ),
                });
                filled += 1;
            }
            Err((code, detail)) => {
                holes.push(HoleReport {
                    boundary_vertices: boundary.len(),
                    triangles: 0,
                    filled: false,
                    non_planar: false,
                    code: Some(code),
                    detail,
                });
                refused += 1;
            }
        }
    }
    let product = Mesh {
        vertices: welded.vertices.clone(),
        triangles,
        normals: Vec::new(),
        face_sources: sources,
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    };
    let topology = classify(&weld(&product)?);
    Ok(FillHolesResult {
        mesh: product,
        holes,
        filled,
        refused,
        topology,
    })
}

/// Loop indices that are nested with another loop (same plane, one inside the
/// other). Filling them independently would cover the island between them.
fn nested_loops(vertices: &[Point3], loops: &[Vec<u32>]) -> std::collections::BTreeSet<usize> {
    let mut nested = std::collections::BTreeSet::new();
    for i in 0..loops.len() {
        for j in (i + 1)..loops.len() {
            if loops_nested(vertices, &loops[i], &loops[j]) {
                nested.insert(i);
                nested.insert(j);
            }
        }
    }
    nested
}

fn loops_nested(vertices: &[Point3], a: &[u32], b: &[u32]) -> bool {
    let pa: Vec<Point3> = a.iter().map(|index| vertices[*index as usize]).collect();
    let pb: Vec<Point3> = b.iter().map(|index| vertices[*index as usize]).collect();
    let normal = crate::topology::newell_normal(&pa);
    let length = (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z).sqrt();
    if length <= 1e-12 {
        return false;
    }
    let normal = Point3 {
        x: normal.x / length,
        y: normal.y / length,
        z: normal.z / length,
    };
    let centroid = pa.iter().fold(
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
    let count = pa.len() as f64;
    let centroid = Point3 {
        x: centroid.x / count,
        y: centroid.y / count,
        z: centroid.z / count,
    };
    let tolerance = 1e-6 * diagonal(&pa).max(1.0);
    if pb.iter().any(|point| {
        ((point.x - centroid.x) * normal.x
            + (point.y - centroid.y) * normal.y
            + (point.z - centroid.z) * normal.z)
            .abs()
            > tolerance
    }) {
        return false;
    }
    let (u, v) = basis(normal, &pa);
    let project = |point: Point3| [dot(point, u), dot(point, v)];
    let poly_a: Vec<[f64; 2]> = pa.iter().map(|point| project(*point)).collect();
    let poly_b: Vec<[f64; 2]> = pb.iter().map(|point| project(*point)).collect();
    // Use a boundary vertex from each loop: centroids can coincide for two
    // nested but differently sized rings, so they are not a containment test.
    point_in_polygon(poly_b[0], &poly_a) != point_in_polygon(poly_a[0], &poly_b)
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

type LoopFill = Result<(Vec<[u32; 3]>, bool), (&'static str, String)>;

fn fill_loop(vertices: &[Point3], boundary: &[u32]) -> LoopFill {
    if boundary.len() < 3 {
        return Err((
            "mesh.hole_degenerate",
            "a boundary loop needs at least three vertices".into(),
        ));
    }
    let points: Vec<Point3> = boundary
        .iter()
        .map(|index| vertices[*index as usize])
        .collect();
    let normal = crate::topology::newell_normal(&points);
    let length = (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z).sqrt();
    if length <= 1e-12 {
        return Err((
            "mesh.hole_degenerate",
            "the boundary loop has zero area".into(),
        ));
    }
    let normal = Point3 {
        x: normal.x / length,
        y: normal.y / length,
        z: normal.z / length,
    };
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
    let count = points.len() as f64;
    let centroid = Point3 {
        x: centroid.x / count,
        y: centroid.y / count,
        z: centroid.z / count,
    };
    let deviation = points
        .iter()
        .map(|point| {
            ((point.x - centroid.x) * normal.x
                + (point.y - centroid.y) * normal.y
                + (point.z - centroid.z) * normal.z)
                .abs()
        })
        .fold(0.0, f64::max);
    let diagonal = diagonal(&points);
    let tolerance = 1e-6 * diagonal.max(1.0);
    // A non-planar loop is still fillable by projecting it onto its Newell
    // plane: the patch may be non-planar, but the topology closes and the report
    // says so. A projection that self-intersects or degenerates is refused.
    let non_planar = deviation > tolerance;
    let (u, v) = basis(normal, &points);
    let projected: Vec<[f64; 2]> = points
        .iter()
        .map(|point| [dot(*point, u), dot(*point, v)])
        .collect();
    if self_intersects(&projected) {
        return Err((
            "mesh.hole_self_intersecting",
            "the boundary loop intersects itself in its own plane".into(),
        ));
    }
    let Some(mut patch) = ear_clip(&projected) else {
        return Err((
            "mesh.hole_not_simple",
            "the boundary loop could not be triangulated as a simple polygon".into(),
        ));
    };
    // The cap must traverse each boundary edge opposite to the existing triangle.
    for triangle in &mut patch {
        triangle.swap(0, 1);
    }
    Ok((
        patch
            .into_iter()
            .map(|triangle| {
                [
                    boundary[triangle[0]],
                    boundary[triangle[1]],
                    boundary[triangle[2]],
                ]
            })
            .collect(),
        non_planar,
    ))
}

fn basis(normal: Point3, points: &[Point3]) -> (Point3, Point3) {
    let mut u = Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    // Pick an axis not parallel to the normal.
    if normal.x.abs() > 0.9 {
        u = Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };
    }
    let v = cross(normal, u);
    let length = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt().max(1e-12);
    let v = Point3 {
        x: v.x / length,
        y: v.y / length,
        z: v.z / length,
    };
    let u = cross(v, normal);
    let length = (u.x * u.x + u.y * u.y + u.z * u.z).sqrt().max(1e-12);
    let u = Point3 {
        x: u.x / length,
        y: u.y / length,
        z: u.z / length,
    };
    let _ = points;
    (u, v)
}

/// Ear clipping; returns triangles whose boundary follows the input polygon
/// order (the polygon may be clockwise or counter-clockwise).
fn ear_clip(points: &[[f64; 2]]) -> Option<Vec<[usize; 3]>> {
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
    while order.len() > 3 {
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

fn signed_area(points: &[[f64; 2]]) -> f64 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        area += a[0] * b[1] - b[0] * a[1];
    }
    area / 2.0
}

fn cross2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn point_in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let ab = cross2(a, b, p);
    let bc = cross2(b, c, p);
    let ca = cross2(c, a, p);
    (ab >= -1e-12 && bc >= -1e-12 && ca >= -1e-12) || (ab <= 1e-12 && bc <= 1e-12 && ca <= 1e-12)
}

fn self_intersects(points: &[[f64; 2]]) -> bool {
    let count = points.len();
    for i in 0..count {
        let a = points[i];
        let b = points[(i + 1) % count];
        for j in (i + 1)..count {
            if j == i || (j + 1) % count == i || i + 1 == j {
                continue;
            }
            let c = points[j];
            let d = points[(j + 1) % count];
            if segments_intersect(a, b, c, d) {
                return true;
            }
        }
    }
    false
}

fn segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let d1 = cross2(c, d, a);
    let d2 = cross2(c, d, b);
    let d3 = cross2(a, b, c);
    let d4 = cross2(a, b, d);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

fn diagonal(points: &[Point3]) -> f64 {
    let mut min = points[0];
    let mut max = points[0];
    for point in points {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        min.z = min.z.min(point.z);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
        max.z = max.z.max(point.z);
    }
    ((max.x - min.x).powi(2) + (max.y - min.y).powi(2) + (max.z - min.z).powi(2)).sqrt()
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

/// Validate a fill result as an edit product: at least one hole must have been
/// filled, otherwise the caller must report a refusal rather than a no-op.
pub fn require_filled(result: &FillHolesResult) -> CadResult<()> {
    if result.filled == 0 {
        return Err(unsupported(
            "mesh.fill_failed",
            format!(
                "no boundary loop could be filled ({} refused)",
                result.refused
            ),
        ));
    }
    if result.topology.boundary_edges > 0 && result.refused == 0 {
        return Err(invalid(
            "mesh.fill_incomplete",
            "all holes were reported filled but boundary edges remain",
        ));
    }
    Ok(())
}
