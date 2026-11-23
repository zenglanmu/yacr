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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use cad_db::{
    validate_geometry, BimDefinitions, BimElement, BimRelations, DbEntity, DbObject,
    DrawingDatabaseBuilder, EntityColor, EntityRenderAttributes, Layer, LayerAttributes,
};
use cad_domain::{
    Body3d, CadError, CadResult, Completeness, Diagnostic, EntityCapability, EntityId, LayerId,
    Mesh, ObjectId, Revision, SemanticGeometry, SpaceId, SupportStatus,
};

use crate::{ImportLimits, ImportRequest};

/// Stable diagnostic code: one per-element BIM block was refused and not stored.
///
/// A shape/budget refusal from [`cad_db::DrawingDatabaseBuilder::set_entity_bim`]
/// is never silent: the forwarding [`MeshImport::set_entity_bim`] records it on
/// the import's own report channel (and marks the import `Partial`) before
/// returning the error to its caller.
pub const DIAG_BIM_ENTITY_REFUSED: &str = "import.bim.entity_refused";

/// The PNG container signature (the first eight bytes of every PNG).
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Enough of the JPEG SOI + first marker to identify the container.
const JPEG_MAGIC: [u8; 3] = [0xff, 0xd8, 0xff];

/// The extension a texture key uses for these bytes, or `None` when the bytes are
/// neither PNG nor JPEG.
///
/// This is a **sniff**, not a full decode: the importer only needs to know which
/// archive extension to name, and refusing an unknown container by name is exactly
/// the `import.texture.unsupported_format` contract. A file with a `.png` name and
/// other bytes is refused, because the key must reproduce the bytes' real form.
fn texture_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&PNG_MAGIC) {
        Some("png")
    } else if bytes.starts_with(&JPEG_MAGIC) {
        Some("jpg")
    } else {
        None
    }
}

/// `<sha256 first 32 hex>.<ext>` — the document-owned key for these bytes.
///
/// Content addressing is the whole dedupe mechanism: one image referenced by many
/// faces (or imported twice from the same file) has exactly one key, so the
/// document stores and saves exactly one copy.
fn texture_key(bytes: &[u8], extension: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("{hex}.{extension}")
}

/// Builds a database out of neutral meshes.
pub struct MeshImport {
    builder: DrawingDatabaseBuilder,
    type_key: String,
    /// Every entity type key this import actually inserted, in sorted order.
    ///
    /// Starts empty and gains a key only when [`Self::insert_geometry`] stores a
    /// row, so a single-key importer that inserts one entity reports exactly one
    /// capability row while an import that inserts nothing reports none (no
    /// phantom row for a carrier it never used). A body-carrying importer that
    /// names per-entity keys through [`Self::push_body_with_attributes`] reports
    /// one row per used key with the same statuses.
    type_keys: BTreeSet<String>,
    diagnostics: Vec<Diagnostic>,
    reasons: Vec<String>,
    inserted: usize,
    next_id: u128,
    partial: bool,
    /// Registered textures, keyed by their content-addressed key (ADR-0038 D5).
    ///
    /// Held until [`MeshImport::finish`] flushes them into the builder, so a mesh
    /// may be pushed before or after the image it references and a refused import
    /// never leaves a texture behind in a database.
    textures: BTreeMap<String, Arc<[u8]>>,
    /// Sum of `textures`' payload sizes, for the total-bytes budget.
    texture_bytes: usize,
    /// Element rows that carry a BIM block, for the importer's own reporting.
    bim_elements: usize,
    /// Rows in the shared BIM definition table, for the importer's reporting.
    bim_definitions: usize,
    /// Rows in the BIM relation table, for the importer's reporting.
    bim_relations: usize,
    /// Whether this import declares stored display edges a MEASURE source.
    ///
    /// Off by default: a display cache is a tessellation, so an importer must
    /// opt in explicitly (the SketchUp preview lane does, for its real edge
    /// records). Every existing importer therefore keeps `measure =
    /// Unsupported`.
    edge_snap_measure: bool,
    /// Type keys that actually inserted a `Body3d` with non-empty display edges.
    ///
    /// Only these keys can claim `measure = Partial`: a triangle-only carrier
    /// (for example a `SkpPreviewMesh` group with no stored edge records) is
    /// still not a measurement source.
    edge_body_keys: BTreeSet<String>,
    limits: ImportLimits,
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
        let type_key = type_key.into();
        Ok(MeshImport {
            builder,
            type_keys: BTreeSet::new(),
            type_key,
            diagnostics: Vec::new(),
            reasons: Vec::new(),
            inserted: 0,
            next_id: 1,
            partial: false,
            textures: BTreeMap::new(),
            texture_bytes: 0,
            bim_elements: 0,
            bim_definitions: 0,
            bim_relations: 0,
            edge_snap_measure: false,
            edge_body_keys: BTreeSet::new(),
            limits: request.limits,
        })
    }

    /// Declare whether stored display edges are a MEASURE source for this import.
    ///
    /// Off by default. When enabled, a type key reports `measure = Partial` only
    /// if it actually inserted a [`Body3d`] with non-empty
    /// [`BodyDisplay::edges`](cad_domain::BodyDisplay::edges); every other key
    /// (including a triangle-only carrier of the same import) stays
    /// `Unsupported`. The claim is deliberately narrow: it means endpoint and
    /// midpoint snap on the stored visible edges only, never face, triangle,
    /// center, quadrant or intersection snap and never area/volume.
    pub fn declare_edge_snap_measure(&mut self, enabled: bool) {
        self.edge_snap_measure = enabled;
    }

    /// Register one base-colour texture and return the key a mesh may carry
    /// (ADR-0038 D5).
    ///
    /// The key is content-addressed — `<sha256(bytes) first 32 hex>.<png|jpg>` —
    /// so registering the same bytes twice returns the same key and stores one
    /// copy. The container is **sniffed**, not trusted from a filename: anything
    /// that is not PNG or JPEG is `import.texture.unsupported_format` (a `.png`
    /// name over other bytes is still refused, because the key must reproduce the
    /// bytes' real form).
    ///
    /// Budgets come from [`ImportLimits`] and are refusals by name, never
    /// truncation: `import.texture.too_large` (one image),
    /// `import.texture.budget_exceeded` (the document total) and
    /// `import.texture.too_many` (the count). A refused registration changes no
    /// state, so an importer can decide to drop the texture, report it, and keep
    /// the UVs (the ADR's `Partial` behaviour).
    pub fn register_texture(&mut self, bytes: &[u8]) -> CadResult<String> {
        let extension = texture_extension(bytes).ok_or_else(|| {
            CadError::Unsupported(format!(
                "import.texture.unsupported_format: {} bytes are neither PNG nor JPEG",
                bytes.len()
            ))
        })?;
        let key = texture_key(bytes, extension);
        // Dedupe before any budget check: the image is already stored, so a second
        // registration adds nothing and must not consume budget twice.
        if self.textures.contains_key(&key) {
            return Ok(key);
        }
        if bytes.len() > self.limits.max_texture_bytes {
            return Err(CadError::InvalidInput(format!(
                "import.texture.too_large: {key} is {} bytes, over the {}-byte \
                 per-texture budget",
                bytes.len(),
                self.limits.max_texture_bytes
            )));
        }
        if self.textures.len() >= self.limits.max_textures {
            return Err(CadError::InvalidInput(format!(
                "import.texture.too_many: this import already holds {} textures, at \
                 its {}-texture budget",
                self.textures.len(),
                self.limits.max_textures
            )));
        }
        let total = self.texture_bytes.saturating_add(bytes.len());
        if total > self.limits.max_total_texture_bytes {
            return Err(CadError::InvalidInput(format!(
                "import.texture.budget_exceeded: this import would hold {total} texture \
                 bytes, over the {}-byte total budget",
                self.limits.max_total_texture_bytes
            )));
        }
        // Only now does any state change: a refusal above leaves the import exactly
        // as it was.
        self.texture_bytes = total;
        self.textures.insert(key.clone(), Arc::from(bytes));
        Ok(key)
    }

    /// The textures registered so far, in key order.
    ///
    /// Exposed for the importer's own report (an unreferenced texture is still
    /// carried into the document) and for tests; the bytes themselves are flushed
    /// into the built database at [`Self::finish`].
    pub fn textures(&self) -> impl Iterator<Item = (&str, &Arc<[u8]>)> {
        self.textures
            .iter()
            .map(|(key, bytes)| (key.as_str(), bytes))
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
                clip: None,
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
        let type_key = self.type_key.clone();
        self.insert_geometry(object, SemanticGeometry::Mesh(mesh), &type_key, attributes)
    }

    /// Insert one validated [`Body3d`] as a model-space entity on layer `0`,
    /// under the entity type key the source class maps to.
    ///
    /// A body is drawable when it has a display mesh **or** display edges: a
    /// `BodyKind::Wire` body legitimately has no triangles, and an edge-only
    /// surface body is the honest display of a body whose faces could not be
    /// tessellated. A body with neither is refused by name
    /// (`no drawable geometry`) instead of being stored as an invisible row.
    ///
    /// The `cad-db` body invariants (edge polyline length/finiteness, mesh
    /// validity, solid/wire topology rules) are enforced by [`validate_geometry`],
    /// so a format adapter cannot publish an inconsistent body through here.
    pub fn push_body_with_attributes(
        &mut self,
        object: ObjectId,
        body: Body3d,
        type_key: &str,
        attributes: EntityRenderAttributes,
    ) -> CadResult<EntityId> {
        if body.display.mesh.triangles.is_empty() && body.display.edges.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "import.body.no_geometry: refusing to insert a body '{type_key}' with \
                 no drawable geometry (no display triangles and no display edges)"
            )));
        }
        self.insert_geometry(
            object,
            SemanticGeometry::Body3d(Box::new(body)),
            type_key,
            attributes,
        )
    }

    /// Default-attribute shorthand for [`Self::push_body_with_attributes`].
    pub fn push_body(
        &mut self,
        object: ObjectId,
        body: Body3d,
        type_key: &str,
    ) -> CadResult<EntityId> {
        self.push_body_with_attributes(object, body, type_key, EntityRenderAttributes::default())
    }

    /// Shared insert path: validate, store, set attributes, account for the row.
    fn insert_geometry(
        &mut self,
        object: ObjectId,
        geometry: SemanticGeometry,
        type_key: &str,
        attributes: EntityRenderAttributes,
    ) -> CadResult<EntityId> {
        // The shared writer enforces the `cad-db` geometry invariants (finite
        // vertices/normals, in-range non-degenerate triangles, colour length, body
        // edge polylines, body topology rules). A format adapter cannot publish
        // inconsistent geometry through here.
        validate_geometry(&geometry)?;
        let body_has_edges = match &geometry {
            SemanticGeometry::Body3d(body) => !body.display.edges.is_empty(),
            _ => false,
        };
        let entity_id = EntityId(object.0);
        self.builder.insert_entity(DbEntity {
            object: DbObject {
                id: object,
                type_key: type_key.to_string(),
                revision: Revision(0),
                // Mesh/body formats have no CAD handle; do not invent one.
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
        self.type_keys.insert(type_key.to_string());
        if body_has_edges {
            self.edge_body_keys.insert(type_key.to_string());
        }
        Ok(entity_id)
    }

    /// Record one element's BIM block (ADR-0042 D2/D7).
    ///
    /// Forwards to [`DrawingDatabaseBuilder::set_entity_bim`] and keeps this
    /// import's own accounting: a stored block bumps [`Self::bim_element_count`];
    /// a refusal is written to the import's diagnostic channel **and** marks the
    /// import `Partial` before the error is returned, so a caller that chooses to
    /// continue still leaves an honest report behind. The entity must already be
    /// inserted (`push_mesh*` / a future non-mesh insert), exactly as the builder
    /// requires.
    pub fn set_entity_bim(&mut self, id: EntityId, bim: BimElement) -> CadResult<()> {
        match self.builder.set_entity_bim(id, bim) {
            Ok(()) => {
                self.bim_elements += 1;
                Ok(())
            }
            Err(error) => {
                let reason = "an element's BIM block was refused and was not stored".to_string();
                self.record(
                    Diagnostic {
                        object: Some(ObjectId(id.0)),
                        code: DIAG_BIM_ENTITY_REFUSED.into(),
                        message: error.to_string(),
                    },
                    Some(reason),
                );
                Err(error)
            }
        }
    }

    /// Replace the shared BIM definition table (ADR-0042 D3).
    ///
    /// Forwards to [`DrawingDatabaseBuilder::set_bim_definitions`]. It may be
    /// called before or after [`Self::set_entity_bim`]: cross-references are
    /// checked once, at [`Self::finish`], when the whole document exists.
    pub fn set_bim_definitions(&mut self, definitions: BimDefinitions) -> CadResult<()> {
        let count = definitions.len();
        self.builder.set_bim_definitions(definitions)?;
        self.bim_definitions = count;
        Ok(())
    }

    /// Element rows that carry a BIM block so far.
    pub fn bim_element_count(&self) -> usize {
        self.bim_elements
    }

    /// Rows in the shared BIM definition table so far.
    pub fn bim_definition_count(&self) -> usize {
        self.bim_definitions
    }

    /// Replace the BIM relation table (ADR-0042 D3 tail).
    ///
    /// Forwards to [`DrawingDatabaseBuilder::set_bim_relations`], which
    /// shape-checks every row here and leaves the row-count and encoded-size
    /// budgets to [`Self::finish`] (both are refusals, never truncation). It may
    /// be called before or after the element/definition setters: relation
    /// endpoints do not have to resolve to stored elements.
    pub fn set_bim_relations(&mut self, relations: BimRelations) -> CadResult<()> {
        let count = relations.len();
        self.builder.set_bim_relations(relations)?;
        self.bim_relations = count;
        Ok(())
    }

    /// Rows in the BIM relation table so far.
    pub fn bim_relation_count(&self) -> usize {
        self.bim_relations
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
    /// Registered textures are flushed into the database here (ADR-0038 D5) —
    /// including ones no mesh references. That is deliberate: the importer decided
    /// those bytes belong to the document (a material library, a face set that was
    /// dropped as `Partial`), and dropping an asset the source shipped would lose
    /// data the format could have preserved. The document stays the authority for
    /// what a save writes; a key nothing references simply is not reachable from
    /// geometry.
    ///
    /// A mesh is drawable and pickable but is not a MEASURE target; a body's
    /// stored display edges become `Partial` only when the importer declared
    /// them ([`Self::declare_edge_snap_measure`]) **and** that type key actually
    /// stored edges. Everything else reports `Unsupported`, matching the
    /// DWG/DXF mesh mapping.
    pub fn finish(mut self) -> CadResult<MeshImportOutcome> {
        // Borrow the two fields separately: iterating the texture map while the
        // builder is mutated is fine because they are disjoint.
        for (key, bytes) in &self.textures {
            self.builder
                .insert_texture(key.clone(), Arc::clone(bytes))?;
        }
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
            // One row per entity type key actually inserted, all with the same
            // statuses: a body/mesh is drawable and pickable but is not a MEASURE
            // target, matching the DWG/DXF mesh mapping. A single-key importer
            // that inserted geometry still reports exactly one row; an import
            // that inserted nothing reports none.
            capabilities: self
                .type_keys
                .into_iter()
                .map(|type_key| {
                    // Stored-edge snapping is the one narrow measurement claim:
                    // endpoint/midpoint only, and only for a key that actually
                    // stored display edges. Everything else stays Unsupported
                    // (meshes are drawable and pickable, not measurable).
                    let measure =
                        if self.edge_snap_measure && self.edge_body_keys.contains(&type_key) {
                            SupportStatus::Partial
                        } else {
                            SupportStatus::Unsupported
                        };
                    EntityCapability {
                        type_key,
                        read: SupportStatus::Verified,
                        semantic,
                        render: SupportStatus::Verified,
                        pick: SupportStatus::Verified,
                        measure,
                    }
                })
                .collect(),
            completeness,
            diagnostics: self.diagnostics,
        })
    }
}

#[cfg(test)]
#[path = "mesh_import/tests.rs"]
mod tests;
