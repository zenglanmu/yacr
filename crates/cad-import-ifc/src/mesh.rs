//! One product's `ifc-lite` parts → one validated domain mesh.
//!
//! Kept beside `lib.rs` (which owns the walk) so the per-part validation and
//! budget checks are readable on their own and the adapter's entry module stays
//! within the code-size gate.

use cad_domain::{CadError, CadResult, Diagnostic, Mesh, Point3};
use cad_import_core::{ImportLimits, MeshImport};
use ifc_lite_geometry::Mesh as IfcMesh;

use crate::{DIAG_DEGENERATE_TRIANGLE, DIAG_NORMALS_DROPPED};

/// Convert one product's `ifc-lite` f32 parts into one validated domain mesh.
///
/// `parts` are already world-placed by the router; `Mesh::origin` (set when the
/// router relativised coordinates for f32 precision) is folded back in f64 so
/// the database never holds origin-relative coordinates.
pub(crate) fn element_mesh(
    parts: &[IfcMesh],
    label: &str,
    limits: &ImportLimits,
    import: &mut MeshImport,
) -> CadResult<Option<Mesh>> {
    let mut vertices = Vec::<Point3>::new();
    let mut normals = Vec::<Point3>::new();
    let mut normals_usable = true;
    let mut triangles = Vec::<[u32; 3]>::new();
    let mut degenerate = 0usize;

    for part in parts {
        if part.positions.len() % 3 != 0 {
            return Err(CadError::CorruptData(format!(
                "IFC product {label}: the geometry router returned {} position component(s), \
                 not a multiple of 3",
                part.positions.len()
            )));
        }
        if part.indices.len() % 3 != 0 {
            return Err(CadError::CorruptData(format!(
                "IFC product {label}: the geometry router returned {} triangle indices, not a \
                 multiple of 3",
                part.indices.len()
            )));
        }
        let part_vertices = part.positions.len() / 3;
        let base = vertices.len();
        if base + part_vertices > u32::MAX as usize {
            return Err(CadError::InvalidInput(format!(
                "IFC product {label} needs more than {} vertices, over the u32 mesh index range",
                u32::MAX
            )));
        }

        let origin = part.origin;
        for chunk in part.positions.as_chunks::<3>().0 {
            let point = Point3 {
                x: origin[0] + chunk[0] as f64,
                y: origin[1] + chunk[1] as f64,
                z: origin[2] + chunk[2] as f64,
            };
            if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
                return Err(CadError::CorruptData(format!(
                    "IFC product {label}: the geometry router produced a non-finite vertex"
                )));
            }
            vertices.push(point);
        }

        if normals_usable {
            if part.normals.len() == part.positions.len() {
                for chunk in part.normals.as_chunks::<3>().0 {
                    let normal = Point3 {
                        x: chunk[0] as f64,
                        y: chunk[1] as f64,
                        z: chunk[2] as f64,
                    };
                    if !normal.x.is_finite() || !normal.y.is_finite() || !normal.z.is_finite() {
                        normals_usable = false;
                        break;
                    }
                    normals.push(normal);
                }
            } else {
                normals_usable = false;
            }
            if !normals_usable {
                normals.clear();
            }
        }

        for corner in part.indices.as_chunks::<3>().0 {
            if corner.iter().any(|index| *index as usize >= part_vertices) {
                return Err(CadError::CorruptData(format!(
                    "IFC product {label}: a triangle references a vertex outside its {part_vertices}-vertex mesh"
                )));
            }
            let placed = [
                index_to_u32(base, corner[0], label)?,
                index_to_u32(base, corner[1], label)?,
                index_to_u32(base, corner[2], label)?,
            ];
            if placed[0] == placed[1] || placed[1] == placed[2] || placed[0] == placed[2] {
                degenerate += 1;
                continue;
            }
            triangles.push(placed);
            if let Some(max) = limits.max_mesh_faces {
                if triangles.len() > max {
                    return Err(CadError::InvalidInput(format!(
                        "IFC product {label} has more than {max} triangle(s), over the face \
                         import limit"
                    )));
                }
            }
        }
    }

    if degenerate > 0 {
        let reason = format!(
            "IFC product {label}: {degenerate} triangle(s) with repeated vertex indices were \
             dropped"
        );
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_DEGENERATE_TRIANGLE.into(),
                message: reason.clone(),
            },
            Some(reason),
        );
    }
    if !normals_usable && !vertices.is_empty() {
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_NORMALS_DROPPED.into(),
                message: format!(
                    "IFC product {label}: the router returned a missing, short or non-finite \
                     normal list; the mesh was stored without normals"
                ),
            },
            None,
        );
    }
    if triangles.is_empty() {
        return Ok(None);
    }
    if let Some(max) = limits.max_mesh_face_indices {
        let indices = triangles.len().saturating_mul(3);
        if indices > max {
            return Err(CadError::InvalidInput(format!(
                "IFC product {label} has {indices} face corner indices, over the {max} \
                 face-index import limit"
            )));
        }
    }
    Ok(Some(Mesh {
        vertices,
        triangles,
        normals: if normals_usable { normals } else { Vec::new() },
        face_sources: Vec::new(),
        colors: Vec::new(),
        // UVs and the texture key are attached by the caller: the vertex list
        // here is already final, so a caller-side UV set stays 1:1 with it.
        uvs: Vec::new(),
        texture: None,
    }))
}

fn index_to_u32(base: usize, index: u32, label: &str) -> CadResult<u32> {
    u32::try_from(base + index as usize).map_err(|_| {
        CadError::InvalidInput(format!(
            "IFC product {label} exceeds the u32 mesh index range"
        ))
    })
}
