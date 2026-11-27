//! Per-face material painting on one mesh primitive.
//!
//! A document material can be assigned to one `Mesh.face_sources` key (the same
//! key a pick stores in `SelectionRef.sub_element`). The renderer has no
//! per-triangle colour channel, so this module **splits** a mesh into one
//! primitive per material group: a fragment already carries one concrete colour,
//! which is exactly the granularity a face paint needs. Unassigned faces keep the
//! entity's resolved colour, and a transparent material multiplies the entity
//! alpha like a whole-entity paint does.
//!
//! The split is only attempted when the entity has face assignments and the
//! primitive is a triangle mesh whose `face_sources` align with its triangles. A
//! key that no current triangle uses is reported to the caller (a mesh edit can
//! drop a face); it is never mapped onto another triangle.

use std::collections::BTreeMap;
use std::sync::Arc;

use cad_db::DbEntity;
use cad_domain::material::MaterialDocument;
use cad_domain::{EntityId, Mesh};

use crate::primitive::DisplayPrimitive;
use crate::transform::weaker_completeness;
use crate::*;

/// One primitive with the colour/alpha it must draw with.
pub(super) struct Piece {
    pub primitive: DisplayPrimitive,
    /// Concrete sRGB in `[0, 1]`; `false` in [`Self::color_unresolved`] means the
    /// provider already resolved it (a face material always does).
    pub color: [f32; 3],
    pub color_unresolved: bool,
    pub alpha: f32,
}

/// The split result.
pub(super) struct FaceSplit {
    /// At least one piece; one per material group plus the unassigned group.
    pub pieces: Vec<Piece>,
    /// Assigned face keys that no current triangle uses.
    pub stale: usize,
}

/// The pieces to emit for one primitive, with the stale-paint diagnostic.
///
/// The caller only supplies the resolved entity look; this function decides
/// whether the primitive is a mesh with face paints (split), and reports a paint
/// whose face key no longer exists instead of remapping it.
pub(super) fn pieces_for(
    entity: &DbEntity,
    primitive: DisplayPrimitive,
    materials: &MaterialDocument,
    fallback_color: [f32; 3],
    fallback_alpha: f32,
    fallback_unresolved: bool,
    out: &mut DisplayRepresentation,
) -> Vec<Piece> {
    match split_mesh(
        entity.id,
        &primitive,
        materials,
        fallback_color,
        fallback_alpha,
        fallback_unresolved,
    ) {
        Some(split) => {
            if split.stale > 0 {
                out.completeness = weaker_completeness(
                    &out.completeness,
                    &Completeness::Partial(vec![
                        "a face paint no longer matches a mesh face".into()
                    ]),
                );
                out.diagnostics.push(Diagnostic {
                    object: Some(ObjectId(entity.id.0)),
                    code: "representation.material_face_inert".into(),
                    message: format!(
                        "{} face paint(s) on this mesh no longer match a current face; the paint \
                         is stored but not drawn",
                        split.stale
                    ),
                });
            }
            split.pieces
        }
        None => vec![Piece {
            primitive,
            color: fallback_color,
            color_unresolved: fallback_unresolved,
            alpha: fallback_alpha,
        }],
    }
}

/// Split `primitive` by the entity's face materials.
///
/// `None` means no split applies: not a mesh primitive, or the entity has no face
/// assignments. The caller then emits the primitive unchanged.
pub(super) fn split_mesh(
    entity: EntityId,
    primitive: &DisplayPrimitive,
    materials: &MaterialDocument,
    fallback_color: [f32; 3],
    fallback_alpha: f32,
    fallback_unresolved: bool,
) -> Option<FaceSplit> {
    let assigned: Vec<(&str, u64)> = materials
        .face_assignments()
        .iter()
        .filter(|((owner, _), _)| *owner == entity)
        .map(|((_, face), seq)| (face.as_str(), seq.0))
        .collect();
    if assigned.is_empty() {
        return None;
    }
    let (mesh, stored_edges) = match primitive {
        DisplayPrimitive::Mesh(mesh) => (mesh.clone(), None),
        DisplayPrimitive::MeshWithStoredEdges { mesh, edges, .. } => {
            (mesh.clone(), Some(edges.clone()))
        }
        _ => return None,
    };
    if mesh.face_sources.len() != mesh.triangles.len() {
        return None;
    }
    let mut groups: BTreeMap<Option<u64>, Vec<usize>> = BTreeMap::new();
    let mut matched: Vec<&str> = Vec::new();
    for (index, source) in mesh.face_sources.iter().enumerate() {
        let key = source.as_ref().map(|sub| sub.source_key.as_str());
        let seq = key.and_then(|key| {
            assigned
                .iter()
                .find(|(face, _)| *face == key)
                .map(|(face, seq)| {
                    matched.push(*face);
                    *seq
                })
        });
        groups.entry(seq).or_default().push(index);
    }
    let stale = assigned.len().saturating_sub(
        matched
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
    );
    let mut pieces = Vec::with_capacity(groups.len());
    let mut first = true;
    for (seq, triangles) in groups {
        let submesh = submesh(&mesh, &triangles);
        let (color, alpha, color_unresolved) =
            match seq.and_then(|seq| materials.materials().iter().find(|row| row.seq.0 == seq)) {
                Some(material) => {
                    let look = material.look();
                    (
                        [
                            look.color[0] as f32 / 255.0,
                            look.color[1] as f32 / 255.0,
                            look.color[2] as f32 / 255.0,
                        ],
                        (f64::from(fallback_alpha) * look.alpha) as f32,
                        false,
                    )
                }
                None => (fallback_color, fallback_alpha, fallback_unresolved),
            };
        let primitive = match &stored_edges {
            None => DisplayPrimitive::Mesh(Arc::new(submesh)),
            Some(edges) => DisplayPrimitive::MeshWithStoredEdges {
                mesh: Arc::new(submesh),
                // The stored wireframe belongs to the body once; the other
                // material groups must not derive a second boundary overlay.
                edges: if first { edges.clone() } else { Arc::from([]) },
                derive_when_empty: first,
            },
        };
        first = false;
        pieces.push(Piece {
            primitive,
            color,
            color_unresolved,
            alpha,
        });
    }
    Some(FaceSplit { pieces, stale })
}

/// Rebuild a mesh from the selected triangle indices, remapping the attributes
/// that are indexed by vertex. The texture/PBR channels are keys, not per-vertex
/// data, so they travel unchanged.
fn submesh(mesh: &Mesh, triangles: &[usize]) -> Mesh {
    let mut remap = vec![u32::MAX; mesh.vertices.len()];
    let mut out = Mesh {
        texture: mesh.texture.clone(),
        pbr: mesh.pbr.clone(),
        ..Mesh::default()
    };
    for &index in triangles {
        let triangle = mesh.triangles[index];
        let mut remapped = [0_u32; 3];
        for (slot, old) in triangle.iter().enumerate() {
            let old = *old as usize;
            if remap[old] == u32::MAX {
                remap[old] = out.vertices.len() as u32;
                out.vertices.push(mesh.vertices[old]);
                if mesh.normals.len() == mesh.vertices.len() {
                    out.normals.push(mesh.normals[old]);
                }
                if mesh.colors.len() == mesh.vertices.len() {
                    out.colors.push(mesh.colors[old]);
                }
                if mesh.uvs.len() == mesh.vertices.len() {
                    out.uvs.push(mesh.uvs[old]);
                }
            }
            remapped[slot] = remap[old];
        }
        out.triangles.push(remapped);
        out.face_sources
            .push(mesh.face_sources.get(index).cloned().unwrap_or(None));
    }
    out
}
