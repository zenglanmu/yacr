//! Source geometry collection for the glTF writer.
//!
//! The source is the read-only `cad-db` display model. Only two geometries are
//! writable — a standalone [`SemanticGeometry::Mesh`] and the display cache of a
//! [`SemanticGeometry::Body3d`] — and every other entity (or an entity outside
//! model space) produces one explicit drop entry instead of being skipped.
//!
//! UV0 and the base-colour texture key travel with the mesh; the texture bytes
//! are resolved from the document here because `cad-db` is the only owner of
//! them and the writer never reaches back into it.
//!
//! `cad-db` stores entities in a `BTreeMap<EntityId, _>`, but the collection is
//! sorted by `EntityId` anyway so the output byte order is a stated guarantee
//! rather than a container detail. The entity and triangle budgets are enforced
//! **before** any mesh is copied out of the database.

use std::sync::Arc;

use cad_db::{DbEntity, EntityColor, EntityRenderAttributes, EntityTransparency};
use cad_domain::{CadError, CadResult, ObjectId, Point3, SemanticGeometry, SpaceId};
use cad_import_core::ExportJob;

/// One document-owned base-colour texture resolved from `cad-db`.
///
/// The key is the logical name stored on the mesh (`<32 hex>.png|.jpg`), the
/// bytes are the same `Arc` the database holds, so a texture shared by many
/// meshes is copied into the output buffer once per distinct key.
pub(crate) struct SourceTexture {
    pub key: String,
    pub bytes: Arc<[u8]>,
}

/// One writable source entity, copied out of the read-only database.
pub(crate) struct SourceMesh {
    pub object: ObjectId,
    pub type_key: String,
    /// Deterministic node/mesh name (`<type_key>_<object id>`); never a clock
    /// value or a random id.
    pub name: String,
    pub vertices: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
    /// Empty or one normal per vertex (the `cad-db` invariant).
    pub normals: Vec<Point3>,
    /// Empty or one sRGB colour per vertex (the `cad-db` invariant).
    pub colors: Vec<[u8; 3]>,
    /// Per-vertex UV0, empty or one pair per vertex (ADR-0038 D1).
    ///
    /// Kept in the source `f64` precision; the writer narrows to `f32` once, at
    /// the glTF boundary. The frame rotation deliberately does **not** apply:
    /// UVs are a surface parameterisation, not a position.
    pub uvs: Vec<[f64; 2]>,
    /// The resolved base-colour texture, `None` when this mesh carries none.
    pub texture: Option<SourceTexture>,
    /// A texture key the mesh names but the database does not carry.
    ///
    /// `cad-db` validation refuses that shape, so this is only reachable for a
    /// hand-built database; it is tracked so the writer reports the loss rather
    /// than emitting UVs that point at no image.
    pub missing_texture: Option<String>,
    /// The source declared UVs that do not pair with its vertex list, so none
    /// were copied (defensive; `cad-db` validation refuses this shape too).
    pub uvs_dropped: bool,
    /// The entity's display colour as the importer resolved it.
    ///
    /// Only `Explicit` becomes a glTF material, and only for a mesh that writes
    /// no `COLOR_0`; `ByLayer`/`ByBlock` are symbolic layer references glTF has
    /// no equivalent for.
    pub color: EntityColor,
    /// The entity's display opacity; `Explicit` becomes the material alpha.
    pub transparency: EntityTransparency,
}

/// One source entity that cannot be written, with its stable marker.
pub(crate) struct DropEntry {
    pub object: ObjectId,
    pub type_key: String,
    /// The format-scoped code (`export.gltf.<reason>`), so it is built rather
    /// than borrowed.
    pub code: String,
    pub message: String,
}

/// Everything the writer needs from the source document.
pub(crate) struct Collected {
    pub meshes: Vec<SourceMesh>,
    pub drops: Vec<DropEntry>,
}

impl Collected {
    /// Whether any source mesh carries per-vertex normals.
    pub(crate) fn has_any_normals(&self) -> bool {
        self.meshes.iter().any(|mesh| !mesh.normals.is_empty())
    }

    /// Whether every included vertex has a finite normal.
    ///
    /// glTF attributes are written once for the whole file: a partially normalised
    /// document would need invented normals for the remaining vertices, so the
    /// writer requires this global predicate and otherwise omits `NORMAL`
    /// everywhere with one reported conversion.
    pub(crate) fn normals_complete(&self) -> bool {
        !self.meshes.is_empty()
            && self.meshes.iter().all(|mesh| {
                !mesh.normals.is_empty()
                    && mesh.normals.len() == mesh.vertices.len()
                    && mesh.normals.iter().all(|normal| {
                        normal.x.is_finite() && normal.y.is_finite() && normal.z.is_finite()
                    })
            })
    }

    /// Whether any source mesh carries per-vertex colours.
    pub(crate) fn has_any_colors(&self) -> bool {
        self.meshes.iter().any(|mesh| !mesh.colors.is_empty())
    }

    /// Whether every included vertex has a colour; see [`Self::normals_complete`].
    pub(crate) fn colors_complete(&self) -> bool {
        !self.meshes.is_empty()
            && self
                .meshes
                .iter()
                .all(|mesh| !mesh.colors.is_empty() && mesh.colors.len() == mesh.vertices.len())
    }
}

/// Collect the writable meshes, enforcing the entity and triangle budgets.
///
/// The entity limit is checked against the *source* entity count and the
/// triangle limit incrementally against every triangle that would be written,
/// **before** the entity is copied out of the database. Exceeding either is an
/// explicit error naming the writer; nothing is truncated. A document with no
/// writable mesh is refused instead of producing an empty file.
pub(crate) fn collect(job: &ExportJob<'_>, connector: &str) -> CadResult<Collected> {
    let entity_count = job.database.entity_count();
    if entity_count > job.limits.max_output_entities {
        return Err(CadError::InvalidInput(format!(
            "export.{connector}.limits: {entity_count} source entities exceed the {} entity export limit",
            job.limits.max_output_entities
        )));
    }

    let mut entities: Vec<&DbEntity> = job.database.entities().collect();
    entities.sort_by_key(|entity| entity.id);

    let mut meshes = Vec::new();
    let mut drops = Vec::new();
    let mut triangles = 0usize;
    for entity in entities {
        let object = entity.object.id;
        let type_key = entity.object.type_key.clone();
        if entity.space != SpaceId::Model {
            drops.push(DropEntry {
                object,
                type_key,
                code: format!("export.{connector}.space_not_exported"),
                message: format!(
                    "entity #{} is outside model space ({:?}); the glTF writer exports model space only",
                    object.0, entity.space
                ),
            });
            continue;
        }
        let mesh = match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => mesh,
            SemanticGeometry::Body3d(body) => &body.display.mesh,
            other => {
                let variant = geometry_variant(other);
                drops.push(DropEntry {
                    object,
                    type_key,
                    code: format!("export.{connector}.unsupported_geometry"),
                    message: format!(
                        "{variant} is not mesh geometry and cannot be written to glTF"
                    ),
                });
                continue;
            }
        };
        // An empty triangle list cannot become a glTF primitive, so it is a named
        // drop rather than a mesh with zero primitives (which the format rejects).
        if mesh.triangles.is_empty() {
            drops.push(DropEntry {
                object,
                type_key,
                code: format!("export.{connector}.unsupported_geometry"),
                message: "entity has no display triangles and cannot become a glTF primitive"
                    .to_string(),
            });
            continue;
        }
        // Count before cloning: refusing an over-budget document must not first
        // allocate a copy of every mesh.
        triangles = triangles.saturating_add(mesh.triangles.len());
        if triangles > job.limits.max_output_triangles {
            return Err(CadError::InvalidInput(format!(
                "export.{connector}.limits: {triangles} triangles exceed the {} triangle export limit",
                job.limits.max_output_triangles
            )));
        }
        // Display colour/opacity live beside the entity row; reading them here
        // is what keeps a glTF -> glTF Save As from reporting `Complete` while
        // dropping the material the importer stored.
        let render: EntityRenderAttributes = job.database.entity_render_attributes(entity.id);
        // UV0 is either one pair per vertex or absent; anything else cannot be
        // written and is reported by the writer instead of being truncated.
        let uvs_dropped = !mesh.uvs.is_empty() && mesh.uvs.len() != mesh.vertices.len();
        let uvs = if uvs_dropped {
            Vec::new()
        } else {
            mesh.uvs.clone()
        };
        // The bytes are document-owned, so they are resolved here, while the
        // read-only database is still borrowed; the writer never touches it.
        let (texture, missing_texture) = match &mesh.texture {
            Some(key) => match job.database.texture(key) {
                Some(bytes) => (
                    Some(SourceTexture {
                        key: key.clone(),
                        bytes: Arc::clone(bytes),
                    }),
                    None,
                ),
                None => (None, Some(key.clone())),
            },
            None => (None, None),
        };
        meshes.push(SourceMesh {
            object,
            name: format!("{type_key}_{}", object.0),
            type_key,
            vertices: mesh.vertices.clone(),
            triangles: mesh.triangles.clone(),
            normals: mesh.normals.clone(),
            colors: mesh.colors.clone(),
            uvs,
            texture,
            missing_texture,
            uvs_dropped,
            color: render.color,
            transparency: render.transparency,
        });
    }

    if meshes.is_empty() {
        return Err(CadError::Unsupported(format!(
            "export.{connector}.no_geometry: {} source entity(ies) produced no writable mesh geometry, so no file was written",
            drops.len()
        )));
    }

    Ok(Collected { meshes, drops })
}

/// The variant name reported for an unsupported geometry.
fn geometry_variant(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Line { .. } => "Line",
        SemanticGeometry::Polyline { .. } => "Polyline",
        SemanticGeometry::Circle { .. } => "Circle",
        SemanticGeometry::Arc { .. } => "Arc",
        SemanticGeometry::Ellipse { .. } => "Ellipse",
        SemanticGeometry::Spline { .. } => "Spline",
        SemanticGeometry::Point(_) => "Point",
        SemanticGeometry::Mesh(_) => "Mesh",
        SemanticGeometry::Body3d(_) => "Body3d",
        SemanticGeometry::Insert { .. } => "Insert",
        SemanticGeometry::Text { .. } => "Text",
        SemanticGeometry::Shape { .. } => "Shape",
        SemanticGeometry::Opaque { .. } => "Opaque",
        SemanticGeometry::Compound(_) => "Compound",
        SemanticGeometry::Dimension(_) => "Dimension",
        SemanticGeometry::Hatch(_) => "Hatch",
        SemanticGeometry::MultiLeader(_) => "MultiLeader",
        SemanticGeometry::Table(_) => "Table",
        SemanticGeometry::Image { .. } => "Image",
        SemanticGeometry::Mask { .. } => "Mask",
    }
}
