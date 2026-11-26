//! Topology census, position welding and boundary-loop extraction for triangle
//! meshes (ADR-0061 P-MESH-2/3).
//!
//! This module is kernel-free: it operates on [`cad_domain::Mesh`] and never
//! names a modelling crate. Its job is to answer the questions the edit and hole
//! fill layers must not guess:
//!
//! * how many boundary / non-manifold edges a mesh has;
//! * which boundary loops exist;
//! * which triangles belong to one persisted `Mesh.face_sources` key;
//! * whether a face region is a single planar disk.
//!
//! Positions are welded **bitwise** (the common STL/SKP duplicate-vertex case),
//! exactly like the kernel seam's pre-flight census. Near-coincident vertices are
//! the kernel's decision, not this layer's.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::{CadResult, Mesh, Point3, SubElementId};

use crate::{invalid, unsupported};

/// A position-welded mesh: unique vertices, triangles over their canonical
/// indices, and one source id per triangle.
#[derive(Debug, Clone, PartialEq)]
pub struct WeldedMesh {
    pub vertices: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
    /// One entry per triangle. Empty input face_sources becomes all `None`.
    pub face_sources: Vec<Option<SubElementId>>,
}

impl WeldedMesh {
    /// Triangle count.
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }
}

/// The three topology classes the edit/hole gates care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshClass {
    OpenShell,
    ClosedManifold,
    NonManifold,
}

impl MeshClass {
    /// Stable tag (never localised).
    pub fn as_str(self) -> &'static str {
        match self {
            MeshClass::OpenShell => "open-shell",
            MeshClass::ClosedManifold => "closed-manifold",
            MeshClass::NonManifold => "non-manifold",
        }
    }

    /// Whether this is a closed 2-manifold.
    pub fn is_closed(self) -> bool {
        matches!(self, MeshClass::ClosedManifold)
    }
}

/// Class plus the edge counts that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopologyReport {
    pub class: MeshClass,
    pub boundary_edges: usize,
    pub non_manifold_edges: usize,
    pub edges: usize,
}

/// One unique undirected welded edge plus its geometry-derived key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshEdge {
    pub a: u32,
    pub b: u32,
    pub key: String,
}

/// Every unique undirected edge of a welded mesh, sorted by its key.
pub fn edges(mesh: &WeldedMesh) -> Vec<MeshEdge> {
    let mut seen: BTreeMap<(u32, u32), ()> = BTreeMap::new();
    for triangle in &mesh.triangles {
        for (from, to) in directed_edges(*triangle) {
            seen.insert(undirected(from, to), ());
        }
    }
    let mut rows: Vec<MeshEdge> = seen
        .into_keys()
        .map(|(a, b)| MeshEdge {
            a,
            b,
            key: cad_domain::mesh_edge_key(mesh.vertices[a as usize], mesh.vertices[b as usize]),
        })
        .collect();
    rows.sort_by(|left, right| left.key.cmp(&right.key));
    rows
}

/// Resolve a geometry-derived edge key back to its welded endpoints.
pub fn edge_for_key(mesh: &WeldedMesh, key: &str) -> CadResult<(u32, u32)> {
    edges(mesh)
        .into_iter()
        .find(|edge| edge.key == key)
        .map(|edge| (edge.a, edge.b))
        .ok_or_else(|| {
            unsupported(
                "mesh.edge_not_found",
                "no welded triangle edge carries this geometry-derived key (the mesh may have been                  edited since the pick)",
            )
        })
}

/// One face region: the triangles sharing a persisted source key, one planar
/// boundary loop, and the outward normal.
#[derive(Debug, Clone, PartialEq)]
pub struct FaceRegion {
    /// The `SubElementId.source_key` this region names.
    pub key: String,
    /// Triangle indices into the [`WeldedMesh`].
    pub triangles: Vec<usize>,
    /// Boundary loop, in triangle-edge direction, without the repeated start.
    pub boundary: Vec<u32>,
    /// Unit Newell normal of the boundary loop.
    pub normal: Point3,
    /// Newell area (twice the projected polygon area).
    pub area: f64,
}

/// Weld bitwise-identical positions and validate indices.
pub fn weld(mesh: &Mesh) -> CadResult<WeldedMesh> {
    if mesh.vertices.is_empty() || mesh.triangles.is_empty() {
        return Err(invalid(
            "mesh.empty",
            "a mesh edit needs at least one vertex and one triangle",
        ));
    }
    if !mesh.face_sources.is_empty() && mesh.face_sources.len() != mesh.triangles.len() {
        return Err(invalid(
            "mesh.invalid_face_sources",
            format!(
                "face_sources has {} entries for {} triangles",
                mesh.face_sources.len(),
                mesh.triangles.len()
            ),
        ));
    }
    let mut ids: BTreeMap<(u64, u64, u64), u32> = BTreeMap::new();
    let mut vertices = Vec::new();
    let mut remap = Vec::with_capacity(mesh.vertices.len());
    for point in &mesh.vertices {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(invalid("mesh.non_finite", "a vertex is not finite"));
        }
        let key = (
            canonical_bits(point.x),
            canonical_bits(point.y),
            canonical_bits(point.z),
        );
        let next = vertices.len() as u32;
        let id = *ids.entry(key).or_insert(next);
        if id == next {
            vertices.push(*point);
        }
        remap.push(id);
    }
    let count = vertices.len();
    let mut triangles = Vec::with_capacity(mesh.triangles.len());
    for (index, triangle) in mesh.triangles.iter().enumerate() {
        let mut out = [0u32; 3];
        for (slot, corner) in out.iter_mut().zip(triangle) {
            let id = remap.get(*corner as usize).copied().ok_or_else(|| {
                invalid(
                    "mesh.invalid_index",
                    format!(
                        "triangle {index} references vertex {corner}, but there are {}",
                        mesh.vertices.len()
                    ),
                )
            })?;
            *slot = id;
        }
        if out[0] == out[1] || out[1] == out[2] || out[0] == out[2] {
            return Err(invalid(
                "mesh.degenerate_triangle",
                format!("triangle {index} collapses to one welded vertex"),
            ));
        }
        if out.iter().any(|id| *id as usize >= count) {
            return Err(invalid(
                "mesh.invalid_index",
                format!("triangle {index} references an out-of-range welded vertex"),
            ));
        }
        triangles.push(out);
    }
    let face_sources = if mesh.face_sources.is_empty() {
        vec![None; triangles.len()]
    } else {
        mesh.face_sources.clone()
    };
    Ok(WeldedMesh {
        vertices,
        triangles,
        face_sources,
    })
}

fn canonical_bits(value: f64) -> u64 {
    if value == 0.0 {
        0.0f64.to_bits()
    } else {
        value.to_bits()
    }
}

/// Classify a welded mesh.
pub fn classify(mesh: &WeldedMesh) -> TopologyReport {
    let counts = edge_counts(mesh, None);
    let mut boundary_edges = 0;
    let mut non_manifold_edges = 0;
    for count in counts.values() {
        match *count {
            1 => boundary_edges += 1,
            2 => {}
            _ => non_manifold_edges += 1,
        }
    }
    let class = if non_manifold_edges > 0 {
        MeshClass::NonManifold
    } else if boundary_edges > 0 || counts.is_empty() {
        MeshClass::OpenShell
    } else {
        MeshClass::ClosedManifold
    };
    TopologyReport {
        class,
        boundary_edges,
        non_manifold_edges,
        edges: counts.len(),
    }
}

/// Undirected edge -> number of triangle uses.
fn edge_counts(mesh: &WeldedMesh, subset: Option<&[usize]>) -> BTreeMap<(u32, u32), usize> {
    let mut counts = BTreeMap::new();
    let indices: Box<dyn Iterator<Item = usize>> = match subset {
        Some(list) => Box::new(list.iter().copied()),
        None => Box::new(0..mesh.triangles.len()),
    };
    for index in indices {
        let triangle = mesh.triangles[index];
        for (from, to) in directed_edges(triangle) {
            let key = undirected(from, to);
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    counts
}

fn directed_edges(triangle: [u32; 3]) -> [(u32, u32); 3] {
    [
        (triangle[0], triangle[1]),
        (triangle[1], triangle[2]),
        (triangle[2], triangle[0]),
    ]
}

fn undirected(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Boundary loops of the whole mesh, in the triangles' directed-edge order.
///
/// Returns an empty list for a closed mesh. A vertex with more than one outgoing
/// boundary edge (a pinched/non-simple boundary) is refused by name instead of
/// guessing an order.
pub fn boundary_loops(mesh: &WeldedMesh) -> CadResult<Vec<Vec<u32>>> {
    loops_from(mesh, None)
}

/// Boundary loops of one triangle subset (a face region).
pub fn region_boundary_loops(mesh: &WeldedMesh, triangles: &[usize]) -> CadResult<Vec<Vec<u32>>> {
    loops_from(mesh, Some(triangles))
}

fn loops_from(mesh: &WeldedMesh, subset: Option<&[usize]>) -> CadResult<Vec<Vec<u32>>> {
    let counts = edge_counts(mesh, subset);
    let indices: Box<dyn Iterator<Item = usize>> = match subset {
        Some(list) => Box::new(list.iter().copied()),
        None => Box::new(0..mesh.triangles.len()),
    };
    let mut next: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut edges = 0usize;
    for index in indices {
        for (from, to) in directed_edges(mesh.triangles[index]) {
            if counts.get(&undirected(from, to)) == Some(&1) {
                next.entry(from).or_default().push(to);
                edges += 1;
            }
        }
    }
    if edges == 0 {
        return Ok(Vec::new());
    }
    for targets in next.values() {
        if targets.len() > 1 {
            return Err(unsupported(
                "mesh.boundary_not_simple",
                "a boundary vertex has more than one outgoing boundary edge; the loop order is \
                 ambiguous",
            ));
        }
    }
    let mut visited: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut loops = Vec::new();
    for (&start, targets) in &next {
        for &first in targets {
            if visited.contains(&(start, first)) {
                continue;
            }
            let mut loop_vertices = vec![start];
            let mut from = start;
            let mut to = first;
            loop {
                if !visited.insert((from, to)) {
                    return Err(unsupported(
                        "mesh.boundary_not_simple",
                        "a boundary loop revisits an edge",
                    ));
                }
                if to == start {
                    break;
                }
                loop_vertices.push(to);
                let Some(nexts) = next.get(&to) else {
                    return Err(unsupported(
                        "mesh.boundary_not_simple",
                        format!("boundary loop ends at vertex {to} without closing"),
                    ));
                };
                if nexts.len() != 1 {
                    return Err(unsupported(
                        "mesh.boundary_not_simple",
                        "a boundary vertex has more than one outgoing boundary edge",
                    ));
                }
                from = to;
                to = nexts[0];
                if loop_vertices.len() > edges + 1 {
                    return Err(unsupported(
                        "mesh.boundary_not_simple",
                        "boundary loop does not close",
                    ));
                }
            }
            if loop_vertices.len() < 3 {
                return Err(unsupported(
                    "mesh.boundary_degenerate",
                    "a boundary loop has fewer than three vertices",
                ));
            }
            loops.push(loop_vertices);
        }
    }
    if loops.is_empty() {
        return Err(unsupported(
            "mesh.boundary_not_simple",
            "boundary edges exist but no loop was built",
        ));
    }
    Ok(loops)
}

/// Triangle indices whose source key equals `key`.
pub fn face_region_triangles(mesh: &WeldedMesh, key: &str) -> CadResult<Vec<usize>> {
    if mesh.face_sources.iter().all(Option::is_none) {
        return Err(unsupported(
            "mesh.face_identity_unavailable",
            "this mesh carries no source face ids, so a face region cannot be addressed",
        ));
    }
    let triangles: Vec<usize> = mesh
        .face_sources
        .iter()
        .enumerate()
        .filter_map(|(index, source)| {
            source
                .as_ref()
                .filter(|source| source.source_key == key)
                .map(|_| index)
        })
        .collect();
    if triangles.is_empty() {
        return Err(unsupported(
            "mesh.face_not_found",
            format!("no triangle carries the source face key {key:?}"),
        ));
    }
    Ok(triangles)
}

/// Build one planar face region from its source key.
///
/// The region must be a single disk: exactly one boundary loop and every
/// boundary vertex on one plane. A disjoint multi-region key, a hole, or a
/// non-planar region is refused by name instead of picking an arbitrary loop.
pub fn face_region(mesh: &WeldedMesh, key: &str) -> CadResult<FaceRegion> {
    let triangles = face_region_triangles(mesh, key)?;
    let loops = region_boundary_loops(mesh, &triangles)?;
    let boundary = match loops.as_slice() {
        [one] => one.clone(),
        _ => {
            return Err(unsupported(
                "mesh.region_not_disk",
                format!(
                    "source face {key:?} has {} boundary loops; a press/pull region must be a \
                     single disk",
                    loops.len()
                ),
            ))
        }
    };
    let points: Vec<Point3> = boundary
        .iter()
        .map(|index| mesh.vertices[*index as usize])
        .collect();
    let normal = newell_normal(&points);
    let length = (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z).sqrt();
    if length <= 1e-12 {
        return Err(unsupported(
            "mesh.region_degenerate",
            format!("source face {key:?} has a zero-area boundary loop"),
        ));
    }
    let normal = Point3 {
        x: normal.x / length,
        y: normal.y / length,
        z: normal.z / length,
    };
    let area = length / 2.0;
    let deviation = plane_deviation(&points, normal);
    let diagonal = bbox_diagonal(&points);
    if deviation > 1e-6 * diagonal.max(1.0) {
        return Err(unsupported(
            "mesh.region_not_planar",
            format!(
                "source face {key:?} deviates {deviation} from its plane (tolerance {})",
                1e-6 * diagonal.max(1.0)
            ),
        ));
    }
    Ok(FaceRegion {
        key: key.to_string(),
        triangles,
        boundary,
        normal,
        area,
    })
}

/// Boundary loop direction and normal for a **union of source keys** (one disk).
pub fn face_region_union(mesh: &WeldedMesh, keys: &[&str]) -> CadResult<FaceRegion> {
    if keys.is_empty() {
        return Err(invalid("mesh.edit_no_faces", "no face key was selected"));
    }
    let mut triangles: Vec<usize> = Vec::new();
    for key in keys {
        let mut rows = face_region_triangles(mesh, key)?;
        rows.retain(|index| !triangles.contains(index));
        triangles.extend(rows);
    }
    if triangles.is_empty() {
        return Err(unsupported(
            "mesh.face_not_found",
            "the selected face keys address no triangles",
        ));
    }
    let loops = region_boundary_loops(mesh, &triangles)?;
    let boundary = match loops.as_slice() {
        [one] => one.clone(),
        _ => {
            return Err(unsupported(
                "mesh.region_not_disk",
                format!(
                    "the selected face union has {} boundary loops; a press/pull region must be \
                     a single disk",
                    loops.len()
                ),
            ))
        }
    };
    let points: Vec<Point3> = boundary
        .iter()
        .map(|index| mesh.vertices[*index as usize])
        .collect();
    let normal = newell_normal(&points);
    let length = (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z).sqrt();
    if length <= 1e-12 {
        return Err(unsupported(
            "mesh.region_degenerate",
            "the selected face union has a zero-area boundary loop",
        ));
    }
    let normal = Point3 {
        x: normal.x / length,
        y: normal.y / length,
        z: normal.z / length,
    };
    let area = length / 2.0;
    let deviation = plane_deviation(&points, normal);
    let diagonal = bbox_diagonal(&points);
    if deviation > 1e-6 * diagonal.max(1.0) {
        return Err(unsupported(
            "mesh.region_not_planar",
            format!(
                "the selected face union deviates {deviation} from its plane (tolerance {})",
                1e-6 * diagonal.max(1.0)
            ),
        ));
    }
    Ok(FaceRegion {
        key: keys.join(","),
        triangles,
        boundary,
        normal,
        area,
    })
}

/// Newell normal (not normalised).
pub fn newell_normal(points: &[Point3]) -> Point3 {
    let mut normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        normal.x += (a.y - b.y) * (a.z + b.z);
        normal.y += (a.z - b.z) * (a.x + b.x);
        normal.z += (a.x - b.x) * (a.y + b.y);
    }
    normal
}

fn plane_deviation(points: &[Point3], normal: Point3) -> f64 {
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
    let centroid = Point3 {
        x: centroid.x / count,
        y: centroid.y / count,
        z: centroid.z / count,
    };
    points
        .iter()
        .map(|point| {
            ((point.x - centroid.x) * normal.x
                + (point.y - centroid.y) * normal.y
                + (point.z - centroid.z) * normal.z)
                .abs()
        })
        .fold(0.0, f64::max)
}

fn bbox_diagonal(points: &[Point3]) -> f64 {
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
