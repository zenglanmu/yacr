//! Which rows a kernel-free mesh face edit can address (ADR-0061 P-MESH-2).

use cad_domain::{BodyKind, SemanticGeometry};

/// Whether a row can be edited as a mesh face (P-MESH-2 PRESSPULL).
pub(crate) fn mesh_face_edit_target(entity: &cad_db::DbEntity) -> bool {
    match &entity.geometry {
        SemanticGeometry::Mesh(mesh) => !mesh.triangles.is_empty(),
        SemanticGeometry::Body3d(body) => {
            body.kind == BodyKind::Unknown
                && body.exact_brep().is_none()
                && !body.display.mesh.triangles.is_empty()
        }
        _ => false,
    }
}
