//! Mesh edit products: face-region move/press-pull, subdivision and smoothing.
//!
//! Every operation is content-in/content-out on [`cad_domain::Mesh`]. The result
//! keeps the persisted `face_sources` where the triangle still represents the
//! same source face; derived display attributes (normals, vertex colours, UVs,
//! texture, PBR) are dropped because this layer does not recompute them. That is
//! deliberate honesty: a stale normal or UV would be worse than "not carried".

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::{CadResult, Mesh, Point3, SubElementId};

use crate::topology::{
    classify, edge_for_key, face_region_union, weld, FaceRegion, MeshClass, WeldedMesh,
};
use crate::{invalid, unsupported};

/// Move one planar face region by `delta`, adding side walls.
///
/// The input must be a closed 2-manifold (the same "solid" gate as SketchUp's
/// press/pull). The region is selected by its persisted source keys; the moved
/// cap keeps those keys, while the generated side walls carry no source id.
pub fn move_faces(mesh: &Mesh, keys: &[&str], delta: Point3) -> CadResult<Mesh> {
    if !delta.x.is_finite() || !delta.y.is_finite() || !delta.z.is_finite() {
        return Err(invalid(
            "mesh.edit_non_finite_delta",
            "the move delta is not finite",
        ));
    }
    if delta.x.abs() + delta.y.abs() + delta.z.abs() <= 1e-12 {
        return Err(invalid("mesh.edit_zero_delta", "the move delta is zero"));
    }
    press_pull(mesh, keys, delta)
}

/// Press/pull one planar face region by `distance` along its outward normal.
pub fn extrude_faces(mesh: &Mesh, keys: &[&str], distance: f64) -> CadResult<Mesh> {
    if !distance.is_finite() {
        return Err(invalid(
            "mesh.edit_non_finite_distance",
            "the press/pull distance is not finite",
        ));
    }
    if distance.abs() <= 1e-12 {
        return Err(invalid(
            "mesh.edit_zero_distance",
            "the press/pull distance is zero",
        ));
    }
    let welded = weld(mesh)?;
    let region = face_region_union(&welded, keys)?;
    let delta = Point3 {
        x: region.normal.x * distance,
        y: region.normal.y * distance,
        z: region.normal.z * distance,
    };
    press_pull(&welded_to_mesh(&welded), keys, delta)
}

fn press_pull(mesh: &Mesh, keys: &[&str], delta: Point3) -> CadResult<Mesh> {
    let welded = weld(mesh)?;
    let input = classify(&welded);
    if !input.class.is_closed() {
        return Err(unsupported(
            "mesh.edit_requires_closed",
            format!(
                "a face move needs a closed 2-manifold input; this mesh is {} ({} boundary, {} \
                 non-manifold edge(s))",
                input.class.as_str(),
                input.boundary_edges,
                input.non_manifold_edges
            ),
        ));
    }
    let region = face_region_union(&welded, keys)?;
    let mut vertices = welded.vertices.clone();
    let mut region_map: BTreeMap<u32, u32> = BTreeMap::new();
    let mut region_vertices = BTreeSet::new();
    for index in &region.triangles {
        for corner in welded.triangles[*index] {
            region_vertices.insert(corner);
        }
    }
    for old in &region_vertices {
        let point = welded.vertices[*old as usize];
        let next = vertices.len() as u32;
        vertices.push(Point3 {
            x: point.x + delta.x,
            y: point.y + delta.y,
            z: point.z + delta.z,
        });
        region_map.insert(*old, next);
    }

    let mut triangles = Vec::with_capacity(welded.triangles.len() + region.triangles.len() * 2);
    let mut sources = Vec::with_capacity(triangles.capacity());
    for (index, triangle) in welded.triangles.iter().enumerate() {
        if region.triangles.contains(&index) {
            continue;
        }
        triangles.push(*triangle);
        sources.push(welded.face_sources[index].clone());
    }
    // The moved cap keeps the region's winding and source ids.
    for index in &region.triangles {
        let triangle = welded.triangles[*index];
        triangles.push([
            region_map[&triangle[0]],
            region_map[&triangle[1]],
            region_map[&triangle[2]],
        ]);
        sources.push(welded.face_sources[*index].clone());
    }
    // One quad per boundary edge: [a,b,b'] + [a,b',a']. This pairs the old edge
    // with the adjacent (b->a) use, the new edge with the cap (a'->b'), and the
    // vertical edge with the neighbouring side quad.
    for index in 0..region.boundary.len() {
        let a = region.boundary[index];
        let b = region.boundary[(index + 1) % region.boundary.len()];
        let a2 = region_map[&a];
        let b2 = region_map[&b];
        triangles.push([a, b, b2]);
        triangles.push([a, b2, a2]);
        sources.push(None);
        sources.push(None);
    }
    let (vertices, triangles, sources) = compact(&vertices, &triangles, &sources)?;
    let output = build_mesh(vertices, triangles, sources);
    let welded_out = weld(&output)?;
    let report = classify(&welded_out);
    if !report.class.is_closed() {
        return Err(unsupported(
            "mesh.edit_not_closed",
            format!(
                "the edit produced a {} mesh ({} boundary, {} non-manifold edge(s)); a face move \
                 must leave a closed product",
                report.class.as_str(),
                report.boundary_edges,
                report.non_manifold_edges
            ),
        ));
    }
    Ok(output)
}

/// Move one or more mesh edges by translating both endpoints.
///
/// Unlike a face move this adds no side walls: the incident triangles simply
/// deform, preserving the mesh's topology. The edge is addressed by the same
/// geometry-derived key the pick produces ([`cad_domain::mesh_edge_key`]); a key
/// that no longer matches a welded edge is refused as `mesh.edge_not_found`.
pub fn move_edges(mesh: &Mesh, keys: &[&str], delta: Point3) -> CadResult<Mesh> {
    if !delta.x.is_finite() || !delta.y.is_finite() || !delta.z.is_finite() {
        return Err(invalid(
            "mesh.edit_non_finite_delta",
            "the edge move delta is not finite",
        ));
    }
    if delta.x.abs() + delta.y.abs() + delta.z.abs() <= 1e-12 {
        return Err(invalid(
            "mesh.edit_zero_delta",
            "the edge move delta is zero",
        ));
    }
    if keys.is_empty() {
        return Err(invalid("mesh.edit_no_edges", "no mesh edge was selected"));
    }
    let welded = weld(mesh)?;
    let mut moved = BTreeSet::new();
    for key in keys {
        let (a, b) = edge_for_key(&welded, key)?;
        moved.insert(a);
        moved.insert(b);
    }
    let mut vertices = welded.vertices.clone();
    for id in moved {
        let point = vertices[id as usize];
        vertices[id as usize] = Point3 {
            x: point.x + delta.x,
            y: point.y + delta.y,
            z: point.z + delta.z,
        };
    }
    let (vertices, triangles, sources) =
        compact(&vertices, &welded.triangles, &welded.face_sources)?;
    let output = build_mesh(vertices, triangles, sources);
    validate_same_class(&welded, &output, "move edge")?;
    Ok(output)
}

/// Subdivide every selected triangle into four (one midpoint per edge).
///
/// `keys = None` subdivides the whole mesh. Child triangles inherit the parent
/// source id, so face selection stays stable across the edit.
pub fn subdivide(mesh: &Mesh, keys: Option<&[&str]>) -> CadResult<Mesh> {
    let welded = weld(mesh)?;
    let selected: BTreeSet<usize> = match keys {
        None => (0..welded.triangles.len()).collect(),
        Some(keys) => {
            if keys.is_empty() {
                return Err(invalid("mesh.edit_no_faces", "no face key was selected"));
            }
            let mut rows = BTreeSet::new();
            for key in keys {
                rows.extend(crate::topology::face_region_triangles(&welded, key)?);
            }
            rows
        }
    };
    let mut vertices = welded.vertices.clone();
    let mut midpoints: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut triangles = Vec::with_capacity(welded.triangles.len() + selected.len() * 3);
    let mut sources = Vec::with_capacity(triangles.capacity());
    for (index, triangle) in welded.triangles.iter().enumerate() {
        if !selected.contains(&index) {
            triangles.push(*triangle);
            sources.push(welded.face_sources[index].clone());
            continue;
        }
        let [a, b, c] = *triangle;
        let ab = midpoint(&mut vertices, &mut midpoints, a, b, &welded);
        let bc = midpoint(&mut vertices, &mut midpoints, b, c, &welded);
        let ca = midpoint(&mut vertices, &mut midpoints, c, a, &welded);
        for row in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
            triangles.push(row);
            sources.push(welded.face_sources[index].clone());
        }
    }
    let (vertices, triangles, sources) = compact(&vertices, &triangles, &sources)?;
    let output = build_mesh(vertices, triangles, sources);
    validate_same_class(&welded, &output, "subdivide")?;
    Ok(output)
}

fn midpoint(
    vertices: &mut Vec<Point3>,
    midpoints: &mut BTreeMap<(u32, u32), u32>,
    a: u32,
    b: u32,
    welded: &WeldedMesh,
) -> u32 {
    let key = if a < b { (a, b) } else { (b, a) };
    if let Some(index) = midpoints.get(&key) {
        return *index;
    }
    let pa = welded.vertices[a as usize];
    let pb = welded.vertices[b as usize];
    let index = vertices.len() as u32;
    vertices.push(Point3 {
        x: (pa.x + pb.x) / 2.0,
        y: (pa.y + pb.y) / 2.0,
        z: (pa.z + pb.z) / 2.0,
    });
    midpoints.insert(key, index);
    index
}

/// Laplacian smoothing: move interior vertices toward their one-ring average.
///
/// Boundary vertices (any vertex on a boundary edge) are pinned so an open mesh
/// does not shrink its rim. `iterations` is clamped to `1..=64`; `factor` must be
/// finite and in `(0, 1]`.
pub fn smooth(mesh: &Mesh, iterations: u32, factor: f64) -> CadResult<Mesh> {
    if !factor.is_finite() || factor <= 0.0 || factor > 1.0 {
        return Err(invalid(
            "mesh.edit_invalid_smoothing",
            "the smoothing factor must be finite and in (0, 1]",
        ));
    }
    if iterations == 0 {
        return Err(invalid(
            "mesh.edit_invalid_smoothing",
            "smoothing needs at least one iteration",
        ));
    }
    let iterations = iterations.min(64);
    let welded = weld(mesh)?;
    let mut adjacency: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut boundary_vertices: BTreeSet<u32> = BTreeSet::new();
    let counts = {
        let mut counts: BTreeMap<(u32, u32), usize> = BTreeMap::new();
        for triangle in &welded.triangles {
            for (from, to) in [
                (triangle[0], triangle[1]),
                (triangle[1], triangle[2]),
                (triangle[2], triangle[0]),
            ] {
                adjacency.entry(from).or_default().insert(to);
                adjacency.entry(to).or_default().insert(from);
                let key = if from < to { (from, to) } else { (to, from) };
                *counts.entry(key).or_insert(0) += 1;
            }
        }
        counts
    };
    for ((a, b), count) in counts {
        if count == 1 {
            boundary_vertices.insert(a);
            boundary_vertices.insert(b);
        }
    }
    let mut vertices = welded.vertices.clone();
    let movable: Vec<u32> = vertices
        .iter()
        .enumerate()
        .filter_map(|(index, _)| {
            let id = index as u32;
            (!boundary_vertices.contains(&id) && adjacency.get(&id).is_some_and(|n| n.len() >= 2))
                .then_some(id)
        })
        .collect();
    if movable.is_empty() {
        return Err(unsupported(
            "mesh.edit_no_smoothing",
            "every vertex is on a boundary or has no one-ring; nothing can be smoothed",
        ));
    }
    for _ in 0..iterations {
        let next: Vec<Point3> = movable
            .iter()
            .map(|id| {
                let neighbors = &adjacency[id];
                let mut sum = Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                };
                for neighbor in neighbors {
                    let point = vertices[*neighbor as usize];
                    sum.x += point.x;
                    sum.y += point.y;
                    sum.z += point.z;
                }
                let count = neighbors.len() as f64;
                let current = vertices[*id as usize];
                Point3 {
                    x: current.x + factor * (sum.x / count - current.x),
                    y: current.y + factor * (sum.y / count - current.y),
                    z: current.z + factor * (sum.z / count - current.z),
                }
            })
            .collect();
        for (id, point) in movable.iter().zip(next) {
            vertices[*id as usize] = point;
        }
    }
    let (vertices, triangles, sources) =
        compact(&vertices, &welded.triangles, &welded.face_sources)?;
    let output = build_mesh(vertices, triangles, sources);
    validate_same_class(&welded, &output, "smooth")?;
    Ok(output)
}

fn validate_same_class(input: &WeldedMesh, output: &Mesh, operation: &str) -> CadResult<()> {
    let input_class = classify(input).class;
    let welded_out = weld(output)?;
    let report = classify(&welded_out);
    if input_class.is_closed() && !report.class.is_closed() {
        return Err(unsupported(
            "mesh.edit_not_closed",
            format!(
                "{operation} produced a {} mesh from a closed input ({} boundary, {} non-manifold \
                 edge(s))",
                report.class.as_str(),
                report.boundary_edges,
                report.non_manifold_edges
            ),
        ));
    }
    Ok(())
}

fn welded_to_mesh(welded: &WeldedMesh) -> Mesh {
    build_mesh(
        welded.vertices.clone(),
        welded.triangles.clone(),
        welded.face_sources.clone(),
    )
}

fn build_mesh(
    vertices: Vec<Point3>,
    triangles: Vec<[u32; 3]>,
    face_sources: Vec<Option<SubElementId>>,
) -> Mesh {
    Mesh {
        vertices,
        triangles,
        normals: Vec::new(),
        face_sources,
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

type CompactedMesh = (Vec<Point3>, Vec<[u32; 3]>, Vec<Option<SubElementId>>);

fn compact(
    vertices: &[Point3],
    triangles: &[[u32; 3]],
    sources: &[Option<SubElementId>],
) -> CadResult<CompactedMesh> {
    let mut used = vec![false; vertices.len()];
    for triangle in triangles {
        for corner in triangle {
            let Some(slot) = used.get_mut(*corner as usize) else {
                return Err(invalid(
                    "mesh.edit_invalid_index",
                    "an edit produced an out-of-range triangle index",
                ));
            };
            *slot = true;
        }
    }
    let mut remap = vec![0u32; vertices.len()];
    let mut out_vertices = Vec::new();
    for (index, point) in vertices.iter().enumerate() {
        if used[index] {
            remap[index] = out_vertices.len() as u32;
            out_vertices.push(*point);
        }
    }
    let out_triangles = triangles
        .iter()
        .map(|triangle| {
            [
                remap[triangle[0] as usize],
                remap[triangle[1] as usize],
                remap[triangle[2] as usize],
            ]
        })
        .collect();
    Ok((out_vertices, out_triangles, sources.to_vec()))
}

/// Reclassify a product for the command layer's report.
pub fn report(mesh: &Mesh) -> CadResult<crate::topology::TopologyReport> {
    Ok(classify(&weld(mesh)?))
}

/// Whether a welded mesh is closed (helper for command diagnostics).
pub fn is_closed(mesh: &Mesh) -> CadResult<bool> {
    Ok(classify(&weld(mesh)?).class == MeshClass::ClosedManifold)
}

/// The region a press/pull would move, for a command's plan/diagnostics.
pub fn region_for(mesh: &Mesh, keys: &[&str]) -> CadResult<FaceRegion> {
    let welded = weld(mesh)?;
    face_region_union(&welded, keys)
}
