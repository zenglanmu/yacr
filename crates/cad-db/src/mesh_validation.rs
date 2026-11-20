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
//! ADR-0039's PBR rules use the same shape with the `cad:Mesh.pbr:` prefix.

use cad_domain::{CadError, CadResult, Mesh, MeshPbr, Point3, SemanticGeometry};

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
///   [`cad_domain::is_valid_texture_key`] accepts;
/// * a [`cad_domain::MeshPbr`] has finite in-range scalars, canonical texture
///   keys, and non-empty `uvs` whenever any PBR texture is declared (ADR-0039
///   D3).
///
/// It does **not** check that the textures exist: that needs the database, see
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
    // PBR channels (ADR-0039 D3). `None` is "the source declared no PBR" and
    // costs nothing here; a declared material must be sampleable as written:
    // in-range scalars, canonical keys, and UV0 for every texture slot.
    if let Some(pbr) = &mesh.pbr {
        validate_mesh_pbr(mesh, pbr)?;
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
/// (ADR-0038 D3/D4, extended to the PBR slots by ADR-0039 D3).
///
/// [`crate::validate_geometry`] can only judge a mesh on its own — it has no
/// database — so the "the referenced image really is part of this document" half
/// lives here and runs on the write path (`validate_entity_in`) beside the other
/// referential checks. Walking the same containers `validate_geometry` recurses
/// through means a textured mesh cannot hide inside a `Compound`, a body's display
/// cache, or a dimension/hatch/table display list.
///
/// A missing texture is `InvalidInput` with the stable `cad:Mesh.texture:` /
/// `cad:Mesh.pbr:` prefixes: the mesh is refused rather than stored with a
/// dangling reference, and never silently drawn as if it had no texture.
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
    if let Some(pbr) = &mesh.pbr {
        for (_, slot) in pbr_texture_slots(pbr) {
            let Some(key) = slot else { continue };
            if db.texture(key).is_none() {
                return Err(CadError::InvalidInput(format!(
                    "cad:Mesh.pbr: {key:?} is not present in the drawing database"
                )));
            }
        }
    }
    Ok(())
}

/// The four PBR texture slots with the field name each one is persisted under.
///
/// One list keeps the structural check and the reference check naming the same
/// slots in the same order; the persisted names match the payload keys
/// (`cad:Mesh.pbr`), so a diagnostic names something `jq` can find.
fn pbr_texture_slots(pbr: &MeshPbr) -> [(&'static str, &Option<String>); 4] {
    [
        (
            "metallic_roughness_texture",
            &pbr.metallic_roughness_texture,
        ),
        ("normal_texture", &pbr.normal_texture),
        ("emissive_texture", &pbr.emissive_texture),
        ("occlusion_texture", &pbr.occlusion_texture),
    ]
}

/// The ADR-0039 D3 structural rules for one declared PBR material.
///
/// Nothing is clamped and nothing is defaulted: a scalar outside `[0, 1]` is the
/// source's problem and is reported as such, because silently clamping would make
/// the stored value differ from the imported one.
fn validate_mesh_pbr(mesh: &Mesh, pbr: &MeshPbr) -> CadResult<()> {
    for (field, value) in [
        ("metallic", pbr.metallic),
        ("roughness", pbr.roughness),
        ("occlusion_strength", pbr.occlusion_strength),
    ] {
        if !value.is_finite() {
            return Err(CadError::InvalidInput(format!(
                "cad:Mesh.pbr: {field} is non-finite"
            )));
        }
        if !(0.0..=1.0).contains(&value) {
            return Err(CadError::InvalidInput(format!(
                "cad:Mesh.pbr: {field} {value} is outside [0, 1]"
            )));
        }
    }
    for (index, component) in pbr.emissive.iter().enumerate() {
        if !component.is_finite() {
            return Err(CadError::InvalidInput(format!(
                "cad:Mesh.pbr: emissive[{index}] is non-finite"
            )));
        }
        if !(0.0..=1.0).contains(component) {
            return Err(CadError::InvalidInput(format!(
                "cad:Mesh.pbr: emissive[{index}] {component} is outside [0, 1]"
            )));
        }
    }
    // Every PBR channel is sampled through UV0 (the domain has one UV set), so a
    // declared slot without UVs could never be drawn; the key form is a domain
    // rule and is refused without a database too.
    for (field, slot) in pbr_texture_slots(pbr) {
        let Some(key) = slot else { continue };
        if mesh.uvs.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "cad:Mesh.pbr: {field} {key:?} needs UV0, but the mesh has no UVs"
            )));
        }
        if !cad_domain::is_valid_texture_key(key) {
            return Err(CadError::InvalidInput(format!(
                "cad:Mesh.pbr: {key:?} is not a valid texture key \
                 (expected <32 lowercase hex>.png|.jpg)"
            )));
        }
    }
    Ok(())
}
