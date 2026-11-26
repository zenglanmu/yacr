//! Boundary-loop hole filling (ADR-0061 P-MESH-3).
//!
//! The kernel seam deliberately does **not** fill holes: `manifold-rust` refuses
//! an open shell. This module implements the narrow, honest capability P-MESH-3
//! asks for: close each boundary loop with a cap (projected to the loop's
//! best-fit plane when non-planar), fill the annulus between coplanar nested
//! loops without covering the hole, report every
//! hole, and leave a refused hole open. It never claims a self-intersecting loop
//! or an inconsistent nested group was filled, and the product is still a
//! non-exact mesh.

use cad_domain::{CadResult, Mesh, Point3};

use crate::topology::{boundary_loops, classify, weld, TopologyReport};

mod nested;

use crate::{invalid, unsupported};
use nested::{fill_group, Nesting};

/// What happened to one boundary loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoleReport {
    /// Number of vertices on the loop.
    pub boundary_vertices: usize,
    /// Number of triangles added (0 when refused or closed by an enclosing annulus).
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

/// Fill every boundary loop of an open shell.
///
/// Independent loops are capped directly. Coplanar loops nested inside another
/// boundary loop are treated as one region: the outer loop is filled with the
/// nested loops as holes (an annulus), so the patch never covers the hole or an
/// island. Loops alternate between solid and hole by nesting depth: a hole at
/// odd depth is closed by its parent patch, a nested island at even depth gets
/// its own patch with its direct children as holes.
///
/// A mesh with no boundary loops is refused by name (`mesh.no_holes`), never
/// returned as an empty success. A hole that is degenerate, self-intersecting,
/// inconsistently wound or has no visible bridge stays open and is reported;
/// the other holes are still filled.
pub fn fill_holes(mesh: &Mesh) -> CadResult<FillHolesResult> {
    let welded = weld(mesh)?;
    let loops = boundary_loops(&welded)?;
    if loops.is_empty() {
        return Err(unsupported(
            "mesh.no_holes",
            "the mesh has no boundary loop; there is nothing to fill",
        ));
    }
    let nesting = Nesting::build(&welded.vertices, &loops);
    let mut triangles = welded.triangles.clone();
    let mut sources = welded.face_sources.clone();
    let mut reports: Vec<Option<HoleReport>> = (0..loops.len()).map(|_| None).collect();
    let mut filled = 0usize;
    let mut refused = 0usize;
    for index in 0..loops.len() {
        if reports[index].is_some() {
            continue;
        }
        // Odd depth loops are holes of an enclosing patch and are reported with
        // that patch, not filled on their own.
        if nesting.depth(index) % 2 == 1 {
            continue;
        }
        let children = nesting.children(index);
        if children.is_empty() {
            match fill_loop(&welded.vertices, &loops[index]) {
                Ok((patch, non_planar)) => {
                    let count = patch.len();
                    triangles.extend(patch);
                    sources.extend(std::iter::repeat_n(None, count));
                    reports[index] = Some(HoleReport {
                        boundary_vertices: loops[index].len(),
                        triangles: count,
                        filled: true,
                        non_planar,
                        code: None,
                        detail: format!(
                            "filled {} boundary vertices with {count} triangle(s){}",
                            loops[index].len(),
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
                    reports[index] = Some(HoleReport {
                        boundary_vertices: loops[index].len(),
                        triangles: 0,
                        filled: false,
                        non_planar: false,
                        code: Some(code),
                        detail,
                    });
                    refused += 1;
                }
            }
            continue;
        }
        let holes: Vec<&Vec<u32>> = children.iter().map(|child| &loops[*child]).collect();
        match fill_group(&welded.vertices, &loops[index], &holes) {
            Ok(group) => {
                let count = group.triangles.len();
                triangles.extend(group.triangles);
                sources.extend(std::iter::repeat_n(None, count));
                reports[index] = Some(HoleReport {
                    boundary_vertices: loops[index].len(),
                    triangles: count,
                    filled: true,
                    non_planar: group.non_planar,
                    code: None,
                    detail: format!(
                        "filled the outer loop and {} nested hole loop(s) with {count} triangle(s){}",
                        children.len(),
                        if group.non_planar {
                            " by projecting a non-planar group onto the outer loop's best-fit plane"
                        } else {
                            ""
                        }
                    ),
                });
                filled += 1;
                for child in children {
                    reports[child] = Some(HoleReport {
                        boundary_vertices: loops[child].len(),
                        triangles: 0,
                        filled: true,
                        non_planar: group.non_planar,
                        code: None,
                        detail: format!(
                            "closed as a nested hole of boundary loop {}; the annulus patch does not \
                             cover the hole",
                            index + 1
                        ),
                    });
                    filled += 1;
                }
            }
            Err((code, detail)) => {
                for affected in nesting.subtree(index) {
                    if reports[affected].is_some() {
                        continue;
                    }
                    reports[affected] = Some(HoleReport {
                        boundary_vertices: loops[affected].len(),
                        triangles: 0,
                        filled: false,
                        non_planar: false,
                        code: Some(code),
                        detail: format!(
                            "boundary loop {} is part of a nested group that cannot be filled: {detail}",
                            affected + 1
                        ),
                    });
                    refused += 1;
                }
            }
        }
    }
    for index in 0..loops.len() {
        if reports[index].is_none() {
            reports[index] = Some(HoleReport {
                boundary_vertices: loops[index].len(),
                triangles: 0,
                filled: false,
                non_planar: false,
                code: Some("mesh.hole_nested_incomplete"),
                detail: "the nested boundary group could not be resolved".into(),
            });
            refused += 1;
        }
    }
    let holes: Vec<HoleReport> = reports
        .into_iter()
        .map(|report| report.expect("every boundary loop receives a report"))
        .collect();
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

pub(super) fn basis(normal: Point3, points: &[Point3]) -> (Point3, Point3) {
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
pub(super) fn ear_clip(points: &[[f64; 2]]) -> Option<Vec<[usize; 3]>> {
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

pub(super) fn signed_area(points: &[[f64; 2]]) -> f64 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        area += a[0] * b[1] - b[0] * a[1];
    }
    area / 2.0
}

pub(super) fn cross2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

pub(super) fn point_in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let ab = cross2(a, b, p);
    let bc = cross2(b, c, p);
    let ca = cross2(c, a, p);
    (ab >= -1e-12 && bc >= -1e-12 && ca >= -1e-12) || (ab <= 1e-12 && bc <= 1e-12 && ca <= 1e-12)
}

pub(super) fn self_intersects(points: &[[f64; 2]]) -> bool {
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

pub(super) fn diagonal(points: &[Point3]) -> f64 {
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

pub(super) fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub(super) fn cross(a: Point3, b: Point3) -> Point3 {
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
