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
    pub fn push_mesh(
        &mut self,
        object: ObjectId,
        mesh: Mesh,
        color: EntityColor,
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
        self.builder.set_entity_render_attributes(
            entity_id,
            EntityRenderAttributes {
                color,
                ..Default::default()
            },
        )?;
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
