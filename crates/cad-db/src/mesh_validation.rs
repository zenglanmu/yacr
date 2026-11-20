//! Mesh validation, including the ADR-0038 UV/texture rules.
//!
//! Split out of `validate.rs` when that file reached the code-size gate. The
//! split follows a real boundary: everything here judges **one [`Mesh`]**, either
//! on its own ([`validate_mesh`], which [`crate::validate_body`] and the
//! `SemanticGeometry::Mesh` arm call) or against the document that must supply its
//! texture ([`validate_texture_references`], which needs the database).
//!
//! The two are deliberately separate because they answer different questions and
//! run at different times:
//!
//! * the **structural** rules (finite vertices/normals/UVs, one UV per vertex or
//!   none, a canonical texture key, an in-range triangle list) are a property of
//!   the geometry alone, so [`crate::validate_geometry`] enforces them without a
//!   database — an importer that validates before it has a database still catches
//!   them;
//! * the **referential** rule (the named texture exists in this document) needs
//!   the database, so it runs on the entity write path beside the other reference
//!   checks.
//!
//! Refusals introduced by ADR-0038 use `CadError::InvalidInput` with the stable
//! `cad:Mesh.uvs:` / `cad:Mesh.texture:` prefixes rather than the generic
//! degenerate-geometry form, because importers and reporters match on the prefix.

use cad_domain::{CadError, CadResult, Mesh, Point3, SemanticGeometry};

use crate::validate::degenerate;

/// Validate one mesh's structure.
///
/// Beyond the pre-texture rules (finite vertices/normals, in-range non-degenerate
/// triangles, a colour list parallel to the vertices, a face-source list parallel
/// to the triangles) this enforces the ADR-0038 D1 invariants:
///
/// * `uvs` is empty (no UV0) or exactly one pair per vertex;
/// * every UV component is finite;
/// * `texture.is_some()` requires non-empty `uvs` and a key
///   [`cad_domain::is_valid_texture_key`] accepts.
///
/// It does **not** check that the texture exists: that needs the database, see
/// [`validate_texture_references`].
pub(crate) fn validate_mesh(mesh: &Mesh) -> CadResult<()> {
    let finite = |p: &Point3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
    if !mesh.vertices.iter().all(finite) {
        return Err(degenerate("mesh has a non-finite vertex"));
    }
    if !mesh.normals.iter().all(finite) {
        return Err(degenerate("mesh has a non-finite normal"));
    }
    if !mesh.colors.is_empty() && mesh.colors.len() != mesh.vertices.len() {
        return Err(degenerate(
            "mesh per-vertex colours do not match the vertex count",
        ));
    }
    // UV0 (ADR-0038 D1): either absent (untextured) or exactly one pair per
    // vertex. The refusal is `InvalidInput` with a stable `cad:Mesh.uvs:` prefix
    // rather than the generic degenerate-geometry form, because a renderer and the
    // importer both match on it.
    if !mesh.uvs.is_empty() && mesh.uvs.len() != mesh.vertices.len() {
        return Err(CadError::InvalidInput(
            "cad:Mesh.uvs: expected one UV per vertex or none".to_string(),
        ));
    }
    if let Some(index) = mesh
        .uvs
        .iter()
        .position(|uv| !uv[0].is_finite() || !uv[1].is_finite())
    {
        return Err(CadError::InvalidInput(format!(
            "cad:Mesh.uvs: UV {index} is non-finite"
        )));
    }
    // A texture with no UV could never be sampled, so the combination is refused
    // instead of being drawn untextured behind the importer's back. The key form is
    // checked here too: `cad-db` must not accept a key the archive could not name.
    if let Some(key) = &mesh.texture {
        if mesh.uvs.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "cad:Mesh.texture: {key:?} needs UV0, but the mesh has no UVs"
            )));
        }
        if !cad_domain::is_valid_texture_key(key) {
            return Err(CadError::InvalidInput(format!(
                "cad:Mesh.texture: {key:?} is not a valid texture key \
                 (expected <32 lowercase hex>.png|.jpg)"
            )));
        }
    }
    let count = mesh.vertices.len() as u32;
    for tri in &mesh.triangles {
        if tri.iter().any(|i| *i >= count) {
            return Err(degenerate("mesh triangle references an unknown vertex"));
        }
        if tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] {
            return Err(degenerate("mesh triangle is degenerate"));
        }
    }
    // A face-source list is either absent (not face-addressable) or parallel to
    // the triangle list, one entry per triangle. The ids themselves stay opaque
    // to the database: a source key is a producer-owned stable string (for a
    // B-rep body, the face id the tessellator stamped), not database data.
    if !mesh.face_sources.is_empty() && mesh.face_sources.len() != mesh.triangles.len() {
        return Err(degenerate(
            "mesh face sources do not match the triangle count",
        ));
    }
    Ok(())
}

/// Every mesh texture a geometry tree names must exist in the database
/// (ADR-0038 D3/D4).
///
/// [`crate::validate_geometry`] can only judge a mesh on its own — it has no
/// database — so the "the referenced image really is part of this document" half
/// lives here and runs on the write path (`validate_entity_in`) beside the other
/// referential checks. Walking the same containers `validate_geometry` recurses
/// through means a textured mesh cannot hide inside a `Compound`, a body's display
/// cache, or a dimension/hatch/table display list.
///
/// A missing texture is `InvalidInput` with the stable `cad:Mesh.texture:` prefix:
/// the mesh is refused rather than stored with a dangling reference, and never
/// silently drawn as if it had no texture.
pub fn validate_texture_references(
    db: &crate::DrawingDatabase,
    geometry: &SemanticGeometry,
) -> CadResult<()> {
    match geometry {
        SemanticGeometry::Mesh(mesh) => check_mesh_texture_present(db, mesh),
        SemanticGeometry::Body3d(body) => check_mesh_texture_present(db, &body.display.mesh),
        SemanticGeometry::Compound(children) => children
            .iter()
            .try_for_each(|child| validate_texture_references(db, child)),
        SemanticGeometry::Dimension(dimension) => dimension
            .display()
            .iter()
            .try_for_each(|child| validate_texture_references(db, child)),
        SemanticGeometry::MultiLeader(multileader) => multileader
            .display()
            .iter()
            .try_for_each(|child| validate_texture_references(db, child)),
        SemanticGeometry::Table(table) => table
            .display()
            .iter()
            .try_for_each(|child| validate_texture_references(db, child)),
        SemanticGeometry::Hatch(hatch) => hatch
            .display()
            .iter()
            .try_for_each(|child| validate_texture_references(db, child)),
        // Every other variant is analytic and carries no mesh.
        _ => Ok(()),
    }
}

fn check_mesh_texture_present(db: &crate::DrawingDatabase, mesh: &Mesh) -> CadResult<()> {
    if let Some(key) = &mesh.texture {
        if db.texture(key).is_none() {
            return Err(CadError::InvalidInput(format!(
                "cad:Mesh.texture: {key:?} is not present in the drawing database"
            )));
        }
    }
    Ok(())
}
