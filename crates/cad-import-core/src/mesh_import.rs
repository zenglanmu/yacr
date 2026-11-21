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

use std::collections::BTreeMap;
use std::sync::Arc;

use cad_db::{
    validate_geometry, BimDefinitions, BimElement, DbEntity, DbObject, DrawingDatabaseBuilder,
    EntityColor, EntityRenderAttributes, Layer, LayerAttributes,
};
use cad_domain::{
    CadError, CadResult, Completeness, Diagnostic, EntityCapability, EntityId, LayerId, Mesh,
    ObjectId, Revision, SemanticGeometry, SpaceId, SupportStatus,
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
        Ok(MeshImport {
            builder,
            type_key: type_key.into(),
            diagnostics: Vec::new(),
            reasons: Vec::new(),
            inserted: 0,
            next_id: 1,
            partial: false,
            textures: BTreeMap::new(),
            texture_bytes: 0,
            bim_elements: 0,
            bim_definitions: 0,
            limits: request.limits,
        })
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
    /// A mesh is drawable and pickable but is not a MEASURE target; the
    /// capability rows say exactly that, matching the DWG/DXF mesh mapping.
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
                    clip: None,
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

    // ------------------------------------------------------------ BIM ---

    fn bim(connector: &str, id: &str) -> cad_db::BimElement {
        cad_db::BimElement {
            category: Some("IfcWall".into()),
            name: None,
            source: cad_db::BimSource {
                connector: connector.into(),
                document_id: Some("doc".into()),
                id: Some(id.into()),
                native_id: None,
            },
            level: None,
            element_type: None,
            material: None,
            psets: Vec::new(),
            evidence: cad_db::BimEvidence::Declared,
        }
    }

    #[test]
    fn set_entity_bim_reaches_the_finished_database() {
        let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
        let object = import.reserve_object().unwrap();
        let id = import
            .push_mesh(object, triangle(), EntityColor::ByLayer)
            .unwrap();
        import.set_entity_bim(id, bim("ifc", "guid")).unwrap();
        assert_eq!(import.bim_element_count(), 1);
        let outcome = import.finish().unwrap();
        let stored = outcome.database.bim_element(id).expect("BIM block stored");
        assert_eq!(stored.source.id.as_deref(), Some("guid"));
    }

    #[test]
    fn set_entity_bim_for_an_unknown_entity_reports_and_fails() {
        let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
        let error = import
            .set_entity_bim(EntityId(99), bim("ifc", "guid"))
            .expect_err("no entity row owns id 99");
        assert!(error.to_string().contains("ymodel.bim.invalid"), "{error}");
        assert_eq!(import.bim_element_count(), 0, "nothing was stored");
        let outcome = import.finish().unwrap();
        assert!(outcome.database.bim_element(EntityId(99)).is_none());
        assert!(matches!(
            outcome.completeness,
            Completeness::Partial(ref reasons)
                if reasons.iter().any(|reason| reason.contains("BIM block was refused"))
        ));
        assert!(outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DIAG_BIM_ENTITY_REFUSED));
    }

    // ------------------------------------------------------------ textures ---

    fn png(tail: u8) -> Vec<u8> {
        let mut bytes = PNG_MAGIC.to_vec();
        bytes.extend_from_slice(&[tail, 0x1a, 0x2b, 0x3c]);
        bytes
    }

    fn jpeg() -> Vec<u8> {
        let mut bytes = JPEG_MAGIC.to_vec();
        bytes.extend_from_slice(&[0xe0, 0x00, 0x10, 0x4a]);
        bytes
    }

    fn request_with(limits: ImportLimits) -> ImportRequest {
        ImportRequest {
            limits,
            ..request()
        }
    }

    fn is_texture_key(key: &str, extension: &str) -> bool {
        let Some(stem) = key.strip_suffix(extension) else {
            return false;
        };
        stem.len() == 32
            && stem
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    }

    #[test]
    fn a_png_and_a_jpeg_get_deterministic_content_addressed_keys() {
        let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
        let png_key = import.register_texture(&png(0x01)).expect("png");
        let jpeg_key = import.register_texture(&jpeg()).expect("jpeg");
        assert!(is_texture_key(&png_key, ".png"), "{png_key}");
        assert!(is_texture_key(&jpeg_key, ".jpg"), "{jpeg_key}");
        assert_ne!(png_key, jpeg_key, "different bytes, different keys");

        // Same bytes, fresh import: the key is a function of the bytes alone, so a
        // saved document stays addressable across sessions.
        let mut second = MeshImport::new(&request(), "TestMesh").unwrap();
        assert_eq!(second.register_texture(&png(0x01)).expect("png"), png_key);
        assert_ne!(
            second.register_texture(&png(0x02)).expect("other png"),
            png_key
        );
    }

    #[test]
    fn a_non_image_is_refused_by_name() {
        let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
        for bytes in [
            Vec::new(),
            b"not an image".to_vec(),
            // A truncated PNG signature must not pass the sniff.
            PNG_MAGIC[..7].to_vec(),
            // A two-byte JPEG prefix is not a JPEG either.
            JPEG_MAGIC[..2].to_vec(),
        ] {
            let error = import.register_texture(&bytes).expect_err("not a PNG/JPEG");
            assert!(
                error
                    .to_string()
                    .contains("import.texture.unsupported_format"),
                "{error}"
            );
        }
        assert_eq!(import.textures().count(), 0, "a refusal changes no state");
    }

    #[test]
    fn identical_bytes_are_stored_once() {
        let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
        let first = import.register_texture(&png(0x07)).expect("first");
        let second = import.register_texture(&png(0x07)).expect("second");
        assert_eq!(first, second);
        assert_eq!(import.textures().count(), 1, "one copy, one key");
        // The second registration must not consume budget a second time: the whole
        // document budget is already spent by the one copy.
        let mut tight = MeshImport::new(
            &request_with(ImportLimits {
                max_texture_bytes: png(0x07).len(),
                max_total_texture_bytes: png(0x07).len(),
                max_textures: 1,
                ..ImportLimits::default()
            }),
            "TestMesh",
        )
        .unwrap();
        assert!(tight.register_texture(&png(0x07)).is_ok());
        assert!(
            tight.register_texture(&png(0x07)).is_ok(),
            "a duplicate is not charged twice"
        );
    }

    #[test]
    fn each_texture_budget_is_a_named_refusal_that_keeps_state() {
        // Per-image budget.
        let mut import = MeshImport::new(
            &request_with(ImportLimits {
                max_texture_bytes: 4,
                ..ImportLimits::default()
            }),
            "TestMesh",
        )
        .unwrap();
        let error = import.register_texture(&png(0x01)).expect_err("too large");
        assert!(
            error.to_string().contains("import.texture.too_large"),
            "{error}"
        );
        assert_eq!(import.textures().count(), 0);

        // Per-document total budget: each image fits, the sum does not.
        let one = png(0x01).len();
        let mut import = MeshImport::new(
            &request_with(ImportLimits {
                max_total_texture_bytes: one + 1,
                ..ImportLimits::default()
            }),
            "TestMesh",
        )
        .unwrap();
        import.register_texture(&png(0x01)).expect("first fits");
        let error = import.register_texture(&png(0x02)).expect_err("budget");
        assert!(
            error.to_string().contains("import.texture.budget_exceeded"),
            "{error}"
        );
        assert_eq!(import.textures().count(), 1, "the first texture stays");

        // Texture count budget.
        let mut import = MeshImport::new(
            &request_with(ImportLimits {
                max_textures: 1,
                ..ImportLimits::default()
            }),
            "TestMesh",
        )
        .unwrap();
        import.register_texture(&png(0x01)).expect("first");
        let error = import.register_texture(&png(0x02)).expect_err("too many");
        assert!(
            error.to_string().contains("import.texture.too_many"),
            "{error}"
        );
        assert_eq!(import.textures().count(), 1);
    }

    #[test]
    fn finish_carries_the_bytes_into_the_database() {
        let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
        let key = import.register_texture(&png(0x01)).expect("png");
        // Registered but referenced by no mesh: the importer decided the asset
        // belongs to the document, so it is still carried.
        let outcome = import.finish().expect("finish");
        assert_eq!(
            outcome.database.texture(&key).map(|bytes| &bytes[..]),
            Some(&png(0x01)[..])
        );
        assert_eq!(outcome.database.textures().count(), 1);
    }

    #[test]
    fn a_textured_mesh_reaches_the_finished_database() {
        let mut import = MeshImport::new(&request(), "TestMesh").unwrap();
        let key = import.register_texture(&png(0x01)).expect("png");
        let object = import.reserve_object().unwrap();
        let mut mesh = triangle();
        mesh.uvs = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        mesh.texture = Some(key.clone());
        import
            .push_mesh(object, mesh, EntityColor::Explicit([1, 2, 3]))
            .expect("a textured mesh validates");
        let outcome = import.finish().expect("finish");
        let entity = outcome
            .database
            .entities()
            .find(|entity| entity.id == EntityId(1))
            .expect("the mesh row");
        let SemanticGeometry::Mesh(mesh) = &entity.geometry else {
            panic!("expected a mesh");
        };
        assert_eq!(mesh.texture.as_deref(), Some(key.as_str()));
        assert_eq!(mesh.uvs.len(), 3);
        assert!(outcome.database.texture(&key).is_some());
    }
}
