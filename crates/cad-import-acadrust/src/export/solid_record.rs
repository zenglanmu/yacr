//! The `SOLID` record reverse map: a created 3/4-corner face as a DXF `SOLID`.
//!
//! Kept out of [`super::entity`] only to keep that file inside the repository's
//! size limit; the mapping is part of the same leaf mapper and is reported
//! through the same [`super::report`] entries.

use cad_domain::{Mesh, Point3};

/// Type key of the created DXF `SOLID` entity.
///
/// The domain `SOLID` command (`cad-app`'s draw2 batch) stores a 3- or 4-corner
/// filled face as a two-triangle [`Mesh`] under this key, so the key — not the
/// mesh shape — decides whether the output must be a real `SOLID` record.
/// `SOLID` and `TRACE` share the geometry but not the class: an imported one
/// arrives as `AcDbTrace` (`support::entity_class_name`) and is **not** claimed
/// as a `SOLID` record here, because the stored key no longer says which of the
/// two it was.
pub(super) const SOLID_TYPE_KEY: &str = "AcDbSolid";

/// The four DXF `SOLID` corners (stored order, groups 10/11/12/13) for a
/// domain mesh whose vertices are in boundary order, or the reason the mesh
/// cannot be written as a `SOLID` record.
///
/// DXF `SOLID` stores its corners as first/second/third/fourth, but the drawn
/// boundary is first → second → fourth → third: acadrust keeps that same
/// convention (`Solid::boundary_corners`) and its writer emits groups
/// 10/11/12/13 straight from the four stored corners. The domain mesh stores
/// the boundary cycle in order — the `SOLID` create path and the importer's
/// `quad_mesh` both keep the points as given with triangles `[0,1,2]`/`[0,2,3]`
/// — so the reverse map swaps the last two corners: group 12 is boundary[3]
/// and group 13 is boundary[2]. Getting this backwards would emit a bow-tie.
///
/// A `SOLID` record encodes corners only: it has no triangle list, no per-vertex
/// colours and no plane of its own (it is planar `+Z` OCS here). A mesh that
/// relies on any of those keeps the polyface path and is reported instead.
pub(super) fn solid_record_corners(mesh: &Mesh) -> Result<[Point3; 4], String> {
    let vertices = &mesh.vertices;
    if !(3..=4).contains(&vertices.len()) {
        return Err(format!(
            "a SOLID record has 3 or 4 corners, this mesh has {}",
            vertices.len()
        ));
    }
    if !mesh.colors.is_empty() {
        return Err(format!(
            "a SOLID record has no per-vertex colours, this mesh has {}",
            mesh.colors.len()
        ));
    }
    let z0 = vertices[0].z;
    if vertices.iter().any(|v| (v.z - z0).abs() > 1e-9) {
        return Err(
            "a SOLID record is a planar face in the +Z OCS and this mesh's corners are not \
             coplanar"
                .into(),
        );
    }
    // The record's `[first, second, fourth, third]` cycle *is* the fan
    // `[0,1,2]`/`[0,2,3]`; any other triangulation describes a different
    // filled polygon, so it must not be flattened into four corners.
    let mut actual: Vec<[u32; 3]> = mesh
        .triangles
        .iter()
        .map(|triangle| {
            let mut sorted = *triangle;
            sorted.sort_unstable();
            sorted
        })
        .collect();
    actual.sort_unstable();
    let mut implied: Vec<[u32; 3]> = if vertices.len() == 3 {
        vec![[0, 1, 2]]
    } else {
        vec![[0, 1, 2], [0, 2, 3]]
    };
    implied.sort_unstable();
    if actual != implied {
        return Err(format!(
            "a SOLID record draws the boundary fan {:?}, this mesh has the triangles {:?}",
            implied, actual
        ));
    }
    let corners = if vertices.len() == 3 {
        [vertices[0], vertices[1], vertices[2], vertices[2]]
    } else {
        [vertices[0], vertices[1], vertices[3], vertices[2]]
    };
    Ok(corners)
}
