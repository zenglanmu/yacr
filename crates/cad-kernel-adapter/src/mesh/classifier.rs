//! Kernel-independent triangle-mesh topology census.
//!
//! The classifier is deliberately local: it welds nothing, moves nothing and
//! assumes the caller validated indices. Its answer is one of the three states a
//! mesh boolean/repair gate cares about, plus the counts that explain it.

use std::collections::BTreeMap;

use super::MeshData;

/// The topology class of a triangle mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshTopology {
    /// At least one edge is used by exactly one triangle: a boundary (hole or
    /// open sheet). Never a closed solid.
    OpenShell,
    /// Every edge is used by exactly two triangles, and there is at least one
    /// triangle.
    ClosedManifold,
    /// At least one edge is used by three or more triangles.
    NonManifold,
}

impl MeshTopology {
    /// Stable tag (never localised).
    pub fn as_str(self) -> &'static str {
        match self {
            MeshTopology::OpenShell => "open-shell",
            MeshTopology::ClosedManifold => "closed-manifold",
            MeshTopology::NonManifold => "non-manifold",
        }
    }

    /// Whether a boolean/rebuild may treat this mesh as a closed solid.
    pub fn is_closed(self) -> bool {
        matches!(self, MeshTopology::ClosedManifold)
    }
}

/// Topology plus the edge counts that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshTopologyReport {
    pub topology: MeshTopology,
    /// Edges used by exactly one triangle.
    pub boundary_edges: usize,
    /// Edges used by three or more triangles.
    pub non_manifold_edges: usize,
    /// Distinct undirected edges.
    pub edges: usize,
    /// Vertices merged by position welding; `0` for the index-based census.
    pub welded_vertices: usize,
}

impl MeshTopologyReport {
    /// The class alone.
    pub fn topology(&self) -> MeshTopology {
        self.topology
    }
}

/// Classify a mesh by counting how many triangles use each undirected edge.
///
/// An empty or invalid mesh reads as [`MeshTopology::OpenShell`] (`0` boundary
/// edges, `0` triangles) rather than pretending to be closed; callers validate
/// first when they need an error.
pub fn inspect_topology(mesh: &MeshData) -> MeshTopologyReport {
    let mut counts: BTreeMap<u64, u32> = BTreeMap::new();
    for triangle in &mesh.triangles {
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            if a == b {
                // A degenerate triangle's repeated index is not an edge; the
                // caller's validator rejects it, and classifying it as one
                // would invent a boundary.
                continue;
            }
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            let key = ((lo as u64) << 32) | hi as u64;
            *counts.entry(key).or_insert(0) += 1;
        }
    }

    let mut boundary_edges = 0;
    let mut non_manifold_edges = 0;
    for count in counts.values() {
        match *count {
            1 => boundary_edges += 1,
            2 => {}
            _ => non_manifold_edges += 1,
        }
    }

    let topology = if non_manifold_edges > 0 {
        MeshTopology::NonManifold
    } else if boundary_edges > 0 || mesh.triangles.is_empty() {
        MeshTopology::OpenShell
    } else {
        MeshTopology::ClosedManifold
    };

    MeshTopologyReport {
        topology,
        boundary_edges,
        non_manifold_edges,
        edges: counts.len(),
        welded_vertices: 0,
    }
}

/// Classification of an **unwelded** mesh (for example an STL, where every
/// triangle owns three fresh vertices, or a SketchUp face group whose vertices
/// are copied per face).
///
/// Real importers do not share vertex indices even when the surface is
/// geometrically closed. The raw index census would call such a cube an open
/// shell; this variant first welds **bitwise-identical positions** (the common
/// importer case) and then counts edges on the canonical indices. It does not
/// merge near-duplicates: that is the kernel's robust import decision, not a
/// topology census. `welded_vertices` reports how many vertex records were
/// merged.
pub fn inspect_topology_welded(mesh: &MeshData) -> MeshTopologyReport {
    let mut ids: BTreeMap<(u64, u64, u64), u32> = BTreeMap::new();
    let mut remap = Vec::with_capacity(mesh.vertices.len());
    for point in &mesh.vertices {
        let key = (
            canonical_bits(point.x),
            canonical_bits(point.y),
            canonical_bits(point.z),
        );
        let next = ids.len() as u32;
        let id = *ids.entry(key).or_insert(next);
        remap.push(id);
    }

    let mut counts: BTreeMap<u64, u32> = BTreeMap::new();
    for triangle in &mesh.triangles {
        let a = remap[triangle[0] as usize];
        let b = remap[triangle[1] as usize];
        let c = remap[triangle[2] as usize];
        if a == b || b == c || a == c {
            continue;
        }
        for (from, to) in [(a, b), (b, c), (c, a)] {
            let (lo, hi) = if from < to { (from, to) } else { (to, from) };
            let key = ((lo as u64) << 32) | hi as u64;
            *counts.entry(key).or_insert(0) += 1;
        }
    }

    let mut boundary_edges = 0;
    let mut non_manifold_edges = 0;
    for count in counts.values() {
        match *count {
            1 => boundary_edges += 1,
            2 => {}
            _ => non_manifold_edges += 1,
        }
    }
    let topology = if non_manifold_edges > 0 {
        MeshTopology::NonManifold
    } else if boundary_edges > 0 || counts.is_empty() {
        MeshTopology::OpenShell
    } else {
        MeshTopology::ClosedManifold
    };

    MeshTopologyReport {
        topology,
        boundary_edges,
        non_manifold_edges,
        edges: counts.len(),
        welded_vertices: mesh.vertices.len().saturating_sub(ids.len()),
    }
}

fn canonical_bits(value: f64) -> u64 {
    // `0.0` and `-0.0` are the same position; every other bit pattern is kept
    // exact so the census does not invent a tolerance.
    if value == 0.0 {
        0.0f64.to_bits()
    } else {
        value.to_bits()
    }
}

/// Classify a mesh without the counts.
pub fn classify_topology(mesh: &MeshData) -> MeshTopology {
    inspect_topology(mesh).topology
}
