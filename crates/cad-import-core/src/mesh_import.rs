//! Shared database scaffold for format readers whose whole payload is meshes.
//!
//! OBJ and STL (and a future STEP tessellation path) all reduce to the same
//! neutral shape: one or more [`Mesh`] objects written to layer `0` in model
//! space, with no blocks, layouts, xrefs or vendor records. Keeping that
//! bookkeeping here means the format crates only own parsing and the honest
//! loss reporting, and every mesh-producing importer reports the same
//! capabilities for the same geometry.
//!
//! This helper intentionally knows nothing about a file format. It never
//! invents units, colours or completeness: the caller supplies every value.

use cad_db::{
    validate_geometry, DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor,
    EntityRenderAttributes, Layer, LayerAttributes,
};
use cad_domain::{
    CadError, CadResult, Completeness, Diagnostic, EntityCapability, EntityId, LayerId, Mesh,
    ObjectId, Revision, SemanticGeometry, SpaceId, SupportStatus,
};

use crate::ImportRequest;

/// Builds a database out of neutral meshes.
pub struct MeshImport {
    builder: DrawingDatabaseBuilder,
    type_key: String,
    diagnostics: Vec<Diagnostic>,
    reasons: Vec<String>,
    inserted: usize,
    next_id: u128,
    partial: bool,
}

/// The report-side result of a finished mesh import.
pub struct MeshImportOutcome {
    pub database: cad_db::DrawingDatabase,
    pub capabilities: Vec<EntityCapability>,
    pub completeness: Completeness,
    pub diagnostics: Vec<Diagnostic>,
}

impl MeshImport {
    /// Start a mesh import on layer `0` with default layer attributes.
    pub fn new(request: &ImportRequest, type_key: impl Into<String>) -> CadResult<Self> {
        let mut builder = DrawingDatabaseBuilder::new(request.database);
        let layer = LayerId(0);
        builder.insert_layer(Layer {
            id: layer,
            name: "0".into(),
            visible: true,
        })?;
        builder.set_layer_attributes(layer, LayerAttributes::default())?;
        Ok(MeshImport {
            builder,
            type_key: type_key.into(),
            diagnostics: Vec::new(),
            reasons: Vec::new(),
            inserted: 0,
            next_id: 1,
            partial: false,
        })
    }

    /// Reserve the identity a mesh entity (or an explicit empty-object report)
    /// will use. `ObjectId` and `EntityId` share the numeric value, exactly as
    /// the other importers allocate them.
    pub fn reserve_object(&mut self) -> CadResult<ObjectId> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| CadError::Invariant("mesh import id overflow".into()))?;
        Ok(ObjectId(id))
    }

    /// Insert one validated mesh as a model-space entity on layer `0`.
    ///
    /// Colour-only shorthand for [`Self::push_mesh_with_attributes`]; every
    /// other render attribute keeps its documented default (`ByLayer`
    /// transparency, `ByLayer` lineweight/linetype).
    pub fn push_mesh(
        &mut self,
        object: ObjectId,
        mesh: Mesh,
        color: EntityColor,
    ) -> CadResult<EntityId> {
        self.push_mesh_with_attributes(
            object,
            mesh,
            EntityRenderAttributes {
                color,
                ..Default::default()
            },
        )
    }

    /// Insert one validated mesh with the caller's full render attributes.
    ///
    /// Used when the source carries more than a colour (the RVT preview lane
    /// stores a glTF material's alpha as an explicit opacity). The caller owns
    /// every attribute value: this helper still invents nothing, and layer `0`
    /// keeps its default (fully opaque, ACI 7) attributes.
    pub fn push_mesh_with_attributes(
        &mut self,
        object: ObjectId,
        mesh: Mesh,
        attributes: EntityRenderAttributes,
    ) -> CadResult<EntityId> {
        if mesh.triangles.is_empty() {
            return Err(CadError::Invariant(
                "refusing to insert a triangle-less mesh entity".into(),
            ));
        }
        let geometry = SemanticGeometry::Mesh(mesh);
        // The shared writer enforces the `cad-db` mesh invariants (finite
        // vertices/normals, in-range non-degenerate triangles, colour length).
        // A format adapter cannot publish an inconsistent mesh through here.
        validate_geometry(&geometry)?;
        let entity_id = EntityId(object.0);
        self.builder.insert_entity(DbEntity {
            object: DbObject {
                id: object,
                type_key: self.type_key.clone(),
                revision: Revision(0),
                // Mesh formats have no CAD handle; do not invent one.
                source_handle: None,
            },
            id: entity_id,
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry,
            draw_order: self.inserted as i64,
        })?;
        self.builder
            .set_entity_render_attributes(entity_id, attributes)?;
        self.inserted += 1;
        Ok(entity_id)
    }

    /// Number of entities inserted so far.
    pub fn entity_count(&self) -> usize {
        self.inserted
    }

    /// Record a diagnostic, optionally marking the import `Partial` with the
    /// same human-facing reason (callers pass `None` for a lossless note).
    pub fn record(&mut self, diagnostic: Diagnostic, partial_reason: Option<String>) {
        self.diagnostics.push(diagnostic);
        if let Some(reason) = partial_reason {
            self.partial = true;
            if !self.reasons.contains(&reason) {
                self.reasons.push(reason);
            }
        }
    }

    /// Mark the import `Partial` with a reason and no per-object diagnostic.
    pub fn note_partial(&mut self, reason: impl Into<String>) {
        self.partial = true;
        let reason = reason.into();
        if !self.reasons.contains(&reason) {
            self.reasons.push(reason);
        }
    }

    /// Publish the database and the derived report fields.
    ///
    /// A mesh is drawable and pickable but is not a MEASURE target; the
    /// capability rows say exactly that, matching the DWG/DXF mesh mapping.
    pub fn finish(self) -> CadResult<MeshImportOutcome> {
        let database = self.builder.finish()?;
        let completeness = if self.partial {
            Completeness::Partial(self.reasons)
        } else {
            Completeness::Complete
        };
        let semantic = match &completeness {
            Completeness::Complete => SupportStatus::Verified,
            Completeness::Partial(_) => SupportStatus::Partial,
            Completeness::Missing(_) => SupportStatus::Unsupported,
            Completeness::Unverified => SupportStatus::Unverified,
        };
        Ok(MeshImportOutcome {
            database,
            capabilities: vec![EntityCapability {
                type_key: self.type_key,
                read: SupportStatus::Verified,
                semantic,
                render: SupportStatus::Verified,
                pick: SupportStatus::Verified,
                measure: SupportStatus::Unsupported,
            }],
            completeness,
            diagnostics: self.diagnostics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::EntityTransparency;
    use cad_domain::{DatabaseId, DocumentId, Point3};

    fn request() -> ImportRequest {
        ImportRequest {
            document: DocumentId(1),
            database: DatabaseId(1),
            bytes: std::sync::Arc::from(Vec::new().into_boxed_slice()),
            limits: crate::ImportLimits::default(),
            generation: 0,
            xref: None,
        }
    }

    fn triangle() -> Mesh {
        Mesh {
            vertices: vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            ],
            triangles: vec![[0, 1, 2]],
            ..Mesh::default()
        }
    }

    #[test]
    fn push_mesh_keeps_the_documented_bylayer_defaults() {
        let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
        let object = import.reserve_object().unwrap();
        let id = import
            .push_mesh(object, triangle(), EntityColor::Explicit([1, 2, 3]))
            .unwrap();
        let outcome = import.finish().unwrap();
        let attributes = outcome.database.entity_render_attributes(id);
        assert_eq!(attributes.color, EntityColor::Explicit([1, 2, 3]));
        assert_eq!(attributes.transparency, EntityTransparency::ByLayer);
    }

    #[test]
    fn push_mesh_with_attributes_stores_an_explicit_opacity() {
        let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
        let object = import.reserve_object().unwrap();
        let id = import
            .push_mesh_with_attributes(
                object,
                triangle(),
                EntityRenderAttributes {
                    color: EntityColor::Explicit([10, 20, 30]),
                    transparency: EntityTransparency::Explicit(0.5),
                    ..Default::default()
                },
            )
            .unwrap();
        let outcome = import.finish().unwrap();
        let attributes = outcome.database.entity_render_attributes(id);
        assert_eq!(attributes.color, EntityColor::Explicit([10, 20, 30]));
        assert_eq!(attributes.transparency, EntityTransparency::Explicit(0.5));
        // Nothing else is invented: the layer keeps its opaque, ACI 7 defaults.
        assert_eq!(outcome.database.entity_count(), 1);
    }
}
